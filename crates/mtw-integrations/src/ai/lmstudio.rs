//! LM Studio (local models) AI provider integration.
//!
//! LM Studio exposes an OpenAI-compatible API on localhost:1234.
//! No API key is needed for local usage. Uses the same SSE streaming
//! format as OpenAI (`data: [DONE]` sentinel).

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::openai::{parse_openai_sse_chunk, StreamDelta};
use super::{AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "lmstudio",
    name: "LM Studio (Local)",
    base_url: "http://localhost:1234/v1",
    docs_url: "https://lmstudio.ai/docs",
    supports_streaming: true,
    supports_tool_calling: false,
    supports_vision: false,
    supports_embeddings: true,
};

/// LM Studio uses whatever models the user has loaded, so these are
/// just placeholder entries. The actual available models depend on
/// what is loaded in the LM Studio UI.
pub mod models {
    use super::ModelInfo;

    pub const DEFAULT: ModelInfo = ModelInfo {
        id: "default",
        name: "Currently Loaded Model",
        context_window: 4_096,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const ALL: &[ModelInfo] = &[DEFAULT];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LmStudioConfig {
    /// LM Studio server base URL.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default model to use.
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Request timeout in seconds (local models can be slow).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::DEFAULT.id.to_string()
}

fn default_timeout() -> u64 {
    300
}

pub struct LmStudioProvider {
    config: LmStudioConfig,
    status: AiProviderStatus,
}

impl LmStudioProvider {
    pub fn new(config: LmStudioConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Unavailable("not connected".to_string()),
        }
    }

    pub fn config(&self) -> &LmStudioConfig {
        &self.config
    }

    pub fn status(&self) -> &AiProviderStatus {
        &self.status
    }

    pub fn info() -> &'static AiProviderInfo {
        &INFO
    }

    pub fn common_models() -> &'static [ModelInfo] {
        models::ALL
    }

    /// Check if the LM Studio server is reachable.
    pub async fn validate(&mut self) -> Result<(), String> {
        let url = format!("{}/models", self.config.base_url);
        let client = reqwest::Client::new();
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                self.status = AiProviderStatus::Ready;
                Ok(())
            }
            Ok(resp) => {
                let msg = format!("LM Studio returned status {}", resp.status());
                self.status = AiProviderStatus::Unavailable(msg.clone());
                Err(msg)
            }
            Err(e) => {
                let msg = format!("Cannot reach LM Studio: {}", e);
                self.status = AiProviderStatus::Unavailable(msg.clone());
                Err(msg)
            }
        }
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

    /// Send a chat completion request (OpenAI-compatible).
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<Value, String> {
        let mut body = json!({
            "model": self.resolve_model(model),
            "messages": messages,
            "stream": false,
        });
        if let Some(temp) = temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(max) = max_tokens {
            body["max_tokens"] = json!(max);
        }

        let url = format!("{}/chat/completions", self.config.base_url);
        let response = self
            .http_client()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LM Studio request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("LM Studio returned {}: {}", status, body));
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| format!("Failed to parse LM Studio response: {}", e))
    }

    /// Stream a chat completion response (OpenAI-compatible SSE).
    pub fn stream_chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> BoxStream<'static, Result<StreamDelta, String>> {
        use futures::StreamExt;

        let mut body = json!({
            "model": self.resolve_model(model),
            "messages": messages,
            "stream": true,
        });
        if let Some(temp) = temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(max) = max_tokens {
            body["max_tokens"] = json!(max);
        }

        let url = format!("{}/chat/completions", self.config.base_url);
        let client = self.http_client();

        let stream = futures::stream::once(async move {
            client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("LM Studio stream request failed: {}", e))
        })
        .filter_map(|result| async {
            match result {
                Ok(resp) if resp.status().is_success() => Some(resp.bytes_stream()),
                Ok(resp) => {
                    tracing::error!("LM Studio stream returned status {}", resp.status());
                    None
                }
                Err(e) => {
                    tracing::error!("LM Studio stream error: {}", e);
                    None
                }
            }
        })
        .flatten()
        .map(parse_openai_sse_chunk)
        .filter_map(|r| async { r });

        Box::pin(stream)
    }

    /// Generate embeddings (OpenAI-compatible).
    pub async fn embed(
        &self,
        model: &str,
        input: Vec<String>,
    ) -> Result<Vec<Vec<f64>>, String> {
        let body = json!({
            "model": self.resolve_model(model),
            "input": input,
        });

        let url = format!("{}/embeddings", self.config.base_url);
        let response: Value = self
            .http_client()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LM Studio embeddings request failed: {}", e))?
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

    /// List models currently loaded in LM Studio.
    pub async fn list_loaded_models(&self) -> Result<Vec<String>, String> {
        let url = format!("{}/models", self.config.base_url);
        let client = reqwest::Client::new();
        let response: serde_json::Value = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Parse failed: {}", e))?;

        let models = response["data"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
            .collect();
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let json = r#"{}"#;
        let config: LmStudioConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "http://localhost:1234/v1");
        assert_eq!(config.default_model, "default");
        assert_eq!(config.timeout_secs, 300);
    }

    #[test]
    fn test_config_custom() {
        let json = r#"{"base_url": "http://192.168.1.50:1234/v1", "default_model": "my-model"}"#;
        let config: LmStudioConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "http://192.168.1.50:1234/v1");
        assert_eq!(config.default_model, "my-model");
    }

    #[test]
    fn test_provider_creation() {
        let provider = LmStudioProvider::new(LmStudioConfig {
            base_url: default_base_url(),
            default_model: default_model(),
            timeout_secs: 300,
        });
        assert_eq!(
            *provider.status(),
            AiProviderStatus::Unavailable("not connected".to_string())
        );
        assert_eq!(LmStudioProvider::info().id, "lmstudio");
    }
}
