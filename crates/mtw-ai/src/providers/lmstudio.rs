use async_trait::async_trait;
use futures::stream::StreamExt;
use futures::Stream;
use mtw_core::MtwError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, FinishReason, ModelInfo, MtwAIProvider,
    ProviderCapabilities, StreamChunk, Usage,
};
use super::openai::{
    build_oai_request, OaiResponse, OaiUsage,
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

// LMStudio-specific response types (models endpoint only)
#[derive(Debug, Deserialize)]
struct LmsModelsResponse {
    data: Option<Vec<LmsModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct LmsModelEntry {
    id: Option<String>,
}


fn oai_usage_to_usage(u: Option<&OaiUsage>) -> Usage {
    u.map_or(Usage::default(), |u| Usage {
        prompt_tokens: u.prompt_tokens.unwrap_or(0),
        completion_tokens: u.completion_tokens.unwrap_or(0),
        total_tokens: u.total_tokens.unwrap_or(0),
    })
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

        let mut oai_req = build_oai_request(&req, false);
        oai_req.model = model.clone();
        // LMStudio doesn't support tool_calls
        oai_req.tools = None;

        let url = format!("{}/chat/completions", self.config.base_url);
        let mut http_req = self.client.post(&url).json(&oai_req);
        if let Some(auth) = self.auth_header() {
            http_req = http_req.header("Authorization", auth);
        }

        let resp = http_req
            .send()
            .await
            .map_err(|e| MtwError::Internal(format!("lmstudio request failed: {}", e)))?;

        let status = resp.status();
        let body: OaiResponse = resp
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

        let usage = oai_usage_to_usage(body.usage.as_ref());

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(FinishReason::from_openai)
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
        let mut oai_req = build_oai_request(&req, true);
        if req.model.is_empty() {
            oai_req.model = self.config.default_model.clone();
        }
        oai_req.tools = None;

        let url = format!("{}/chat/completions", self.config.base_url);
        let client = self.client.clone();
        let auth = self.auth_header();

        Box::pin(async_stream::try_stream! {
            let mut http_req = client.post(&url).json(&oai_req);
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

            let mut inner = super::sse::parse_oai_sse_stream(resp.bytes_stream(), "lmstudio", false);
            while let Some(chunk) = StreamExt::next(&mut inner).await {
                yield chunk?;
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
