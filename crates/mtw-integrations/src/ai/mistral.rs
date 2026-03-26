//! Mistral AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "mistral",
    name: "Mistral AI",
    base_url: "https://api.mistral.ai/v1",
    docs_url: "https://docs.mistral.ai/api/",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: true,
};

pub mod models {
    use super::ModelInfo;

    pub const MISTRAL_LARGE: ModelInfo = ModelInfo {
        id: "mistral-large-latest",
        name: "Mistral Large",
        context_window: 128_000,
        max_output_tokens: 8_192,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const MISTRAL_MEDIUM: ModelInfo = ModelInfo {
        id: "mistral-medium-latest",
        name: "Mistral Medium",
        context_window: 128_000,
        max_output_tokens: 8_192,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const MISTRAL_SMALL: ModelInfo = ModelInfo {
        id: "mistral-small-latest",
        name: "Mistral Small",
        context_window: 128_000,
        max_output_tokens: 8_192,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const CODESTRAL: ModelInfo = ModelInfo {
        id: "codestral-latest",
        name: "Codestral",
        context_window: 32_000,
        max_output_tokens: 8_192,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const ALL: &[ModelInfo] = &[MISTRAL_LARGE, MISTRAL_MEDIUM, MISTRAL_SMALL, CODESTRAL];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MistralConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    pub default_max_tokens: Option<u32>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::MISTRAL_LARGE.id.to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<MistralConfig> for AiProviderConfig {
    fn from(config: MistralConfig) -> Self {
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

pub struct MistralProvider {
    config: MistralConfig,
    status: AiProviderStatus,
}

impl MistralProvider {
    pub fn new(config: MistralConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &MistralConfig {
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
        self.status = AiProviderStatus::Ready;
        Ok(())
    }
}
