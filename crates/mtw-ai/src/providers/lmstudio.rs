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

/// Configuration for the LM Studio provider (local, OpenAI-compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LMStudioConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Optional API key (LM Studio accepts Bearer tokens for compatibility)
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_base_url() -> String {
    "http://localhost:1234/v1".to_string()
}

fn default_model() -> String {
    "default".to_string()
}

impl Default for LMStudioConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            default_model: default_model(),
            api_key: None,
        }
    }
}

impl LMStudioConfig {
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

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
}

// --- OpenAI-compatible request/response types (shared format with LM Studio) ---

#[derive(Debug, Serialize)]
struct LmsRequest {
    model: String,
    messages: Vec<LmsMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct LmsMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct LmsResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Option<Vec<LmsChoice>>,
    usage: Option<LmsUsage>,
    error: Option<LmsError>,
}

#[derive(Debug, Deserialize)]
struct LmsChoice {
    message: Option<LmsResponseMessage>,
    delta: Option<LmsDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LmsResponseMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LmsDelta {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LmsUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LmsError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct LmsModelsResponse {
    data: Option<Vec<LmsModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct LmsModelEntry {
    id: Option<String>,
}

fn role_to_string(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "user",
    }
}

fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        _ => FinishReason::Stop,
    }
}

/// LM Studio AI provider for local models (OpenAI-compatible API)
pub struct LMStudioProvider {
    config: LMStudioConfig,
    client: Client,
}

impl LMStudioProvider {
    pub fn new(config: LMStudioConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("failed to build HTTP client");
        Self { config, client }
    }

    pub fn config(&self) -> &LMStudioConfig {
        &self.config
    }

    fn auth_header(&self) -> Option<String> {
        self.config
            .api_key
            .as_ref()
            .map(|k| format!("Bearer {}", k))
    }
}

