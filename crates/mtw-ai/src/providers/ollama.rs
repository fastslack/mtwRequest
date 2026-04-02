use async_trait::async_trait;
use futures::stream::StreamExt;
use futures::Stream;
use mtw_core::MtwError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, FinishReason, MessageRole, ModelInfo, MtwAIProvider,
    ProviderCapabilities, StreamChunk, Usage,
};

// Common local model identifiers
pub const LLAMA3: &str = "llama3";
pub const MISTRAL: &str = "mistral";
pub const CODELLAMA: &str = "codellama";
pub const PHI3: &str = "phi3";

/// Configuration for the Ollama provider (local models)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
}

fn default_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_model() -> String {
    LLAMA3.to_string()
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            default_model: default_model(),
        }
    }
}

impl OllamaConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }
}

// --- Ollama API request/response types ---

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaResponse {
    model: Option<String>,
    message: Option<OllamaResponseMessage>,
    done: Option<bool>,
    total_duration: Option<u64>,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaResponseMessage {
    role: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Option<Vec<OllamaModelInfo>>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelInfo {
    name: Option<String>,
    #[allow(dead_code)]
    size: Option<u64>,
}

fn role_to_string(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "user",
    }
}

/// Ollama AI provider for local models
pub struct OllamaProvider {
    config: OllamaConfig,
    client: Client,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // local models can be slow
            .build()
            .expect("failed to build HTTP client");
        Self { config, client }
    }

    pub fn config(&self) -> &OllamaConfig {
        &self.config
    }
}

#[async_trait]
impl MtwAIProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: false,
            vision: false,
            embeddings: true,
            max_context: 8192,
        }
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError> {
        let model = if req.model.is_empty() {
            self.config.default_model.clone()
        } else {
            req.model.clone()
        };

        let messages: Vec<OllamaMessage> = req
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: role_to_string(&m.role).to_string(),
                content: m.content.clone(),
            })
            .collect();

        let options = if req.temperature.is_some() || req.max_tokens.is_some() {
            Some(OllamaOptions {
                temperature: req.temperature,
                num_predict: req.max_tokens,
            })
        } else {
            None
        };

        let ollama_req = OllamaRequest {
            model: model.clone(),
            messages,
            stream: false,
            options,
        };

        let url = format!("{}/api/chat", self.config.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&ollama_req)
            .send()
            .await
            .map_err(|e| MtwError::Internal(format!("ollama request failed: {}", e)))?;

        let status = resp.status();
        let body: OllamaResponse = resp
            .json()
            .await
            .map_err(|e| MtwError::Internal(format!("ollama response parse failed: {}", e)))?;

        if let Some(err) = body.error {
            return Err(MtwError::Internal(format!(
                "ollama API error ({}): {}",
                status, err
            )));
        }

        let content = body
            .message
            .as_ref()
            .and_then(|m| m.content.clone())
            .unwrap_or_default();

        let prompt_tokens = body.prompt_eval_count.unwrap_or(0);
        let completion_tokens = body.eval_count.unwrap_or(0);

        Ok(CompletionResponse {
            id: ulid::Ulid::new().to_string(),
            model: body.model.unwrap_or(model),
            content,
            tool_calls: vec![],
            usage: Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            },
            finish_reason: FinishReason::Stop,
        })
    }

    fn stream(
        &self,
        req: CompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>> {
        let model = if req.model.is_empty() {
            self.config.default_model.clone()
        } else {
            req.model.clone()
        };

        let messages: Vec<OllamaMessage> = req
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: role_to_string(&m.role).to_string(),
                content: m.content.clone(),
            })
            .collect();

        let options = if req.temperature.is_some() || req.max_tokens.is_some() {
            Some(OllamaOptions {
                temperature: req.temperature,
                num_predict: req.max_tokens,
            })
        } else {
            None
        };

        let ollama_req = OllamaRequest {
            model,
            messages,
            stream: true,
            options,
        };

        let url = format!("{}/api/chat", self.config.base_url);
        let client = self.client.clone();

        Box::pin(async_stream::try_stream! {
            let resp = client
                .post(&url)
                .json(&ollama_req)
                .send()
                .await
                .map_err(|e| MtwError::Internal(format!("ollama stream request failed: {}", e)))?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                Err(MtwError::Internal(format!("ollama stream error ({}): {}", status, body)))?;
                unreachable!();
            }

            let mut stream = resp.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| MtwError::Internal(format!("ollama stream read: {}", e)))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Ollama streams newline-delimited JSON
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<OllamaResponse>(&line) {
                        Ok(parsed) => {
                            let done = parsed.done.unwrap_or(false);
                            let content = parsed
                                .message
                                .as_ref()
                                .and_then(|m| m.content.clone())
                                .unwrap_or_default();

                            let finish_reason = if done {
                                Some(FinishReason::Stop)
                            } else {
                                None
                            };

                            let usage = if done {
                                let prompt = parsed.prompt_eval_count.unwrap_or(0);
                                let completion = parsed.eval_count.unwrap_or(0);
                                Some(Usage {
                                    prompt_tokens: prompt,
                                    completion_tokens: completion,
                                    total_tokens: prompt + completion,
                                })
                            } else {
                                None
                            };

                            yield StreamChunk {
                                delta: content,
                                tool_calls: vec![],
                                finish_reason,
                                usage,
                            };

                            if done {
                                return;
                            }
                        }
                        Err(_) => { /* skip unparseable lines */ }
                    }
                }
            }
        })
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        let url = format!("{}/api/tags", self.config.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| MtwError::Internal(format!("ollama models request failed: {}", e)))?;

        if !resp.status().is_success() {
            // Fallback to static list if Ollama is not running
            return Ok(self.static_models());
        }

        let body: OllamaTagsResponse = resp
            .json()
            .await
            .map_err(|e| MtwError::Internal(format!("ollama models parse failed: {}", e)))?;

        let models = body
            .models
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                let name = m.name.unwrap_or_else(|| "unknown".to_string());
                ModelInfo {
                    id: name.clone(),
                    name: name.clone(),
                    max_context: 8192,
                    supports_tools: false,
                    supports_vision: false,
                }
            })
            .collect();

        Ok(models)
    }
}

