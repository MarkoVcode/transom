//! Ollama, for users who will not send network data anywhere.
//!
//! Runs against a local `/api/chat` endpoint. The trade is stated plainly in
//! the UI rather than hidden: local models are markedly weaker at multi-step
//! tool use, which is the whole mechanism here, so a diagnosis from this
//! backend is worth less than one from a frontier model. It exists because a
//! worse diagnosis that never leaves the machine beats no diagnosis at all for
//! anyone who cannot share their topology.

use serde::Deserialize;
use std::time::Duration;

use super::http::post_json;
use super::provider::{
    brief, ChatProvider, Message, ProviderError, Request, StopReason, ToolCall, Turn, Usage,
};

pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";
pub const DEFAULT_MODEL: &str = "llama3.1";

const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;

pub struct Ollama {
    endpoint: String,
    model: String,
    timeout: Duration,
}

impl Ollama {
    pub fn new(endpoint: Option<String>, model: Option<String>) -> Self {
        let endpoint = endpoint
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());

        Self {
            endpoint,
            model: model
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            // Local inference on CPU is slow; a short timeout would report a
            // working setup as broken.
            timeout: Duration::from_secs(600),
        }
    }
}

impl ChatProvider for Ollama {
    fn describe(&self) -> String {
        format!("Ollama {} at {}", self.model, self.endpoint)
    }

    async fn complete(&self, request: Request<'_>) -> Result<Turn, ProviderError> {
        let mut messages = vec![serde_json::json!({
            "role": "system",
            "content": request.system,
        })];
        messages.extend(encode_messages(request.messages));

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": request.tools.iter().map(|tool| serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                },
            })).collect::<Vec<_>>(),
            // One JSON object rather than a token stream: the loop consumes
            // whole turns, and streaming would only complicate the parse.
            "stream": false,
            "options": {"num_predict": request.max_tokens},
        });

        let payload = serde_json::to_string(&body)
            .map_err(|e| ProviderError::Protocol(format!("could not encode the request: {e}")))?;

        let response = post_json(
            &format!("{}/api/chat", self.endpoint),
            &payload,
            &[],
            self.timeout,
            RESPONSE_LIMIT,
        )
        .await
        .map_err(|error| {
            ProviderError::Transport(format!("{error}. Is Ollama running at {}?", self.endpoint))
        })?;

        match response.status {
            200 => decode_turn(&response.body),
            404 => Err(ProviderError::Protocol(format!(
                "{} — the model may not be pulled yet: try `ollama pull {}`",
                brief(&response.body, 160),
                self.model
            ))),
            429 | 503 => Err(ProviderError::Busy(brief(&response.body, 200))),
            status => Err(ProviderError::Protocol(format!(
                "HTTP {status}: {}",
                brief(&response.body, 300)
            ))),
        }
    }
}

fn encode_messages(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|message| match message {
            Message::User { text } => serde_json::json!({"role": "user", "content": text}),
            Message::Assistant { text, tool_calls } => {
                let mut value = serde_json::json!({"role": "assistant", "content": text});
                if !tool_calls.is_empty() {
                    value["tool_calls"] = tool_calls
                        .iter()
                        .map(|call| {
                            serde_json::json!({
                                "function": {"name": call.name, "arguments": call.input},
                            })
                        })
                        .collect();
                }
                value
            }
            // Ollama pairs a result to its call by tool name, not by id.
            Message::ToolResult { name, content, .. } => serde_json::json!({
                "role": "tool",
                "tool_name": name,
                "content": content,
            }),
        })
        .collect()
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    message: Option<ApiMessage>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[derive(Deserialize, Default)]
struct ApiMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<ApiToolCall>,
}

#[derive(Deserialize)]
struct ApiToolCall {
    function: ApiFunction,
}

