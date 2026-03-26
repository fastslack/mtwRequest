//! Cohere AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "cohere",
    name: "Cohere",
    base_url: "https://api.cohere.com/v2",
    docs_url: "https://docs.cohere.com/reference/about",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: false,
    supports_embeddings: true,
};

pub mod models {
    use super::ModelInfo;

    pub const COMMAND_R_PLUS: ModelInfo = ModelInfo {
        id: "command-r-plus",
        name: "Command R+",
        context_window: 128_000,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const COMMAND_R: ModelInfo = ModelInfo {
        id: "command-r",
        name: "Command R",
        context_window: 128_000,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const COMMAND_LIGHT: ModelInfo = ModelInfo {
        id: "command-light",
        name: "Command Light",
        context_window: 4_096,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const ALL: &[ModelInfo] = &[COMMAND_R_PLUS, COMMAND_R, COMMAND_LIGHT];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CohereConfig {
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
    models::COMMAND_R_PLUS.id.to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<CohereConfig> for AiProviderConfig {
    fn from(config: CohereConfig) -> Self {
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

pub struct CohereProvider {
    config: CohereConfig,
    status: AiProviderStatus,
}

impl CohereProvider {
    pub fn new(config: CohereConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &CohereConfig {
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
