//! AI model provider integrations.
//!
//! Each sub-module provides a config struct, supported model constants, and
//! provider information for a specific AI model provider.

pub mod anthropic;
pub mod cohere;
pub mod deepseek;
pub mod google;
pub mod huggingface;
pub mod lmstudio;
pub mod meta;
pub mod mistral;
pub mod ollama;
pub mod openai;
pub mod xai;

use serde::{Deserialize, Serialize};

/// Information about an AI model provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderInfo {
    /// Machine-readable identifier (e.g. "anthropic").
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
    /// Default API base URL.
    pub base_url: &'static str,
    /// Documentation URL.
    pub docs_url: &'static str,
    /// Whether the provider supports streaming.
    pub supports_streaming: bool,
    /// Whether the provider supports tool/function calling.
    pub supports_tool_calling: bool,
    /// Whether the provider supports vision/image inputs.
    pub supports_vision: bool,
    /// Whether the provider supports embeddings.
    pub supports_embeddings: bool,
}

/// Information about a specific AI model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier (e.g. "claude-sonnet-4-20250514").
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
    /// Maximum context window in tokens.
    pub context_window: usize,
    /// Maximum output tokens.
    pub max_output_tokens: usize,
    /// Whether this model supports vision.
    pub supports_vision: bool,
    /// Whether this model supports tool calling.
    pub supports_tool_calling: bool,
}

/// Common configuration shared by all AI providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    /// API key for authentication.
    pub api_key: String,
    /// API base URL (override the default).
    pub base_url: Option<String>,
    /// Default model to use.
    pub default_model: Option<String>,
    /// Default temperature (0.0 - 2.0).
    pub default_temperature: Option<f32>,
    /// Default max tokens for responses.
    pub default_max_tokens: Option<u32>,
    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    120
}

/// Status of an AI provider connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderStatus {
    Ready,
    Unavailable(String),
}
