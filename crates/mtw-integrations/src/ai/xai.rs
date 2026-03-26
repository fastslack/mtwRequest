//! xAI (Grok) AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "xai",
    name: "xAI (Grok)",
    base_url: "https://api.x.ai/v1",
    docs_url: "https://docs.x.ai/",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: false,
};

pub mod models {
    use super::ModelInfo;

    pub const GROK_3: ModelInfo = ModelInfo {
        id: "grok-3",
        name: "Grok 3",
        context_window: 131_072,
        max_output_tokens: 16_384,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const GROK_3_MINI: ModelInfo = ModelInfo {
        id: "grok-3-mini",
        name: "Grok 3 Mini",
        context_window: 131_072,
        max_output_tokens: 16_384,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const ALL: &[ModelInfo] = &[GROK_3, GROK_3_MINI];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XaiConfig {
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
    models::GROK_3.id.to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<XaiConfig> for AiProviderConfig {
    fn from(config: XaiConfig) -> Self {
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

pub struct XaiProvider {
    config: XaiConfig,
    status: AiProviderStatus,
}

impl XaiProvider {
    pub fn new(config: XaiConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &XaiConfig {
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