impl OllamaProvider {
    fn static_models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: LLAMA3.to_string(),
                name: "Llama 3".to_string(),
                max_context: 8192,
                supports_tools: false,
                supports_vision: false,
            },
            ModelInfo {
                id: MISTRAL.to_string(),
                name: "Mistral".to_string(),
                max_context: 8192,
                supports_tools: false,
                supports_vision: false,
            },
            ModelInfo {
                id: CODELLAMA.to_string(),
                name: "Code Llama".to_string(),
                max_context: 16384,
                supports_tools: false,
                supports_vision: false,
            },
            ModelInfo {
                id: PHI3.to_string(),
                name: "Phi-3".to_string(),
                max_context: 4096,
                supports_tools: false,
                supports_vision: false,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = OllamaConfig::default();
        assert_eq!(config.base_url, "http://localhost:11434");
        assert_eq!(config.default_model, LLAMA3);
    }

    #[test]
    fn test_config_builder() {
        let config = OllamaConfig::new()
            .with_base_url("http://gpu-server:11434")
            .with_model(MISTRAL);
        assert_eq!(config.base_url, "http://gpu-server:11434");
        assert_eq!(config.default_model, MISTRAL);
    }

    #[test]
    fn test_provider_name() {
        let provider = OllamaProvider::new(OllamaConfig::default());
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_capabilities() {
        let provider = OllamaProvider::new(OllamaConfig::default());
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(!caps.tool_calling);
        assert!(!caps.vision);
        assert!(caps.embeddings);
    }

    #[test]
    fn test_static_models() {
        let provider = OllamaProvider::new(OllamaConfig::default());
        let models = provider.static_models();
        assert_eq!(models.len(), 4);
        assert!(models.iter().any(|m| m.id == LLAMA3));
    }
}
