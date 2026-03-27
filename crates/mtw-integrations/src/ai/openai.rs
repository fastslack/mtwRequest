//! OpenAI (GPT) AI provider integration.
//!
//! Includes real HTTP chat, streaming (SSE with `data: [DONE]` sentinel),
//! and embeddings support.

use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

    fn http_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.config.timeout_secs))
            .build()
            .unwrap_or_default()
    }

    fn resolve_model(&self, model: &str) -> String {
        if model.is_empty() {
            self.config.default_model.clone()
        } else {
            model.to_string()
        }
    }

    fn build_chat_body(&self, messages: &[Value], model: &str, stream: bool) -> Value {
        json!({
            "model": self.resolve_model(model),
            "messages": messages,
            "stream": stream,
        })
    }

    /// Send a chat completion request.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<Value, String> {
        let mut body = self.build_chat_body(&messages, model, false);
        if let Some(temp) = temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(max) = max_tokens.or(self.config.default_max_tokens) {
            body["max_tokens"] = json!(max);
        }

        let url = format!("{}/chat/completions", self.config.base_url);
        let response = self
            .http_client()
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("OpenAI request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("OpenAI returned {}: {}", status, body));
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| format!("Failed to parse OpenAI response: {}", e))
    }

    /// Stream a chat completion response. Returns SSE chunks parsed into JSON deltas.
    pub fn stream_chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> BoxStream<'static, Result<StreamDelta, String>> {
        let mut body = self.build_chat_body(&messages, model, true);
        if let Some(temp) = temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(max) = max_tokens.or(self.config.default_max_tokens) {
            body["max_tokens"] = json!(max);
        }

        let url = format!("{}/chat/completions", self.config.base_url);
        let client = self.http_client();
        let api_key = self.config.api_key.clone();

        let stream = futures::stream::once(async move {
            client
                .post(&url)
                .bearer_auth(&api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("OpenAI stream request failed: {}", e))
        })
        .filter_map(|result| async {
            match result {
                Ok(resp) if resp.status().is_success() => Some(resp.bytes_stream()),
                Ok(resp) => {
                    tracing::error!("OpenAI stream returned status {}", resp.status());
                    None
                }
                Err(e) => {
                    tracing::error!("OpenAI stream error: {}", e);
                    None
                }
            }
        })
        .flatten()
        .map(parse_openai_sse_chunk)
        .filter_map(|r| async { r });

        Box::pin(stream)
    }

    /// Generate embeddings.
    pub async fn embed(
        &self,
        model: &str,
        input: Vec<String>,
    ) -> Result<Vec<Vec<f64>>, String> {
        let model = if model.is_empty() {
            "text-embedding-3-small".to_string()
        } else {
            model.to_string()
        };

        let body = json!({
            "model": model,
            "input": input,
        });

        let url = format!("{}/embeddings", self.config.base_url);
        let response: Value = self
            .http_client()
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("OpenAI embeddings request failed: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse embeddings response: {}", e))?;

        let embeddings = response["data"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|item| {
                item["embedding"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
            })
            .collect();

        Ok(embeddings)
    }

    /// List available models.
    pub async fn list_models(&self) -> Result<Vec<String>, String> {
        let url = format!("{}/models", self.config.base_url);
        let response: Value = self
            .http_client()
            .get(&url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|e| format!("OpenAI list models failed: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse models response: {}", e))?;

        let models = response["data"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
            .collect();
        Ok(models)
    }
}

/// A parsed streaming delta from an OpenAI-compatible SSE response.
#[derive(Debug, Clone)]
pub struct StreamDelta {
    /// Text content delta.
    pub delta: String,
    /// Finish reason, if the stream is complete.
    pub finish_reason: Option<String>,
}

/// Parse a single SSE byte chunk from an OpenAI-compatible stream.
/// Public so that OpenAI-compatible providers (e.g. LM Studio) can reuse this.
pub fn parse_openai_sse_chunk(result: Result<Bytes, reqwest::Error>) -> Option<Result<StreamDelta, String>> {
    let bytes = match result {
        Ok(b) => b,
        Err(e) => return Some(Err(format!("Stream read error: {}", e))),
    };
    let text = String::from_utf8_lossy(&bytes);

    let mut delta = String::new();
    let mut finish_reason = None;

    for line in text.lines() {
        let line = line.trim();
        if line == "data: [DONE]" {
            finish_reason = Some("stop".to_string());
            break;
        }
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                if let Some(d) = parsed["choices"][0]["delta"]["content"].as_str() {
                    delta.push_str(d);
                }
                if let Some(reason) = parsed["choices"][0]["finish_reason"].as_str() {
                    finish_reason = Some(reason.to_string());
                }
            }
        }
    }

    // Skip empty keep-alive chunks
    if delta.is_empty() && finish_reason.is_none() {
        return None;
    }

    Some(Ok(StreamDelta {
        delta,
        finish_reason,
    }))
}
