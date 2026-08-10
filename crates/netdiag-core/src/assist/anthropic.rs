//! Anthropic Messages API, over raw HTTP.
//!
//! Raw HTTP rather than the official SDK because there is no Rust SDK, and
//! because the engine crate builds with no system dependencies — a rule worth
//! more here than the convenience would be.
//!
//! # What this deliberately does not send
//!
//! `temperature`, `top_p`, `top_k` and `thinking.budget_tokens` are **rejected
//! with a 400** by the current models. They are not merely discouraged, so
//! there is nothing to make configurable: any knob exposed for them would be a
//! knob that breaks every request.
//!
//! Thinking is left at its default, which is on. Disabling it is possible but
//! carries a documented failure mode — the model can write a tool call into its
//! visible text instead of emitting a structured call, and the call then
//! silently never runs. For a diagnosis driven entirely by tool calls, that is
//! the worst possible way to fail.

use serde::Deserialize;
use std::time::Duration;

use super::http::post_json;
use super::provider::{
    brief, ChatProvider, Message, ProviderError, Request, StopReason, ToolCall, Turn, Usage,
};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

/// The model used unless the user names another.
pub const DEFAULT_MODEL: &str = "claude-opus-5";

/// Generous: a diagnosis reads structured evidence and the whole exchange is
/// one request. Well inside the model's ceiling.
const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;

pub struct Anthropic {
    api_key: String,
    model: String,
    timeout: Duration,
}

impl Anthropic {
    pub fn new(api_key: impl Into<String>, model: Option<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            // Thinking models take their time on a hard question, and this is
            // one request per tool round rather than a stream.
            timeout: Duration::from_secs(300),
        }
    }
}

impl ChatProvider for Anthropic {
    fn describe(&self) -> String {
        format!("Anthropic {}", self.model)
    }

    async fn complete(&self, request: Request<'_>) -> Result<Turn, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "system": request.system,
            "messages": encode_messages(request.messages),
            "tools": request.tools.iter().map(|tool| serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })).collect::<Vec<_>>(),
        });

        let payload = serde_json::to_string(&body)
            .map_err(|e| ProviderError::Protocol(format!("could not encode the request: {e}")))?;

        let response = post_json(
            ENDPOINT,
            &payload,
            &[
                ("x-api-key", self.api_key.as_str()),
                ("anthropic-version", API_VERSION),
            ],
            self.timeout,
            RESPONSE_LIMIT,
        )
        .await
        .map_err(ProviderError::Transport)?;

        match response.status {
            200 => decode_turn(&response.body),
            401 | 403 => Err(ProviderError::Auth(brief(&response.body, 200))),
            429 | 529 => Err(ProviderError::Busy(brief(&response.body, 200))),
            status => Err(ProviderError::Protocol(format!(
                "HTTP {status}: {}",
                brief(&response.body, 300)
            ))),
        }
    }
}

/// Rebuilds the wire-format message list.
///
/// Consecutive tool results are merged into one user message: the API pairs
/// each result to its call by id, and splitting them across messages teaches
/// the model to stop requesting tools in parallel.
fn encode_messages(messages: &[Message]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut pending_results: Vec<serde_json::Value> = Vec::new();

    let flush = |pending: &mut Vec<serde_json::Value>, out: &mut Vec<serde_json::Value>| {
        if !pending.is_empty() {
            out.push(serde_json::json!({
                "role": "user",
                "content": std::mem::take(pending),
            }));
        }
    };

    for message in messages {
        match message {
            Message::ToolResult {
                call_id,
                content,
                is_error,
                ..
            } => pending_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": call_id,
                "content": content,
                "is_error": is_error,
            })),
            Message::User { text } => {
                flush(&mut pending_results, &mut out);
                out.push(serde_json::json!({"role": "user", "content": text}));
            }
            Message::Assistant { text, tool_calls } => {
                flush(&mut pending_results, &mut out);
                let mut content: Vec<serde_json::Value> = Vec::new();
                if !text.trim().is_empty() {
                    content.push(serde_json::json!({"type": "text", "text": text}));
                }
                for call in tool_calls {
                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.input,
                    }));
                }
                // An assistant turn with neither text nor calls is not
                // representable on the wire and would be rejected.
                if !content.is_empty() {
                    out.push(serde_json::json!({"role": "assistant", "content": content}));
                }
            }
        }
    }

    flush(&mut pending_results, &mut out);
    out
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    content: Vec<ApiBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    /// Thinking blocks arrive with empty text unless summaries are requested;
    /// either way they are not the answer. Named so they are ignored rather
    /// than failing the parse.
    Thinking {},
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

