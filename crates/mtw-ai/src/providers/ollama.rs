use async_trait::async_trait;
use futures::Stream;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, ModelInfo, MtwAIProvider, ProviderCapabilities,
    StreamChunk,
};

// Common local model identifiers
pub const LLAMA3: &str = "llama3";
pub const MISTRAL: &str = "mistral";
pub const CODELLAMA: &str = "codellama";
pub const PHI3: &str = "phi3";

/// Configuration for the Ollama provider (local models)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
}

fn default_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_model() -> String {
    LLAMA3.to_string()
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            default_model: default_model(),
        }
    }
}

impl OllamaConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }
}

/// Ollama AI provider for local models
pub struct OllamaProvider {
    config: OllamaConfig,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &OllamaConfig {
        &self.config
    }
}

#[async_trait]
impl MtwAIProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: false,
            vision: false,
            embeddings: true,
            max_context: 8192,
        }
    }

    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, MtwError> {
        Err(MtwError::Internal(
            "ollama provider not yet implemented".into(),
        ))
    }

    fn stream(
        &self,
        _req: CompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>> {
        Box::pin(futures::stream::once(async {
            Err(MtwError::Internal(
                "ollama provider streaming not yet implemented".into(),
            ))
        }))
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        // In a real implementation, this would query the Ollama API
        Ok(vec![
            ModelInfo {
                id: LLAMA3.to_string(),
                name: "Llama 3".to_string(),
                max_context: 8192,
                supports_tools: false,
                supports_vision: false,
            },
            ModelInfo {
                id: MISTRAL.to_string(),
                name: "Mistral".to_string(),
                max_context: 8192,
                supports_tools: false,
                supports_vision: false,
            },
            ModelInfo {
                id: CODELLAMA.to_string(),
                name: "Code Llama".to_string(),
                max_context: 16384,
                supports_tools: false,
                supports_vision: false,
            },
            ModelInfo {
                id: PHI3.to_string(),
                name: "Phi-3".to_string(),
                max_context: 4096,
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
    fn test_default_config() {
        let config = OllamaConfig::default();
        assert_eq!(config.base_url, "http://localhost:11434");
        assert_eq!(config.default_model, LLAMA3);
    }

    #[test]
    fn test_config_builder() {
        let config = OllamaConfig::new()
            .with_base_url("http://gpu-server:11434")
            .with_model(MISTRAL);
        assert_eq!(config.base_url, "http://gpu-server:11434");
        assert_eq!(config.default_model, MISTRAL);
    }

    #[test]
    fn test_provider_name() {
        let provider = OllamaProvider::new(OllamaConfig::default());
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_capabilities() {
        let provider = OllamaProvider::new(OllamaConfig::default());
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(!caps.tool_calling);
        assert!(!caps.vision);
        assert!(caps.embeddings);
    }

    #[tokio::test]
    async fn test_models_list() {
        let provider = OllamaProvider::new(OllamaConfig::default());
        let models = provider.models().await.unwrap();
        assert_eq!(models.len(), 4);
        assert!(models.iter().any(|m| m.id == LLAMA3));
    }

    #[tokio::test]
    async fn test_complete_not_implemented() {
        let provider = OllamaProvider::new(OllamaConfig::default());
        let req = CompletionRequest::default();
        let result = provider.complete(req).await;
        assert!(result.is_err());
    }
}
