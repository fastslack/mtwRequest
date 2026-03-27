//! Ollama (local models) AI provider integration.
//!
//! Includes real HTTP chat, streaming (newline-delimited JSON, not SSE),
//! embeddings, and model management.

use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{AiProviderInfo, AiProviderStatus, ModelInfo};

pub const INFO: AiProviderInfo = AiProviderInfo {
    id: "ollama",
    name: "Ollama (Local)",
    base_url: "http://localhost:11434/api",
    docs_url: "https://github.com/ollama/ollama/blob/main/docs/api.md",
    supports_streaming: true,
    supports_tool_calling: true,
    supports_vision: true,
    supports_embeddings: true,
};

/// Common models available through Ollama.
/// Unlike cloud providers, the actual available models depend on what the
/// user has pulled locally.
pub mod models {
    use super::ModelInfo;

    pub const LLAMA3: ModelInfo = ModelInfo {
        id: "llama3",
        name: "Llama 3 (8B)",
        context_window: 8_192,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const LLAMA3_70B: ModelInfo = ModelInfo {
        id: "llama3:70b",
        name: "Llama 3 (70B)",
        context_window: 8_192,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const MISTRAL: ModelInfo = ModelInfo {
        id: "mistral",
        name: "Mistral (7B)",
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: true,
    };

    pub const LLAVA: ModelInfo = ModelInfo {
        id: "llava",
        name: "LLaVA (Vision)",
        context_window: 4_096,
        max_output_tokens: 4_096,
        supports_vision: true,
        supports_tool_calling: false,
    };

    pub const CODELLAMA: ModelInfo = ModelInfo {
        id: "codellama",
        name: "Code Llama",
        context_window: 16_384,
        max_output_tokens: 4_096,
        supports_vision: false,
        supports_tool_calling: false,
    };

    pub const ALL: &[ModelInfo] = &[LLAMA3, LLAMA3_70B, MISTRAL, LLAVA, CODELLAMA];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    /// Ollama server base URL.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default model to use.
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Request timeout in seconds (local models can be slow).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Keep model loaded in memory after request.
    #[serde(default = "default_keep_alive")]
    pub keep_alive: bool,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

fn default_model() -> String {
    models::LLAMA3.id.to_string()
}

fn default_timeout() -> u64 {
    300 // local models may need longer
}

fn default_keep_alive() -> bool {
    true
}

pub struct OllamaProvider {
    config: OllamaConfig,
    status: AiProviderStatus,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Self {
        Self {
            config,
            status: AiProviderStatus::Unavailable("not connected".to_string()),
        }
    }

    pub fn config(&self) -> &OllamaConfig {
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

    /// Check if the Ollama server is reachable by listing local models.
    pub async fn validate(&mut self) -> Result<(), String> {
        let url = format!("{}/tags", self.config.base_url);
        let client = reqwest::Client::new();
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                self.status = AiProviderStatus::Ready;
                Ok(())
            }
            Ok(resp) => {
                let msg = format!("Ollama returned status {}", resp.status());
                self.status = AiProviderStatus::Unavailable(msg.clone());
                Err(msg)
            }
            Err(e) => {
                let msg = format!("Cannot reach Ollama: {}", e);
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

    /// Send a chat request (non-streaming).
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
    ) -> Result<Value, String> {
        let mut body = json!({
            "model": self.resolve_model(model),
            "messages": messages,
            "stream": false,
        });

        if let Some(temp) = temperature {
            body["options"] = json!({ "temperature": temp });
        }

        let url = format!("{}/chat", self.config.base_url);
        let response = self
            .http_client()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Ollama returned {}: {}", status, body));
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| format!("Failed to parse Ollama response: {}", e))
    }

    /// Stream a chat response. Ollama uses newline-delimited JSON (not SSE).
    pub fn stream_chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        temperature: Option<f32>,
    ) -> BoxStream<'static, Result<OllamaStreamDelta, String>> {
        let mut body = json!({
            "model": self.resolve_model(model),
            "messages": messages,
            "stream": true,
        });

        if let Some(temp) = temperature {
            body["options"] = json!({ "temperature": temp });
        }

        let url = format!("{}/chat", self.config.base_url);
        let client = self.http_client();

        let stream = futures::stream::once(async move {
            client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Ollama stream request failed: {}", e))
        })
        .filter_map(|result| async {
            match result {
                Ok(resp) if resp.status().is_success() => Some(resp.bytes_stream()),
                Ok(resp) => {
                    tracing::error!("Ollama stream returned status {}", resp.status());
                    None
                }
                Err(e) => {
                    tracing::error!("Ollama stream error: {}", e);
                    None
                }
            }
        })
        .flatten()
        .map(parse_ollama_ndjson_chunk)
        .filter_map(|r| async { r });

        Box::pin(stream)
    }

    /// Generate embeddings (one input at a time via /api/embed).
    pub async fn embed(
        &self,
        model: &str,
        input: Vec<String>,
    ) -> Result<Vec<Vec<f64>>, String> {
        let mut all_embeddings = Vec::new();

        for text in &input {
            let body = json!({
                "model": self.resolve_model(model),
                "input": text,
            });

            let url = format!("{}/embed", self.config.base_url);
            let response: Value = self
                .http_client()
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Ollama embed request failed: {}", e))?
                .json()
                .await
                .map_err(|e| format!("Failed to parse embed response: {}", e))?;

            if let Some(embeddings) = response["embeddings"].as_array() {
                for emb in embeddings {
                    if let Some(arr) = emb.as_array() {
                        let vec: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                        all_embeddings.push(vec);
                    }
                }
            }
        }

        Ok(all_embeddings)
    }

    /// List locally available models.
    pub async fn list_local_models(&self) -> Result<Vec<String>, String> {
        let url = format!("{}/tags", self.config.base_url);
        let response: Value = self
            .http_client()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Ollama list models failed: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse models response: {}", e))?;

        let models = response["models"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
            .collect();
        Ok(models)
    }

    /// Pull a model from the Ollama registry.
    pub async fn pull_model(&self, model: &str) -> Result<(), String> {
        let body = json!({ "name": model });
        let url = format!("{}/pull", self.config.base_url);

        let response = self
            .http_client()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama pull request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Ollama pull returned {}: {}", status, body));
        }

        Ok(())
    }
}

/// A parsed streaming delta from an Ollama newline-delimited JSON response.
#[derive(Debug, Clone)]
pub struct OllamaStreamDelta {
    /// Text content delta.
    pub delta: String,
    /// Whether the stream is done.
    pub done: bool,
}

/// Parse Ollama newline-delimited JSON chunks.
/// Unlike OpenAI/Anthropic SSE, Ollama sends plain JSON objects separated by newlines.
fn parse_ollama_ndjson_chunk(
    result: Result<Bytes, reqwest::Error>,
) -> Option<Result<OllamaStreamDelta, String>> {
    let bytes = match result {
        Ok(b) => b,
        Err(e) => return Some(Err(format!("Stream read error: {}", e))),
    };
    let text = String::from_utf8_lossy(&bytes);

    let mut delta = String::new();
    let mut done = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(parsed) = serde_json::from_str::<Value>(line) {
            if let Some(content) = parsed["message"]["content"].as_str() {
                delta.push_str(content);
            }
            if parsed["done"].as_bool().unwrap_or(false) {
                done = true;
            }
        }
    }

    if delta.is_empty() && !done {
        return None;
    }

    Some(Ok(OllamaStreamDelta { delta, done }))
}