fn decode_turn(body: &str) -> Result<Turn, ProviderError> {
    let response: ApiResponse = serde_json::from_str(body)
        .map_err(|e| ProviderError::Protocol(format!("could not read the response: {e}")))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    for block in response.content {
        match block {
            ApiBlock::Text { text: chunk } => {
                if !text.is_empty() && !chunk.is_empty() {
                    text.push('\n');
                }
                text.push_str(&chunk);
            }
            ApiBlock::ToolUse { id, name, input } => tool_calls.push(ToolCall { id, name, input }),
            ApiBlock::Thinking {} | ApiBlock::Other => {}
        }
    }

    // Checked before the content is trusted: a declined request returns a
    // successful 200 whose content is empty or partial.
    let stop_reason = match response.stop_reason.as_deref() {
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("refusal") => StopReason::Refusal,
        _ => StopReason::EndTurn,
    };

    let usage = response.usage.unwrap_or_default();

    Ok(Turn {
        text,
        tool_calls,
        stop_reason,
        usage: Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::provider::ToolSpec;

    #[test]
    fn a_tool_using_response_is_decoded_with_its_call() {
        let body = r#"{
            "content": [
                {"type":"thinking","thinking":""},
                {"type":"text","text":"Checking that port."},
                {"type":"tool_use","id":"toolu_01","name":"device_detail",
                 "input":{"ip":"10.0.3.22"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 1200, "output_tokens": 85}
        }"#;

        let turn = decode_turn(body).unwrap();
        assert_eq!(turn.stop_reason, StopReason::ToolUse);
        assert_eq!(turn.text, "Checking that port.");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "device_detail");
        assert_eq!(turn.tool_calls[0].input["ip"], "10.0.3.22");
        assert_eq!(turn.usage.output_tokens, 85);
    }

    #[test]
    fn a_refusal_is_surfaced_rather_than_read_as_an_empty_answer() {
        // A declined request is a successful 200 with empty content. Treating
        // that as "the model had nothing to say" would report a diagnosis of
        // silence.
        let body = r#"{"content": [], "stop_reason": "refusal"}"#;
        let turn = decode_turn(body).unwrap();
        assert_eq!(turn.stop_reason, StopReason::Refusal);
        assert!(turn.text.is_empty());
    }

    #[test]
    fn unknown_block_types_do_not_fail_the_parse() {
        // The API adds block types over time; one we do not model must cost
        // that block, not the whole response.
        let body = r#"{"content":[
            {"type":"server_tool_use","id":"x","name":"web_search","input":{}},
            {"type":"text","text":"done"}
        ],"stop_reason":"end_turn"}"#;
        assert_eq!(decode_turn(body).unwrap().text, "done");
    }

    #[test]
    fn parallel_tool_results_are_sent_as_one_user_message() {
        // Splitting them teaches the model to stop asking for tools in
        // parallel, which doubles the round trips on every later turn.
        let messages = vec![
            Message::User {
                text: "why is it slow".into(),
            },
            Message::Assistant {
                text: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "a".into(),
                        name: "ping".into(),
                        input: serde_json::json!({}),
                    },
                    ToolCall {
                        id: "b".into(),
                        name: "wifi".into(),
                        input: serde_json::json!({}),
                    },
                ],
            },
            Message::ToolResult {
                call_id: "a".into(),
                name: "ping".into(),
                content: "1ms".into(),
                is_error: false,
            },
            Message::ToolResult {
                call_id: "b".into(),
                name: "wifi".into(),
                content: "ok".into(),
                is_error: false,
            },
        ];

        let encoded = encode_messages(&messages);
        assert_eq!(encoded.len(), 3, "user, assistant, one merged result turn");
        let results = encoded[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["tool_use_id"], "a");
        assert_eq!(results[1]["tool_use_id"], "b");
    }

    #[test]
    fn an_assistant_turn_carrying_only_tool_calls_omits_the_empty_text_block() {
        let messages = vec![Message::Assistant {
            text: String::new(),
            tool_calls: vec![ToolCall {
                id: "a".into(),
                name: "ping".into(),
                input: serde_json::json!({}),
            }],
        }];

        let content = encode_messages(&messages)[0]["content"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_use");
    }

    #[test]
    fn the_request_omits_every_parameter_the_model_rejects() {
        // temperature, top_p, top_k and thinking.budget_tokens are 400s on the
        // current models — a request carrying any of them never runs at all.
        let provider = Anthropic::new("key", None);
        let tools = [ToolSpec {
            name: "ping".into(),
            description: "d".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];
        let messages = [Message::User { text: "hi".into() }];

        let body = serde_json::json!({
            "model": provider.model,
            "max_tokens": 4096u32,
            "system": "s",
            "messages": encode_messages(&messages),
            "tools": tools.iter().map(|tool| serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })).collect::<Vec<_>>(),
        });

        for rejected in ["temperature", "top_p", "top_k", "thinking"] {
            assert!(
                body.get(rejected).is_none(),
                "{rejected} must never be sent"
            );
        }
        assert_eq!(body["model"], DEFAULT_MODEL);
    }

    #[test]
    fn a_blank_model_falls_back_to_the_default() {
        assert_eq!(Anthropic::new("k", Some("  ".into())).model, DEFAULT_MODEL);
        assert_eq!(Anthropic::new("k", None).model, DEFAULT_MODEL);
        assert_eq!(
            Anthropic::new("k", Some("claude-sonnet-5".into())).model,
            "claude-sonnet-5"
        );
    }
}
