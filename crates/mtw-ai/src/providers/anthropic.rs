use async_trait::async_trait;
use futures::Stream;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, ModelInfo, MtwAIProvider, ProviderCapabilities,
    StreamChunk,
};

// Claude model constants
pub const CLAUDE_OPUS: &str = "claude-opus-4-6";
pub const CLAUDE_SONNET: &str = "claude-sonnet-4-6";
pub const CLAUDE_HAIKU: &str = "claude-haiku-3-5";

/// Configuration for the Anthropic provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
}

fn default_base_url() -> String {
    "https://api.anthropic.com".to_string()
}

fn default_model() -> String {
    CLAUDE_SONNET.to_string()
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: default_base_url(),
            default_model: default_model(),
        }
    }
}

/// Anthropic AI provider (Claude models)
pub struct AnthropicProvider {
    config: AnthropicConfig,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }
}

#[async_trait]
impl MtwAIProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            vision: true,
            embeddings: false,
            max_context: 200_000,
        }
    }

    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, MtwError> {
        Err(MtwError::Internal(
            "anthropic provider not yet implemented".into(),
        ))
    }

    fn stream(
        &self,
        _req: CompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>> {
        Box::pin(futures::stream::once(async {
            Err(MtwError::Internal(
                "anthropic provider streaming not yet implemented".into(),
            ))
        }))
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        Ok(vec![
            ModelInfo {
                id: CLAUDE_OPUS.to_string(),
                name: "Claude Opus 4".to_string(),
                max_context: 200_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: CLAUDE_SONNET.to_string(),
                name: "Claude Sonnet 4".to_string(),
                max_context: 200_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: CLAUDE_HAIKU.to_string(),
                name: "Claude Haiku 3.5".to_string(),
                max_context: 200_000,
                supports_tools: true,
                supports_vision: true,
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_creation() {
        let config = AnthropicConfig::new("sk-test-key");
        assert_eq!(config.api_key, "sk-test-key");
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.default_model, CLAUDE_SONNET);
    }

    #[test]
    fn test_provider_name() {
        let provider = AnthropicProvider::new(AnthropicConfig::new("test"));
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_capabilities() {
        let provider = AnthropicProvider::new(AnthropicConfig::new("test"));
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(caps.tool_calling);
        assert!(caps.vision);
        assert!(!caps.embeddings);
        assert_eq!(caps.max_context, 200_000);
    }

    #[tokio::test]
    async fn test_models_list() {
        let provider = AnthropicProvider::new(AnthropicConfig::new("test"));
        let models = provider.models().await.unwrap();
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.id == CLAUDE_SONNET));
        assert!(models.iter().any(|m| m.id == CLAUDE_OPUS));
    }

    #[tokio::test]
    async fn test_complete_not_implemented() {
        let provider = AnthropicProvider::new(AnthropicConfig::new("test"));
        let req = CompletionRequest::default();
        let result = provider.complete(req).await;
        assert!(result.is_err());
    }
}
