use async_trait::async_trait;
use futures::Stream;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, ModelInfo, MtwAIProvider, ProviderCapabilities,
    StreamChunk,
};

// OpenAI model constants
pub const GPT_4O: &str = "gpt-4o";
pub const GPT_4O_MINI: &str = "gpt-4o-mini";
pub const GPT_4_TURBO: &str = "gpt-4-turbo";
pub const O1: &str = "o1";
pub const O1_MINI: &str = "o1-mini";

/// Configuration for the OpenAI provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_model() -> String {
    GPT_4O.to_string()
}

impl OpenAIConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: default_base_url(),
            default_model: default_model(),
        }
    }
}

/// OpenAI AI provider (GPT models)
pub struct OpenAIProvider {
    config: OpenAIConfig,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &OpenAIConfig {
        &self.config
    }
}

#[async_trait]
impl MtwAIProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            vision: true,
            embeddings: true,
            max_context: 128_000,
        }
    }

    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, MtwError> {
        Err(MtwError::Internal(
            "openai provider not yet implemented".into(),
        ))
    }

    fn stream(
        &self,
        _req: CompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>> {
        Box::pin(futures::stream::once(async {
            Err(MtwError::Internal(
                "openai provider streaming not yet implemented".into(),
            ))
        }))
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        Ok(vec![
            ModelInfo {
                id: GPT_4O.to_string(),
                name: "GPT-4o".to_string(),
                max_context: 128_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: GPT_4O_MINI.to_string(),
                name: "GPT-4o Mini".to_string(),
                max_context: 128_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: GPT_4_TURBO.to_string(),
                name: "GPT-4 Turbo".to_string(),
                max_context: 128_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: O1.to_string(),
                name: "o1".to_string(),
                max_context: 200_000,
                supports_tools: false,
                supports_vision: true,
            },
            ModelInfo {
                id: O1_MINI.to_string(),
                name: "o1-mini".to_string(),
                max_context: 128_000,
                supports_tools: false,
                supports_vision: false,
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_creation() {
        let config = OpenAIConfig::new("sk-test-key");
        assert_eq!(config.api_key, "sk-test-key");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.default_model, GPT_4O);
    }

    #[test]
    fn test_provider_name() {
        let provider = OpenAIProvider::new(OpenAIConfig::new("test"));
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_capabilities() {
        let provider = OpenAIProvider::new(OpenAIConfig::new("test"));
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(caps.tool_calling);
        assert!(caps.vision);
        assert!(caps.embeddings);
        assert_eq!(caps.max_context, 128_000);
    }

    #[tokio::test]
    async fn test_models_list() {
        let provider = OpenAIProvider::new(OpenAIConfig::new("test"));
        let models = provider.models().await.unwrap();
        assert!(models.len() >= 4);
        assert!(models.iter().any(|m| m.id == GPT_4O));
    }

    #[tokio::test]
    async fn test_complete_not_implemented() {
        let provider = OpenAIProvider::new(OpenAIConfig::new("test"));
        let req = CompletionRequest::default();
        let result = provider.complete(req).await;
        assert!(result.is_err());
    }
}
