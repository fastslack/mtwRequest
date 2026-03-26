//! HuggingFace Inference API provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "huggingface",
    name: "HuggingFace Inference",
    base_url: "https://api-inference.huggingface.co",
    docs_url: "https://huggingface.co/docs/api-inference",
    supports_streaming: true,
    supports_tool_calling: false,
    supports_vision: true,
    supports_embeddings: true,
};

/// Some popular models on HuggingFace. The actual model selection is
/// virtually unlimited since users can point to any hosted model.
pub mod models {
    use super::ModelInfo;

    pub const ZEPHYR_7B: ModelInfo = ModelInfo {
        id: "HuggingFaceH4/zephyr-7b-beta",
        name: "Zephyr 7B",
        context_window: 8_192,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const MIXTRAL_8X7B: ModelInfo = ModelInfo {
        id: "mistralai/Mixtral-8x7B-Instruct-v0.1",
        name: "Mixtral 8x7B",
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const FALCON_7B: ModelInfo = ModelInfo {
        id: "tiiuae/falcon-7b-instruct",
        name: "Falcon 7B",
        context_window: 2_048,
        max_output_tokens: 2_048,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const ALL: &[ModelInfo] = &[ZEPHYR_7B, MIXTRAL_8X7B, FALCON_7B];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuggingFaceConfig {
    /// HuggingFace API token.
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default model repository ID (e.g. "meta-llama/Llama-3-8B-Instruct").
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Use dedicated inference endpoint instead of serverless.
    pub endpoint_url: Option<String>,
    /// Wait for model to load if it's cold.
    #[serde(default = "default_wait_for_model")]
    pub wait_for_model: bool,
    pub default_max_tokens: Option<u32>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::ZEPHYR_7B.id.to_string()
}

fn default_wait_for_model() -> bool {
    true
}

fn default_timeout() -> u64 {
    120
}

impl From<HuggingFaceConfig> for AiProviderConfig {
    fn from(config: HuggingFaceConfig) -> Self {
        AiProviderConfig {
            api_key: config.api_key,
            base_url: Some(config.endpoint_url.unwrap_or(config.base_url)),
            default_model: Some(config.default_model),
            default_temperature: None,
            default_max_tokens: config.default_max_tokens,
            timeout_secs: config.timeout_secs,
        }
    }
}

pub struct HuggingFaceProvider {
    config: HuggingFaceConfig,
    status: AiProviderStatus,
}

impl HuggingFaceProvider {
    pub fn new(config: HuggingFaceConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &HuggingFaceConfig {
        &self.config
    }

    pub fn status(&self) -> &AiProviderStatus {
        &self.status
    }

    pub fn info() -> &'static AiProviderInfo {
        &INFO
    }

    pub fn popular_models() -> &'static [ModelInfo] {
        models::ALL
    }

    pub async fn validate(&mut self) -> Result<(), String> {
        self.status = AiProviderStatus::Ready;
        Ok(())
    }
}
