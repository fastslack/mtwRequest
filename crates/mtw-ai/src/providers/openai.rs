use async_trait::async_trait;
use futures::stream::StreamExt;
use futures::Stream;
use mtw_core::MtwError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, FinishReason, ModelInfo,
    MtwAIProvider, ProviderCapabilities, StreamChunk, ToolCall, Usage,
};

// OpenAI model constants
pub const GPT_4O: &str = "gpt-4o";
pub const GPT_4O_MINI: &str = "gpt-4o-mini";
pub const GPT_4_TURBO: &str = "gpt-4-turbo";
pub const O1: &str = "o1";
pub const O1_MINI: &str = "o1-mini";

/// Configuration for the OpenAI provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_model() -> String {
    GPT_4O.to_string()
}

impl OpenAIConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: default_base_url(),
            default_model: default_model(),
        }
    }
}

// --- OpenAI API request/response types (shared with LMStudio) ---

#[derive(Debug, Serialize)]
pub(crate) struct OaiRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<OaiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<OaiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OaiMessage {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct OaiTool {
    pub(crate) r#type: String,
    pub(crate) function: OaiFunction,
}

#[derive(Debug, Serialize)]
pub(crate) struct OaiFunction {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiResponse {
    pub(crate) id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) choices: Option<Vec<OaiChoice>>,
    pub(crate) usage: Option<OaiUsage>,
    pub(crate) error: Option<OaiError>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiChoice {
    pub(crate) message: Option<OaiResponseMessage>,
    pub(crate) delta: Option<OaiDelta>,
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiResponseMessage {
    pub(crate) content: Option<String>,
    pub(crate) tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiDelta {
    pub(crate) content: Option<String>,
    pub(crate) tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiToolCall {
    pub(crate) id: Option<String>,
    pub(crate) function: Option<OaiToolCallFunction>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiToolCallFunction {
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiUsage {
    pub(crate) prompt_tokens: Option<u32>,
    pub(crate) completion_tokens: Option<u32>,
    pub(crate) total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OaiError {
    pub(crate) message: String,
}

pub(crate) fn build_oai_request(req: &CompletionRequest, stream: bool) -> OaiRequest {
    let messages = req
        .messages
        .iter()
        .map(|m| OaiMessage {
            role: m.role.as_openai_str().to_string(),
            content: m.content.clone(),
        })
        .collect();

    let tools = req.tools.as_ref().map(|ts| {
        ts.iter()
            .map(|t| OaiTool {
                r#type: "function".to_string(),
                function: OaiFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect()
    });

    OaiRequest {
        model: req.model.clone(),
        messages,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        tools,
        stream: if stream { Some(true) } else { None },
    }
}

pub(crate) fn parse_tool_calls(oai_calls: &[OaiToolCall]) -> Vec<ToolCall> {
    oai_calls
        .iter()
        .filter_map(|tc| {
            let id = tc.id.clone().unwrap_or_default();
            let func = tc.function.as_ref()?;
            let name = func.name.clone().unwrap_or_default();
            let args_str = func.arguments.clone().unwrap_or_else(|| "{}".to_string());
            let arguments = serde_json::from_str(&args_str).unwrap_or(serde_json::json!({}));
            Some(ToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

/// OpenAI AI provider (GPT models)
pub struct OpenAIProvider {
    config: OpenAIConfig,
    client: Client,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");
        Self { config, client }
    }

    pub fn config(&self) -> &OpenAIConfig {
        &self.config
    }
}

#[async_trait]
impl MtwAIProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            vision: true,
            embeddings: true,
            max_context: 128_000,
        }
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError> {
        let model = if req.model.is_empty() {
            self.config.default_model.clone()
        } else {
            req.model.clone()
        };

        let mut oai_req = build_oai_request(&req, false);
        oai_req.model = model;

        let url = format!("{}/chat/completions", self.config.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&oai_req)
            .send()
            .await
            .map_err(|e| MtwError::Internal(format!("openai request failed: {}", e)))?;

        let status = resp.status();
        let body: OaiResponse = resp
            .json()
            .await
            .map_err(|e| MtwError::Internal(format!("openai response parse failed: {}", e)))?;

        if let Some(err) = body.error {
            return Err(MtwError::Internal(format!(
                "openai API error ({}): {}",
                status, err.message
            )));
        }

        let choice = body
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .ok_or_else(|| MtwError::Internal("openai: no choices in response".into()))?;

        let msg = choice
            .message
            .as_ref()
            .ok_or_else(|| MtwError::Internal("openai: no message in choice".into()))?;

        let content = msg.content.clone().unwrap_or_default();
        let tool_calls = msg
            .tool_calls
            .as_ref()
            .map(|tc| parse_tool_calls(tc))
            .unwrap_or_default();

        let usage = body.usage.as_ref().map_or(Usage::default(), |u| Usage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
        });

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(FinishReason::from_openai)
            .unwrap_or(FinishReason::Stop);

        Ok(CompletionResponse {
            id: body.id.unwrap_or_default(),
            model: body.model.unwrap_or_default(),
            content,
            tool_calls,
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

        let mut oai_req = build_oai_request(&req, true);
        oai_req.model = model;

        let url = format!("{}/chat/completions", self.config.base_url);
        let client = self.client.clone();
        let api_key = self.config.api_key.clone();

        Box::pin(async_stream::try_stream! {
            let resp = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&oai_req)
                .send()
                .await
                .map_err(|e| MtwError::Internal(format!("openai stream request failed: {}", e)))?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                Err(MtwError::Internal(format!("openai stream error ({}): {}", status, body)))?;
                unreachable!();
            }

            let mut inner = super::sse::parse_oai_sse_stream(resp.bytes_stream(), "openai", true);
            while let Some(chunk) = StreamExt::next(&mut inner).await {
                yield chunk?;
            }
        })
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        Ok(vec![
            ModelInfo {
                id: GPT_4O.to_string(),
                name: "GPT-4o".to_string(),
                max_context: 128_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: GPT_4O_MINI.to_string(),
                name: "GPT-4o Mini".to_string(),
                max_context: 128_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: GPT_4_TURBO.to_string(),
                name: "GPT-4 Turbo".to_string(),
                max_context: 128_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: O1.to_string(),
                name: "o1".to_string(),
                max_context: 200_000,
                supports_tools: false,
                supports_vision: true,
            },
            ModelInfo {
                id: O1_MINI.to_string(),
                name: "o1-mini".to_string(),
                max_context: 128_000,
                supports_tools: false,
                supports_vision: false,
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Message;

    #[test]
    fn test_config_creation() {
        let config = OpenAIConfig::new("sk-test-key");
        assert_eq!(config.api_key, "sk-test-key");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.default_model, GPT_4O);
    }

    #[test]
    fn test_provider_name() {
        let provider = OpenAIProvider::new(OpenAIConfig::new("test"));
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_capabilities() {
        let provider = OpenAIProvider::new(OpenAIConfig::new("test"));
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(caps.tool_calling);
        assert!(caps.vision);
        assert!(caps.embeddings);
        assert_eq!(caps.max_context, 128_000);
    }

    #[tokio::test]
    async fn test_models_list() {
        let provider = OpenAIProvider::new(OpenAIConfig::new("test"));
        let models = provider.models().await.unwrap();
        assert!(models.len() >= 4);
        assert!(models.iter().any(|m| m.id == GPT_4O));
    }

    #[test]
    fn test_build_request() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![
                Message::system("You are helpful"),
                Message::user("Hello"),
            ],
            tools: None,
            temperature: Some(0.7),
            max_tokens: Some(1000),
            ..Default::default()
        };
        let oai = build_oai_request(&req, false);
        assert_eq!(oai.messages.len(), 2);
        assert_eq!(oai.messages[0].role, "system");
        assert!(oai.stream.is_none());
    }

    #[test]
    fn test_build_request_with_stream() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![Message::user("Hello")],
            ..Default::default()
        };
        let oai = build_oai_request(&req, true);
        assert_eq!(oai.stream, Some(true));
    }

    #[test]
    fn test_finish_reason_from_openai() {
        assert_eq!(FinishReason::from_openai("stop"), FinishReason::Stop);
        assert_eq!(FinishReason::from_openai("length"), FinishReason::Length);
        assert_eq!(FinishReason::from_openai("tool_calls"), FinishReason::ToolUse);
        assert_eq!(
            FinishReason::from_openai("content_filter"),
            FinishReason::ContentFilter
        );
        assert_eq!(FinishReason::from_openai("unknown"), FinishReason::Stop);
    }
}
