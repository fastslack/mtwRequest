//! Anthropic (Claude) AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "anthropic",
    name: "Anthropic",
    base_url: "https://api.anthropic.com/v1",
    docs_url: "https://docs.anthropic.com/en/api",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: false,
};

/// Supported Anthropic models.
pub mod models {
    use super::ModelInfo;

    pub const CLAUDE_OPUS_4: ModelInfo = ModelInfo {
        id: "claude-opus-4-20250514",
        name: "Claude Opus 4",
        context_window: 200_000,
        max_output_tokens: 32_000,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const CLAUDE_SONNET_4: ModelInfo = ModelInfo {
        id: "claude-sonnet-4-20250514",
        name: "Claude Sonnet 4",
        context_window: 200_000,
        max_output_tokens: 16_000,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const CLAUDE_HAIKU_35: ModelInfo = ModelInfo {
        id: "claude-3-5-haiku-20241022",
        name: "Claude 3.5 Haiku",
        context_window: 200_000,
        max_output_tokens: 8_192,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const ALL: &[ModelInfo] = &[CLAUDE_OPUS_4, CLAUDE_SONNET_4, CLAUDE_HAIKU_35];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    /// API key (starts with "sk-ant-").
    pub api_key: String,
    /// API base URL.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default model.
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Anthropic API version header.
    #[serde(default = "default_api_version")]
    pub api_version: String,
    /// Default max tokens.
    pub default_max_tokens: Option<u32>,
    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::CLAUDE_SONNET_4.id.to_string()
}

fn default_api_version() -> String {
    "2023-06-01".to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<AnthropicConfig> for AiProviderConfig {
    fn from(config: AnthropicConfig) -> Self {
        AiProviderConfig {
            api_key: config.api_key,
            base_url: Some(config.base_url),
            default_model: Some(config.default_model),
            default_temperature: None,
            default_max_tokens: config.default_max_tokens,
            timeout_secs: config.timeout_secs,
        }
    }
}

pub struct AnthropicProvider {
    config: AnthropicConfig,
    status: AiProviderStatus,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }

    pub fn status(&self) -> &AiProviderStatus {
        &self.status
    }

    pub fn info() -> &'static AiProviderInfo {
        &INFO
    }

    pub fn supported_models() -> &'static [ModelInfo] {
        models::ALL
    }

    pub async fn validate(&mut self) -> Result<(), String> {
        // TODO: call /v1/messages with a minimal request to validate key
        self.status = AiProviderStatus::Ready;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let json = r#"{"api_key": "sk-ant-test"}"#;
        let config: AnthropicConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "https://api.anthropic.com/v1");
        assert_eq!(config.default_model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_models() {
        assert_eq!(models::ALL.len(), 3);
        assert!(models::CLAUDE_OPUS_4.context_window >= 200_000);
    }

    #[test]
    fn test_provider_creation() {
        let provider = AnthropicProvider::new(AnthropicConfig {
            api_key: "sk-ant-test".to_string(),
            base_url: default_base_url(),
            default_model: default_model(),
            api_version: default_api_version(),
            default_max_tokens: Some(4096),
            timeout_secs: 120,
        });
        assert_eq!(*provider.status(), AiProviderStatus::Ready);
    }
}
