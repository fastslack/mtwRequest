//! Meta (Llama) AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "meta",
    name: "Meta (Llama)",
    base_url: "https://api.llama.com/v1",
    docs_url: "https://docs.llama.com/",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: false,
};

pub mod models {
    use super::ModelInfo;

    pub const LLAMA_4_MAVERICK: ModelInfo = ModelInfo {
        id: "Llama-4-Maverick-17B-128E-Instruct",
        name: "Llama 4 Maverick",
        context_window: 1_048_576,
        max_output_tokens: 16_384,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const LLAMA_4_SCOUT: ModelInfo = ModelInfo {
        id: "Llama-4-Scout-17B-16E-Instruct",
        name: "Llama 4 Scout",
        context_window: 512_000,
        max_output_tokens: 16_384,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const LLAMA_33_70B: ModelInfo = ModelInfo {
        id: "Llama-3.3-70B-Instruct",
        name: "Llama 3.3 70B",
        context_window: 128_000,
        max_output_tokens: 8_192,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const ALL: &[ModelInfo] = &[LLAMA_4_MAVERICK, LLAMA_4_SCOUT, LLAMA_33_70B];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaConfig {
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
    models::LLAMA_4_MAVERICK.id.to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<MetaConfig> for AiProviderConfig {
    fn from(config: MetaConfig) -> Self {
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

pub struct MetaProvider {
    config: MetaConfig,
    status: AiProviderStatus,
}

impl MetaProvider {
    pub fn new(config: MetaConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &MetaConfig {
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
