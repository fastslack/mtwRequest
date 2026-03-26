//! DeepSeek AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "deepseek",
    name: "DeepSeek",
    base_url: "https://api.deepseek.com/v1",
    docs_url: "https://platform.deepseek.com/api-docs",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: false,
    supports_embeddings: false,
};

pub mod models {
    use super::ModelInfo;

    pub const DEEPSEEK_CHAT: ModelInfo = ModelInfo {
        id: "deepseek-chat",
        name: "DeepSeek Chat (V3)",
        context_window: 64_000,
        max_output_tokens: 8_192,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const DEEPSEEK_REASONER: ModelInfo = ModelInfo {
        id: "deepseek-reasoner",
        name: "DeepSeek Reasoner (R1)",
        context_window: 64_000,
        max_output_tokens: 8_192,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const ALL: &[ModelInfo] = &[DEEPSEEK_CHAT, DEEPSEEK_REASONER];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepSeekConfig {
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
    models::DEEPSEEK_CHAT.id.to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<DeepSeekConfig> for AiProviderConfig {
    fn from(config: DeepSeekConfig) -> Self {
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

pub struct DeepSeekProvider {
    config: DeepSeekConfig,
    status: AiProviderStatus,
}

impl DeepSeekProvider {
    pub fn new(config: DeepSeekConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &DeepSeekConfig {
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