#[async_trait]
impl MtwAIProvider for LMStudioProvider {
    fn name(&self) -> &str {
        "lmstudio"
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

        let messages: Vec<LmsMessage> = req
            .messages
            .iter()
            .map(|m| LmsMessage {
                role: role_to_string(&m.role).to_string(),
                content: m.content.clone(),
            })
            .collect();

        let lms_req = LmsRequest {
            model: model.clone(),
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            stream: None,
        };

        let url = format!("{}/chat/completions", self.config.base_url);
        let mut http_req = self.client.post(&url).json(&lms_req);
        if let Some(auth) = self.auth_header() {
            http_req = http_req.header("Authorization", auth);
        }

        let resp = http_req
            .send()
            .await
            .map_err(|e| MtwError::Internal(format!("lmstudio request failed: {}", e)))?;

        let status = resp.status();
        let body: LmsResponse = resp
            .json()
            .await
            .map_err(|e| {
                MtwError::Internal(format!("lmstudio response parse failed: {}", e))
            })?;

        if let Some(err) = body.error {
            return Err(MtwError::Internal(format!(
                "lmstudio API error ({}): {}",
                status, err.message
            )));
        }

        let choice = body
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .ok_or_else(|| MtwError::Internal("lmstudio: no choices in response".into()))?;

        let content = choice
            .message
            .as_ref()
            .and_then(|m| m.content.clone())
            .unwrap_or_default();

        let usage = body.usage.as_ref().map_or(Usage::default(), |u| Usage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
        });

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FinishReason::Stop);

        Ok(CompletionResponse {
            id: body.id.unwrap_or_else(|| ulid::Ulid::new().to_string()),
            model: body.model.unwrap_or(model),
            content,
            tool_calls: vec![],
            usage,
            finish_reason,
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

        let messages: Vec<LmsMessage> = req
            .messages
            .iter()
            .map(|m| LmsMessage {
                role: role_to_string(&m.role).to_string(),
                content: m.content.clone(),
            })
            .collect();

        let lms_req = LmsRequest {
            model,
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            stream: Some(true),
        };

        let url = format!("{}/chat/completions", self.config.base_url);
        let client = self.client.clone();
        let auth = self.auth_header();

        Box::pin(async_stream::try_stream! {
            let mut http_req = client.post(&url).json(&lms_req);
            if let Some(auth) = auth {
                http_req = http_req.header("Authorization", auth);
            }

            let resp = http_req
                .send()
                .await
                .map_err(|e| MtwError::Internal(format!("lmstudio stream request failed: {}", e)))?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                Err(MtwError::Internal(format!("lmstudio stream error ({}): {}", status, body)))?;
                unreachable!();
            }

            let mut stream = resp.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| MtwError::Internal(format!("lmstudio stream read: {}", e)))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            return;
                        }

                        match serde_json::from_str::<LmsResponse>(data) {
                            Ok(parsed) => {
                                if let Some(choices) = &parsed.choices {
                                    if let Some(choice) = choices.first() {
                                        let delta_content = choice
                                            .delta
                                            .as_ref()
                                            .and_then(|d| d.content.clone())
                                            .unwrap_or_default();
                                        let finish_reason = choice
                                            .finish_reason
                                            .as_deref()
                                            .map(parse_finish_reason);
                                        let usage = parsed.usage.as_ref().map(|u| Usage {
                                            prompt_tokens: u.prompt_tokens.unwrap_or(0),
                                            completion_tokens: u.completion_tokens.unwrap_or(0),
                                            total_tokens: u.total_tokens.unwrap_or(0),
                                        });

                                        yield StreamChunk {
                                            delta: delta_content,
                                            tool_calls: vec![],
                                            finish_reason,
                                            usage,
                                        };
                                    }
                                }
                            }
                            Err(_) => { /* skip unparseable lines */ }
                        }
                    }
                }
            }
        })
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        let url = format!("{}/models", self.config.base_url);
        let mut http_req = self.client.get(&url);
        if let Some(auth) = self.auth_header() {
            http_req = http_req.header("Authorization", auth);
        }

        let resp = http_req
            .send()
            .await
            .map_err(|e| MtwError::Internal(format!("lmstudio models request failed: {}", e)))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let body: LmsModelsResponse = resp
            .json()
            .await
            .map_err(|e| MtwError::Internal(format!("lmstudio models parse failed: {}", e)))?;

        let models = body
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                let id = m.id.unwrap_or_else(|| "unknown".to_string());
                ModelInfo {
                    name: id.clone(),
                    id,
                    max_context: 8192,
                    supports_tools: false,
                    supports_vision: false,
                }
            })
            .collect();

        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LMStudioConfig::default();
        assert_eq!(config.base_url, "http://localhost:1234/v1");
        assert_eq!(config.default_model, "default");
        assert!(config.api_key.is_none());
    }

    #[test]
    fn test_config_builder() {
        let config = LMStudioConfig::new()
            .with_base_url("http://gpu-server:1234/v1")
            .with_model("my-model")
            .with_api_key("test-key");
        assert_eq!(config.base_url, "http://gpu-server:1234/v1");
        assert_eq!(config.default_model, "my-model");
        assert_eq!(config.api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_provider_name() {
        let provider = LMStudioProvider::new(LMStudioConfig::default());
        assert_eq!(provider.name(), "lmstudio");
    }

    #[test]
    fn test_capabilities() {
        let provider = LMStudioProvider::new(LMStudioConfig::default());
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(!caps.tool_calling);
        assert!(!caps.vision);
        assert!(caps.embeddings);
        assert_eq!(caps.max_context, 8192);
    }

    #[test]
    fn test_auth_header_none() {
        let provider = LMStudioProvider::new(LMStudioConfig::default());
        assert!(provider.auth_header().is_none());
    }

    #[test]
    fn test_auth_header_with_key() {
        let config = LMStudioConfig::new().with_api_key("my-key");
        let provider = LMStudioProvider::new(config);
        assert_eq!(provider.auth_header(), Some("Bearer my-key".to_string()));
    }
}
