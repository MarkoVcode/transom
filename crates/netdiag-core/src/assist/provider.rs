//! What a model provider must do, stated once.
//!
//! The assistant's value is in the evidence it gathers and the reasoning over
//! it, not in which model does the reasoning — so the loop is written against
//! this trait and each backend adapts its own wire format to these types.
//!
//! Only what the loop actually needs is modelled. There is no streaming here:
//! a diagnosis is a sequence of tool calls whose *results* the user cares
//! about, and token-by-token text between tool calls is noise. Progress is
//! reported from the loop instead, the same way scan phases are.

use serde::{Deserialize, Serialize};

/// One turn of the conversation.
///
/// Tool results are a message role rather than a side channel because that is
/// how both providers model them, and flattening them here would mean
/// re-deriving the association on the way out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum Message {
    /// What the user said — the symptom, or an answer to a question.
    #[serde(rename_all = "camelCase")]
    User { text: String },
    /// What the model said, plus any tools it wants run.
    #[serde(rename_all = "camelCase")]
    Assistant {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    /// The outcome of one tool call, keyed back to it.
    #[serde(rename_all = "camelCase")]
    ToolResult {
        call_id: String,
        /// The tool's name. Anthropic keys results by id alone; Ollama keys
        /// them by name, so both are carried.
        name: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// A tool offered to the model. `input_schema` is JSON Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool the model asked to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    /// Provider-assigned id. Ollama assigns none, so one is synthesised — the
    /// loop needs a stable key to pair a result back to its call.
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Why the model stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    /// It finished speaking.
    EndTurn,
    /// It wants tools run before continuing.
    ToolUse,
    /// It ran out of output budget mid-answer.
    MaxTokens,
    /// The provider's safety classifiers declined the request.
    Refusal,
}

/// One model response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// What the loop asks a provider for.
pub struct Request<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolSpec],
    pub max_tokens: u32,
}

#[derive(Debug)]
pub enum ProviderError {
    /// The credential is missing, wrong, or lacks access.
    Auth(String),
    /// The provider could not be reached.
    Transport(String),
    /// Reached, but it rejected the request or answered unintelligibly.
    Protocol(String),
    /// Rate limited or overloaded — worth retrying.
    Busy(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Auth(detail) => write!(f, "authentication failed: {detail}"),
            ProviderError::Transport(detail) => write!(f, "could not reach the model: {detail}"),
            ProviderError::Protocol(detail) => write!(f, "unexpected response: {detail}"),
            ProviderError::Busy(detail) => write!(f, "the model is busy: {detail}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// A model that can hold a tool-using conversation.
pub trait ChatProvider: Send + Sync {
    /// How this provider should be named to the user.
    fn describe(&self) -> String;

    fn complete(
        &self,
        request: Request<'_>,
    ) -> impl std::future::Future<Output = Result<Turn, ProviderError>> + Send;
}

/// Truncates a provider's error body to something a UI can show.
///
/// Provider errors are JSON and can be long; the first line of the message is
/// the part a user can act on.
pub(crate) fn brief(body: &str, limit: usize) -> String {
    let text = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .or_else(|| value.get("error"))
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());

    if text.chars().count() <= limit {
        return text;
    }
    text.chars().take(limit).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_error_body_is_reduced_to_its_message() {
        let body = r#"{"type":"error","error":{"type":"authentication_error",
                       "message":"invalid x-api-key"}}"#;
        assert_eq!(brief(body, 200), "invalid x-api-key");
    }

    #[test]
    fn a_plain_string_error_field_is_also_understood() {
        // Ollama reports errors as a bare string under `error`.
        assert_eq!(
            brief(r#"{"error":"model not found"}"#, 200),
            "model not found"
        );
    }

    #[test]
    fn an_unparseable_body_falls_back_to_its_text() {
        assert_eq!(brief("  gateway timeout  ", 200), "gateway timeout");
    }

    #[test]
    fn a_long_message_is_truncated_rather_than_dumped() {
        let long = "x".repeat(500);
        let body = format!(r#"{{"error":{{"message":"{long}"}}}}"#);
        let briefed = brief(&body, 50);
        assert_eq!(
            briefed.chars().count(),
            51,
            "50 characters plus an ellipsis"
        );
        assert!(briefed.ends_with('…'));
    }

    #[test]
    fn messages_round_trip_through_json() {
        // The transcript is persisted with the case, so these must survive a
        // save and reload unchanged.
        let messages = vec![
            Message::User {
                text: "uploads are slow".into(),
            },
            Message::Assistant {
                text: "Checking the switch port.".into(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "device_detail".into(),
                    input: serde_json::json!({"ip": "10.0.3.22"}),
                }],
            },
            Message::ToolResult {
                call_id: "call-1".into(),
                name: "device_detail".into(),
                content: "{}".into(),
                is_error: false,
            },
        ];

        let json = serde_json::to_string(&messages).unwrap();
        let back: Vec<Message> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 3);
        assert!(matches!(&back[2], Message::ToolResult { call_id, .. } if call_id == "call-1"));
    }
}
