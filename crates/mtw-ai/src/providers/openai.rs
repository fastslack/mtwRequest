use async_trait::async_trait;
use futures::stream::StreamExt;
use futures::Stream;
use mtw_core::MtwError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, FinishReason, MessageRole, ModelInfo,
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

// --- OpenAI API request/response types ---

#[derive(Debug, Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OaiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OaiTool {
    r#type: String,
    function: OaiFunction,
}

#[derive(Debug, Serialize)]
struct OaiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OaiResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Option<Vec<OaiChoice>>,
    usage: Option<OaiUsage>,
    error: Option<OaiError>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: Option<OaiResponseMessage>,
    delta: Option<OaiDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OaiDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OaiToolCall {
    id: Option<String>,
    function: Option<OaiToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct OaiToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OaiError {
    message: String,
}

fn role_to_string(role: &MessageRole) -> String {
    match role {
        MessageRole::System => "system".to_string(),
        MessageRole::User => "user".to_string(),
        MessageRole::Assistant => "assistant".to_string(),
        MessageRole::Tool => "tool".to_string(),
    }
}

fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" => FinishReason::ToolUse,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Stop,
    }
}

fn build_oai_request(req: &CompletionRequest, stream: bool) -> OaiRequest {
    let messages = req
        .messages
        .iter()
        .map(|m| OaiMessage {
            role: role_to_string(&m.role),
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

fn parse_tool_calls(oai_calls: &[OaiToolCall]) -> Vec<ToolCall> {
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
            .map(parse_finish_reason)
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

            let mut stream = resp.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| MtwError::Internal(format!("openai stream read: {}", e)))?;
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

                        match serde_json::from_str::<OaiResponse>(data) {
                            Ok(parsed) => {
                                if let Some(choices) = &parsed.choices {
                                    if let Some(choice) = choices.first() {
                                        let delta_content = choice
                                            .delta
                                            .as_ref()
                                            .and_then(|d| d.content.clone())
                                            .unwrap_or_default();
                                        let tool_calls = choice
                                            .delta
                                            .as_ref()
                                            .and_then(|d| d.tool_calls.as_ref())
                                            .map(|tc| parse_tool_calls(tc))
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
                                            tool_calls,
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
    fn test_parse_finish_reason() {
        assert_eq!(parse_finish_reason("stop"), FinishReason::Stop);
        assert_eq!(parse_finish_reason("length"), FinishReason::Length);
        assert_eq!(parse_finish_reason("tool_calls"), FinishReason::ToolUse);
        assert_eq!(
            parse_finish_reason("content_filter"),
            FinishReason::ContentFilter
        );
        assert_eq!(parse_finish_reason("unknown"), FinishReason::Stop);
    }
}