#[derive(Deserialize)]
struct ApiFunction {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

fn decode_turn(body: &str) -> Result<Turn, ProviderError> {
    let response: ApiResponse = serde_json::from_str(body)
        .map_err(|e| ProviderError::Protocol(format!("could not read the response: {e}")))?;

    let message = response.message.unwrap_or_default();

    let tool_calls: Vec<ToolCall> = message
        .tool_calls
        .into_iter()
        .enumerate()
        .map(|(index, call)| ToolCall {
            // Ollama assigns no call ids. The loop needs a stable key to pair
            // each result back to its call, so one is synthesised from the
            // position within this turn.
            id: format!("{}-{index}", call.function.name),
            name: call.function.name,
            input: normalise_arguments(call.function.arguments),
        })
        .collect();

    let stop_reason = if tool_calls.is_empty() {
        StopReason::EndTurn
    } else {
        StopReason::ToolUse
    };

    Ok(Turn {
        text: message.content,
        tool_calls,
        stop_reason,
        usage: Usage {
            input_tokens: response.prompt_eval_count.unwrap_or(0),
            output_tokens: response.eval_count.unwrap_or(0),
        },
    })
}

/// Some models emit arguments as a JSON *string* rather than an object.
///
/// Handing that to a tool as-is fails schema validation for a reason the user
/// cannot act on, so it is parsed once here rather than in every tool.
fn normalise_arguments(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => {
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_call_is_decoded_and_given_a_stable_id() {
        let body = r#"{"message":{"role":"assistant","content":"",
            "tool_calls":[{"function":{"name":"ping","arguments":{"target":"10.0.3.1"}}}]},
            "prompt_eval_count": 900, "eval_count": 40}"#;

        let turn = decode_turn(body).unwrap();
        assert_eq!(turn.stop_reason, StopReason::ToolUse);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].id, "ping-0");
        assert_eq!(turn.tool_calls[0].input["target"], "10.0.3.1");
        assert_eq!(turn.usage.input_tokens, 900);
    }

    #[test]
    fn stringified_arguments_are_parsed_into_an_object() {
        // Several local models emit arguments as a JSON string; passing that
        // through would fail every tool's schema for an opaque reason.
        let body = r#"{"message":{"tool_calls":[
            {"function":{"name":"ping","arguments":"{\"target\":\"10.0.3.1\"}"}}]}}"#;
        let turn = decode_turn(body).unwrap();
        assert_eq!(turn.tool_calls[0].input["target"], "10.0.3.1");
    }

    #[test]
    fn arguments_that_are_genuinely_a_string_survive() {
        let body = r#"{"message":{"tool_calls":[
            {"function":{"name":"note","arguments":"just text"}}]}}"#;
        let turn = decode_turn(body).unwrap();
        assert_eq!(turn.tool_calls[0].input, serde_json::json!("just text"));
    }

    #[test]
    fn a_plain_answer_ends_the_turn() {
        let body = r#"{"message":{"role":"assistant","content":"The link is fine."}}"#;
        let turn = decode_turn(body).unwrap();
        assert_eq!(turn.stop_reason, StopReason::EndTurn);
        assert_eq!(turn.text, "The link is fine.");
        assert!(turn.tool_calls.is_empty());
    }

    #[test]
    fn parallel_calls_get_distinct_ids_even_with_one_tool() {
        let body = r#"{"message":{"tool_calls":[
            {"function":{"name":"ping","arguments":{"target":"a"}}},
            {"function":{"name":"ping","arguments":{"target":"b"}}}]}}"#;
        let turn = decode_turn(body).unwrap();
        assert_eq!(turn.tool_calls[0].id, "ping-0");
        assert_eq!(turn.tool_calls[1].id, "ping-1");
    }

    #[test]
    fn the_endpoint_is_normalised_and_defaulted() {
        assert_eq!(Ollama::new(None, None).endpoint, DEFAULT_ENDPOINT);
        assert_eq!(
            Ollama::new(Some("http://box:11434/".into()), None).endpoint,
            "http://box:11434"
        );
        assert_eq!(
            Ollama::new(Some("   ".into()), None).endpoint,
            DEFAULT_ENDPOINT
        );
    }

    #[test]
    fn tool_results_are_keyed_by_name_not_id() {
        // The reason `Message::ToolResult` carries both: Anthropic pairs on
        // id, Ollama on name.
        let encoded = encode_messages(&[Message::ToolResult {
            call_id: "ping-0".into(),
            name: "ping".into(),
            content: "1ms".into(),
            is_error: false,
        }]);
        assert_eq!(encoded[0]["role"], "tool");
        assert_eq!(encoded[0]["tool_name"], "ping");
    }
}
