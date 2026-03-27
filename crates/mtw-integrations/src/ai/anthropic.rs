//! Anthropic (Claude) AI provider integration.
//!
//! Includes real HTTP chat, streaming (SSE with event-type routing for
//! `content_block_delta` and `message_delta`), and embeddings via Voyage.

use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{AiProviderConfig, AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "anthropic",
    name: "Anthropic",
    base_url: "https://api.anthropic.com/v1",
    docs_url: "https://docs.anthropic.com/en/api",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: false,
};

/// Supported Anthropic models.
pub mod models {
    use super::ModelInfo;

    pub const CLAUDE_OPUS_4: ModelInfo = ModelInfo {
        id: "claude-opus-4-20250514",
        name: "Claude Opus 4",
        context_window: 200_000,
        max_output_tokens: 32_000,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const CLAUDE_SONNET_4: ModelInfo = ModelInfo {
        id: "claude-sonnet-4-20250514",
        name: "Claude Sonnet 4",
        context_window: 200_000,
        max_output_tokens: 16_000,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const CLAUDE_HAIKU_35: ModelInfo = ModelInfo {
        id: "claude-3-5-haiku-20241022",
        name: "Claude 3.5 Haiku",
        context_window: 200_000,
        max_output_tokens: 8_192,
        supports_vision: true,
        supports_tool_calling: true,
    };

    pub const ALL: &[ModelInfo] = &[CLAUDE_OPUS_4, CLAUDE_SONNET_4, CLAUDE_HAIKU_35];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    /// API key (starts with "sk-ant-").
    pub api_key: String,
    /// API base URL.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default model.
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Anthropic API version header.
    #[serde(default = "default_api_version")]
    pub api_version: String,
    /// Default max tokens.
    pub default_max_tokens: Option<u32>,
    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::CLAUDE_SONNET_4.id.to_string()
}

fn default_api_version() -> String {
    "2023-06-01".to_string()
}

fn default_timeout() -> u64 {
    120
}

impl From<AnthropicConfig> for AiProviderConfig {
    fn from(config: AnthropicConfig) -> Self {
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

pub struct AnthropicProvider {
    config: AnthropicConfig,
    status: AiProviderStatus,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Ready,
        }
    }

    pub fn config(&self) -> &AnthropicConfig {
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

    /// Build the Anthropic messages body.
    /// Anthropic uses a separate `system` field instead of a system message in the array.
    fn build_messages_body(
        &self,
        messages: &[Value],
        model: &str,
        stream: bool,
        max_tokens: Option<u32>,
    ) -> Value {
        let mut system_prompt = String::new();
        let filtered_messages: Vec<Value> = messages
            .iter()
            .filter_map(|m| {
                if m["role"].as_str() == Some("system") {
                    if let Some(content) = m["content"].as_str() {
                        system_prompt = content.to_string();
                    }
                    None
                } else {
                    Some(m.clone())
                }
            })
            .collect();

        let max_tokens = max_tokens
            .or(self.config.default_max_tokens)
            .unwrap_or(4096);

        let mut body = json!({
            "model": self.resolve_model(model),
            "messages": filtered_messages,
            "max_tokens": max_tokens,
            "stream": stream,
        });

        if !system_prompt.is_empty() {
            body["system"] = json!(system_prompt);
        }

        body
    }

    /// Send a chat message request.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<Value, String> {
        let mut body = self.build_messages_body(&messages, model, false, max_tokens);
        if let Some(temp) = temperature {
            body["temperature"] = json!(temp);
        }

        let url = format!("{}/messages", self.config.base_url);
        let response = self
            .http_client()
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Anthropic request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Anthropic returned {}: {}", status, body));
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| format!("Failed to parse Anthropic response: {}", e))
    }

    /// Stream a chat response. Parses Anthropic SSE events:
    /// - `content_block_delta` for text chunks
    /// - `message_delta` for stop reason
    pub fn stream_chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> BoxStream<'static, Result<AnthropicStreamDelta, String>> {
        let mut body = self.build_messages_body(&messages, model, true, max_tokens);
        if let Some(temp) = temperature {
            body["temperature"] = json!(temp);
        }

        let url = format!("{}/messages", self.config.base_url);
        let client = self.http_client();
        let api_key = self.config.api_key.clone();
        let api_version = self.config.api_version.clone();

        let stream = futures::stream::once(async move {
            client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", &api_version)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Anthropic stream request failed: {}", e))
        })
        .filter_map(|result| async {
            match result {
                Ok(resp) if resp.status().is_success() => Some(resp.bytes_stream()),
                Ok(resp) => {
                    tracing::error!("Anthropic stream returned status {}", resp.status());
                    None
                }
                Err(e) => {
                    tracing::error!("Anthropic stream error: {}", e);
                    None
                }
            }
        })
        .flatten()
        .map(parse_anthropic_sse_chunk)
        .filter_map(|r| async { r });

        Box::pin(stream)
    }

    /// Generate embeddings via Voyage API (Anthropic's embedding partner).
    pub async fn embed(
        &self,
        model: &str,
        input: Vec<String>,
    ) -> Result<Vec<Vec<f64>>, String> {
        let model = if model.is_empty() {
            "voyage-3".to_string()
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
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Anthropic embeddings request failed: {}", e))?
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
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .send()
            .await
            .map_err(|e| format!("Anthropic list models failed: {}", e))?
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

/// A parsed streaming delta from an Anthropic SSE response.
#[derive(Debug, Clone)]
pub struct AnthropicStreamDelta {
    /// Text content delta.
    pub delta: String,
    /// Stop reason, if the stream is complete.
    pub stop_reason: Option<String>,
}

/// Parse Anthropic SSE chunks. Anthropic uses event-type routing:
/// - `content_block_delta` → text delta in `delta.text`
/// - `message_delta` → stop reason in `delta.stop_reason`
fn parse_anthropic_sse_chunk(
    result: Result<Bytes, reqwest::Error>,
) -> Option<Result<AnthropicStreamDelta, String>> {
    let bytes = match result {
        Ok(b) => b,
        Err(e) => return Some(Err(format!("Stream read error: {}", e))),
    };
    let text = String::from_utf8_lossy(&bytes);

    let mut delta = String::new();
    let mut stop_reason = None;

    for line in text.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                let event_type = parsed["type"].as_str().unwrap_or("");
                match event_type {
                    "content_block_delta" => {
                        if let Some(t) = parsed["delta"]["text"].as_str() {
                            delta.push_str(t);
                        }
                    }
                    "message_delta" => {
                        if let Some(reason) = parsed["delta"]["stop_reason"].as_str() {
                            stop_reason = Some(reason.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if delta.is_empty() && stop_reason.is_none() {
        return None;
    }

    Some(Ok(AnthropicStreamDelta { delta, stop_reason }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let json = r#"{"api_key": "sk-ant-test"}"#;
        let config: AnthropicConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "https://api.anthropic.com/v1");
        assert_eq!(config.default_model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_models() {
        assert_eq!(models::ALL.len(), 3);
        assert!(models::CLAUDE_OPUS_4.context_window >= 200_000);
    }

    #[test]
    fn test_provider_creation() {
        let provider = AnthropicProvider::new(AnthropicConfig {
            api_key: "sk-ant-test".to_string(),
            base_url: default_base_url(),
            default_model: default_model(),
            api_version: default_api_version(),
            default_max_tokens: Some(4096),
            timeout_secs: 120,
        });
        assert_eq!(*provider.status(), AiProviderStatus::Ready);
    }
}
