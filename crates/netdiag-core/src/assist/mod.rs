//! The troubleshooting assistant's model layer.
//!
//! A user describes a symptom in their own words; the assistant decides what to
//! measure, asks what only they can answer, and explains what the evidence
//! means. This module is the part that talks to a model. The evidence itself
//! comes from the engine, and the rule the whole feature rests on is:
//!
//! > The model never measures and never guesses a number. Deterministic code
//! > produces evidence; the model chooses what to measure next and explains it.
//!
//! Two backends sit behind one trait so that choice is the user's: a cloud
//! model for the best reasoning, or a local one for people who cannot send
//! their network's topology anywhere. Neither is assumed.

pub mod anthropic;
pub mod case;
pub mod config;
pub mod http;
pub mod observe;
pub mod ollama;
pub mod provider;
pub mod redact;
pub mod session;
pub mod tools;

pub use case::{Case, CaseStatus, CaseSummary, Confidence, Finding, Question, Remedy};
pub use config::{AssistConfig, ProviderKind};
pub use provider::{
    ChatProvider, Message, ProviderError, Request, StopReason, ToolCall, ToolSpec, Turn, Usage,
};
pub use redact::Redactor;
pub use session::{AssistEvent, Cancel, Limits};
pub use tools::ToolContext;

/// Builds the configured provider.
///
/// The API key is passed in rather than read here: the engine crate holds no
/// secrets and has no keychain dependency, exactly as with the controller
/// password. That is what keeps it buildable and testable with no system
/// libraries.
pub fn build(
    config: &AssistConfig,
    api_key: Option<String>,
) -> Result<Box<dyn DynChatProvider>, String> {
    match config.provider {
        ProviderKind::Anthropic => {
            let key = api_key
                .filter(|key| !key.trim().is_empty())
                .ok_or("No API key is saved for the cloud model")?;
            Ok(Box::new(anthropic::Anthropic::new(
                key,
                config.model.clone(),
            )))
        }
        ProviderKind::Ollama => Ok(Box::new(ollama::Ollama::new(
            config.endpoint.clone(),
            config.model.clone(),
        ))),
    }
}

/// Object-safe form of [`ChatProvider`].
///
/// [`ChatProvider`] uses an `async fn`, which is convenient to implement and
/// not object-safe. The loop needs to hold whichever backend the user picked
/// without being generic over it, so the boxed-future form exists alongside it
/// and is blanket-implemented — each backend still writes the plain `async fn`.
pub trait DynChatProvider: Send + Sync {
    fn describe(&self) -> String;

    fn complete<'a>(
        &'a self,
        request: Request<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Turn, ProviderError>> + Send + 'a>>;
}

impl<T: ChatProvider> DynChatProvider for T {
    fn describe(&self) -> String {
        ChatProvider::describe(self)
    }

    fn complete<'a>(
        &'a self,
        request: Request<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Turn, ProviderError>> + Send + 'a>>
    {
        Box::pin(ChatProvider::complete(self, request))
    }
}

/// Confirms the configured model answers, without spending a diagnosis on it.
///
/// Mirrors the controller's "Test connection": the failure a user needs to see
/// is a wrong key or an unreachable endpoint, and finding that out at the start
/// of a real troubleshooting session is the worst possible moment.
pub async fn verify(
    config: &AssistConfig,
    api_key: Option<String>,
) -> Result<String, ProviderError> {
    let provider = build(config, api_key).map_err(ProviderError::Auth)?;

    let messages = [Message::User {
        text: "Reply with the single word: ready".into(),
    }];

    let turn = provider
        .complete(Request {
            system: "You are being checked for connectivity. Answer in one word.",
            messages: &messages,
            tools: &[],
            max_tokens: 32,
        })
        .await?;

    if turn.stop_reason == StopReason::Refusal {
        return Err(ProviderError::Protocol(
            "the model declined a connectivity check — the credential works, but the request was \
             refused"
                .into(),
        ));
    }

    Ok(format!(
        "{} answered ({} tokens in, {} out).",
        provider.describe(),
        turn.usage.input_tokens,
        turn.usage.output_tokens
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cloud_provider_refuses_to_build_without_a_key() {
        // Better a named error at setup than an authentication failure in the
        // middle of a diagnosis.
        let config = AssistConfig {
            provider: ProviderKind::Anthropic,
            ..Default::default()
        };
        assert!(build(&config, None).is_err());
        assert!(build(&config, Some("  ".into())).is_err());
        assert!(build(&config, Some("sk-ant-x".into())).is_ok());
    }

    #[test]
    fn the_local_provider_needs_no_key() {
        let config = AssistConfig {
            provider: ProviderKind::Ollama,
            ..Default::default()
        };
        assert!(build(&config, None).is_ok());
    }

    #[test]
    fn a_built_provider_describes_itself_for_the_ui() {
        let config = AssistConfig {
            provider: ProviderKind::Ollama,
            model: Some("qwen2.5".into()),
            ..Default::default()
        };
        let provider = build(&config, None).unwrap();
        assert!(provider.describe().contains("qwen2.5"));
    }
}
