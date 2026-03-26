//! Google (Gemini) AI provider integration.

use serde::{Deserialize, Serialize};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "google",
    name: "Google AI (Gemini)",
    base_url: "https://generativelanguage.googleapis.com/v1beta",
    docs_url: "https://ai.google.dev/api",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: true,
};

pub mod models {
    use super::ModelInfo;

    pub const GEMINI_25_PRO: ModelInfo = ModelInfo {
        id: "gemini-2.5-pro",
        name: "Gemini 2.5 Pro",
        context_window: 1_000_000,
        max_output_tokens: 65_536,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const GEMINI_25_FLASH: ModelInfo = ModelInfo {
        id: "gemini-2.5-flash",
        name: "Gemini 2.5 Flash",
        context_window: 1_000_000,
        max_output_tokens: 65_536,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const GEMINI_20_FLASH: ModelInfo = ModelInfo {
        id: "gemini-2.0-flash",
        name: "Gemini 2.0 Flash",
        context_window: 1_000_000,
        max_output_tokens: 8_192,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const ALL: &[ModelInfo] = &[GEMINI_25_PRO, GEMINI_25_FLASH, GEMINI_20_FLASH];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleAiConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Google Cloud project ID (for Vertex AI).
    pub project_id: Option<String>,
    /// Region for Vertex AI.
    pub region: Option<String>,
    pub default_max_tokens: Option<u32>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::GEMINI_25_FLASH.id.to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<GoogleAiConfig> for AiProviderConfig {
    fn from(config: GoogleAiConfig) -> Self {
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

pub struct GoogleAiProvider {
    config: GoogleAiConfig,
    status: AiProviderStatus,
}

impl GoogleAiProvider {
    pub fn new(config: GoogleAiConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &GoogleAiConfig {
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
