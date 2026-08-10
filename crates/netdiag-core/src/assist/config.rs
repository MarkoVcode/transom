//! Which model to use, and how much of the network to tell it.
//!
//! Stored per installation rather than per network: the choice of model is
//! about this machine and its operator, not about the site being diagnosed.
//! The API key is **not** here — it lives in the OS keychain, the same
//! separation the controller password uses.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    /// A cloud model. Best reasoning; evidence leaves the machine.
    #[default]
    Anthropic,
    /// A local model. Nothing leaves the machine; weaker at multi-step tool use.
    Ollama,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistConfig {
    pub provider: ProviderKind,
    /// Overrides the backend's default model when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Ollama's base URL. Ignored by the cloud provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Replace identifiers with stable placeholders before sending.
    ///
    /// Defaults to on. A diagnosis needs vendors, models, speeds, error counts
    /// and signal levels — none of which identify anyone. It does not need the
    /// MAC of the user's phone or the name of their Wi-Fi, so those do not go.
    #[serde(default = "default_true")]
    pub redact: bool,
    /// Whether the assistant is available at all. Off until configured, so the
    /// app never implies it is talking to a model it has not been given.
    #[serde(default)]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AssistConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::default(),
            model: None,
            endpoint: None,
            redact: true,
            enabled: false,
        }
    }
}

impl AssistConfig {
    pub fn path(root: &Path) -> PathBuf {
        root.join("assistant.json")
    }

    pub async fn load(root: &Path) -> Option<Self> {
        let bytes = tokio::fs::read(Self::path(root)).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub async fn save(&self, root: &Path) -> Result<(), String> {
        tokio::fs::create_dir_all(root)
            .await
            .map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        tokio::fs::write(Self::path(root), json)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn delete(root: &Path) -> Result<(), String> {
        match tokio::fs::remove_file(Self::path(root)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Whether this configuration can actually run.
    ///
    /// The cloud provider additionally needs a key, which lives in the keychain
    /// and so cannot be checked from here.
    pub fn is_usable(&self) -> bool {
        self.enabled
    }

    /// A stable identity for the keychain entry.
    ///
    /// Keyed by provider rather than by model so that changing model does not
    /// orphan the stored key.
    pub fn credential_id(&self) -> String {
        match self.provider {
            ProviderKind::Anthropic => "assistant:anthropic".into(),
            ProviderKind::Ollama => "assistant:ollama".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_is_on_by_default_and_the_assistant_is_off() {
        let config = AssistConfig::default();
        assert!(
            config.redact,
            "evidence must not leave unredacted by accident"
        );
        assert!(
            !config.enabled,
            "the app must not imply a model it was never given"
        );
    }

    #[test]
    fn a_stored_config_without_the_redact_field_still_redacts() {
        // Defaulting this to false on an older file would silently start
        // sending identifiers that were never meant to leave.
        let config: AssistConfig =
            serde_json::from_str(r#"{"provider":"anthropic","enabled":true}"#).unwrap();
        assert!(config.redact);
        assert!(config.enabled);
    }

    #[test]
    fn the_credential_id_survives_a_model_change() {
        let mut config = AssistConfig::default();
        let before = config.credential_id();
        config.model = Some("claude-sonnet-5".into());
        assert_eq!(config.credential_id(), before);
    }

    #[test]
    fn switching_provider_changes_the_credential_id() {
        let cloud = AssistConfig::default();
        let local = AssistConfig {
            provider: ProviderKind::Ollama,
            ..Default::default()
        };
        assert_ne!(cloud.credential_id(), local.credential_id());
    }

    #[test]
    fn the_config_round_trips_without_carrying_a_key() {
        let config = AssistConfig {
            provider: ProviderKind::Ollama,
            model: Some("qwen2.5".into()),
            endpoint: Some("http://127.0.0.1:11434".into()),
            redact: false,
            enabled: true,
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("key") && !json.contains("token"),
            "secrets belong in the keychain, never in this file: {json}"
        );

        let back: AssistConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, ProviderKind::Ollama);
        assert_eq!(back.model.as_deref(), Some("qwen2.5"));
        assert!(!back.redact);
    }
}
