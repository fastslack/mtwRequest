//! OpenAI (GPT) AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "openai",
    name: "OpenAI",
    base_url: "https://api.openai.com/v1",
    docs_url: "https://platform.openai.com/docs/api-reference",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: true,
};

pub mod models {
    use super::ModelInfo;

    pub const GPT4O: ModelInfo = ModelInfo {
        id: "gpt-4o",
        name: "GPT-4o",
        context_window: 128_000,
        max_output_tokens: 16_384,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const GPT4O_MINI: ModelInfo = ModelInfo {
        id: "gpt-4o-mini",
        name: "GPT-4o Mini",
        context_window: 128_000,
        max_output_tokens: 16_384,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const O1: ModelInfo = ModelInfo {
        id: "o1",
        name: "o1",
        context_window: 200_000,
        max_output_tokens: 100_000,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const O3_MINI: ModelInfo = ModelInfo {
        id: "o3-mini",
        name: "o3-mini",
        context_window: 200_000,
        max_output_tokens: 100_000,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const ALL: &[ModelInfo] = &[GPT4O, GPT4O_MINI, O1, O3_MINI];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Organization ID (optional).
    pub organization: Option<String>,
    pub default_max_tokens: Option<u32>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::GPT4O.id.to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<OpenAiConfig> for AiProviderConfig {
    fn from(config: OpenAiConfig) -> Self {
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

pub struct OpenAiProvider {
    config: OpenAiConfig,
    status: AiProviderStatus,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &OpenAiConfig {
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
