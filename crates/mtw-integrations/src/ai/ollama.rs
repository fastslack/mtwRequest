//! Ollama (local models) AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "ollama",
    name: "Ollama (Local)",
    base_url: "http://localhost:11434/api",
    docs_url: "https://github.com/ollama/ollama/blob/main/docs/api.md",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: true,
};

/// Common models available through Ollama.
/// Unlike cloud providers, the actual available models depend on what the
/// user has pulled locally.
pub mod models {
    use super::ModelInfo;

    pub const LLAMA3: ModelInfo = ModelInfo {
        id: "llama3",
        name: "Llama 3 (8B)",
        context_window: 8_192,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const LLAMA3_70B: ModelInfo = ModelInfo {
        id: "llama3:70b",
        name: "Llama 3 (70B)",
        context_window: 8_192,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const MISTRAL: ModelInfo = ModelInfo {
        id: "mistral",
        name: "Mistral (7B)",
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const LLAVA: ModelInfo = ModelInfo {
        id: "llava",
        name: "LLaVA (Vision)",
        context_window: 4_096,
        max_output_tokens: 4_096,
        supports_vision: true,
        supports_tool_calling: false,
    };

    pub const CODELLAMA: ModelInfo = ModelInfo {
        id: "codellama",
        name: "Code Llama",
        context_window: 16_384,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const ALL: &[ModelInfo] = &[LLAMA3, LLAMA3_70B, MISTRAL, LLAVA, CODELLAMA];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    /// Ollama server base URL.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default model to use.
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Request timeout in seconds (local models can be slow).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Keep model loaded in memory after request.
    #[serde(default = "default_keep_alive")]
    pub keep_alive: bool,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::LLAMA3.id.to_string()
}

fn default_timeout() -> u64 {
    300 // local models may need longer
}

fn default_keep_alive() -> bool {
    true
}

pub struct OllamaProvider {
    config: OllamaConfig,
    status: AiProviderStatus,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Unavailable("not connected".to_string()),
        }
    }

    pub fn config(&self) -> &OllamaConfig {
        &self.config
    }

    pub fn status(&self) -> &AiProviderStatus {
        &self.status
    }

    pub fn info() -> &'static AiProviderInfo {
        &INFO
    }

    pub fn common_models() -> &'static [ModelInfo] {
        models::ALL
    }

    /// Check if the Ollama server is reachable.
    pub async fn validate(&mut self) -> Result<(), String> {
        // TODO: GET /api/tags to list locally available models
        self.status = AiProviderStatus::Ready;
        Ok(())
    }

    /// List locally available models.
    pub async fn list_local_models(&self) -> Result<Vec<String>, String> {
        // TODO: GET /api/tags
        Err("list_local_models not yet implemented".to_string())
    }

    /// Pull a model from the Ollama registry.
    pub async fn pull_model(&self, _model: &str) -> Result<(), String> {
        // TODO: POST /api/pull
        Err("pull_model not yet implemented".to_string())
    }
}
