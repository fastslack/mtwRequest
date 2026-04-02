use async_trait::async_trait;
use futures::stream::StreamExt;
use futures::Stream;
use mtw_core::MtwError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::provider::{
    CompletionRequest, CompletionResponse, FinishReason, MessageRole, ModelInfo, MtwAIProvider,
    ProviderCapabilities, StreamChunk, ToolCall, Usage,
};

// Claude model constants
pub const CLAUDE_OPUS: &str = "claude-opus-4-20250514";
pub const CLAUDE_SONNET: &str = "claude-sonnet-4-20250514";
pub const CLAUDE_HAIKU: &str = "claude-haiku-4-5-20251001";

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Configuration for the Anthropic provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
}

fn default_base_url() -> String {
    "https://api.anthropic.com".to_string()
}

fn default_model() -> String {
    CLAUDE_SONNET.to_string()
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: default_base_url(),
            default_model: default_model(),
        }
    }
}

// --- Anthropic API request/response types ---

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AnthropicResponse {
    id: Option<String>,
    model: Option<String>,
    content: Option<Vec<AnthropicContent>>,
    usage: Option<AnthropicUsage>,
    stop_reason: Option<String>,
    error: Option<AnthropicError>,
    #[serde(rename = "type")]
    response_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    message: String,
}

// --- SSE event types for streaming ---

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    index: Option<usize>,
    delta: Option<StreamDelta>,
    content_block: Option<AnthropicContent>,
    message: Option<AnthropicResponse>,
    usage: Option<AnthropicUsage>,
    error: Option<AnthropicError>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(rename = "type")]
    delta_type: Option<String>,
    text: Option<String>,
    partial_json: Option<String>,
    stop_reason: Option<String>,
}

fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "end_turn" | "stop" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolUse,
        _ => FinishReason::Stop,
    }
}

fn build_anthropic_request(
    req: &CompletionRequest,
    default_model: &str,
    stream: bool,
) -> AnthropicRequest {
    let model = if req.model.is_empty() {
        default_model.to_string()
    } else {
        req.model.clone()
    };

    // Extract system message separately (Anthropic API requirement)
    let system = req
        .messages
        .iter()
        .find(|m| m.role == MessageRole::System)
        .map(|m| m.content.clone());

    // Filter out system messages from the messages array
    let messages = req
        .messages
        .iter()
        .filter(|m| m.role != MessageRole::System)
        .map(|m| {
            let role = match m.role {
                MessageRole::User | MessageRole::Tool => "user",
                MessageRole::Assistant => "assistant",
                _ => "user",
            };
            AnthropicMessage {
                role: role.to_string(),
                content: m.content.clone(),
            }
        })
        .collect();

    let tools = req.tools.as_ref().map(|ts| {
        ts.iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect()
    });

    let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);

    AnthropicRequest {
        model,
        max_tokens,
        system,
        messages,
        temperature: req.temperature,
        tools,
        stream: if stream { Some(true) } else { None },
    }
}

/// Anthropic AI provider (Claude models)
pub struct AnthropicProvider {
    config: AnthropicConfig,
    client: Client,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");
        Self { config, client }
    }

    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }
}

#[async_trait]
impl MtwAIProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            vision: true,
            embeddings: false,
            max_context: 200_000,
        }
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError> {
        let anthropic_req =
            build_anthropic_request(&req, &self.config.default_model, false);

        let url = format!("{}/v1/messages", self.config.base_url);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&anthropic_req)
            .send()
            .await
            .map_err(|e| MtwError::Internal(format!("anthropic request failed: {}", e)))?;

        let status = resp.status();
        let body: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| {
                MtwError::Internal(format!("anthropic response parse failed: {}", e))
            })?;

        if let Some(err) = body.error {
            return Err(MtwError::Internal(format!(
                "anthropic API error ({}): {}",
                status, err.message
            )));
        }

        // Extract text content and tool calls
        let mut content = String::new();
        let mut tool_calls = Vec::new();

        if let Some(contents) = &body.content {
            for block in contents {
                match block.content_type.as_str() {
                    "text" => {
                        if let Some(text) = &block.text {
                            content.push_str(text);
                        }
                    }
                    "tool_use" => {
                        tool_calls.push(ToolCall {
                            id: block.id.clone().unwrap_or_default(),
                            name: block.name.clone().unwrap_or_default(),
                            arguments: block
                                .input
                                .clone()
                                .unwrap_or(serde_json::json!({})),
                        });
                    }
                    _ => {}
                }
            }
        }

        let usage = body.usage.as_ref().map_or(Usage::default(), |u| {
            let input = u.input_tokens.unwrap_or(0);
            let output = u.output_tokens.unwrap_or(0);
            Usage {
                prompt_tokens: input,
                completion_tokens: output,
                total_tokens: input + output,
            }
        });

        let finish_reason = body
            .stop_reason
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
        let anthropic_req =
            build_anthropic_request(&req, &self.config.default_model, true);

        let url = format!("{}/v1/messages", self.config.base_url);
        let client = self.client.clone();
        let api_key = self.config.api_key.clone();

        Box::pin(async_stream::try_stream! {
            let resp = client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&anthropic_req)
                .send()
                .await
                .map_err(|e| MtwError::Internal(format!("anthropic stream request failed: {}", e)))?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                Err(MtwError::Internal(format!("anthropic stream error ({}): {}", status, body)))?;
                unreachable!();
            }

            let mut stream = resp.bytes_stream();
            let mut buffer = String::new();
            let mut current_event_type = String::new();

            // Track tool_use blocks being built during streaming
            let mut active_tool_id = String::new();
            let mut active_tool_name = String::new();
            let mut active_tool_json = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| MtwError::Internal(format!("anthropic stream read: {}", e)))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    if let Some(event) = line.strip_prefix("event: ") {
                        current_event_type = event.trim().to_string();
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        match current_event_type.as_str() {
                            "content_block_start" => {
                                if let Ok(evt) = serde_json::from_str::<StreamEvent>(data) {
                                    if let Some(block) = &evt.content_block {
                                        if block.content_type == "tool_use" {
                                            active_tool_id = block.id.clone().unwrap_or_default();
                                            active_tool_name = block.name.clone().unwrap_or_default();
                                            active_tool_json.clear();
                                        }
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Ok(evt) = serde_json::from_str::<StreamEvent>(data) {
                                    if let Some(delta) = &evt.delta {
                                        match delta.delta_type.as_deref() {
                                            Some("text_delta") => {
                                                let text = delta.text.clone().unwrap_or_default();
                                                yield StreamChunk {
                                                    delta: text,
                                                    tool_calls: vec![],
                                                    finish_reason: None,
                                                    usage: None,
                                                };
                                            }
                                            Some("input_json_delta") => {
                                                if let Some(partial) = &delta.partial_json {
                                                    active_tool_json.push_str(partial);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            "content_block_stop" => {
                                // If we were building a tool call, emit it now
                                if !active_tool_id.is_empty() {
                                    let arguments = serde_json::from_str(&active_tool_json)
                                        .unwrap_or(serde_json::json!({}));
                                    yield StreamChunk {
                                        delta: String::new(),
                                        tool_calls: vec![ToolCall {
                                            id: active_tool_id.clone(),
                                            name: active_tool_name.clone(),
                                            arguments,
                                        }],
                                        finish_reason: None,
                                        usage: None,
                                    };
                                    active_tool_id.clear();
                                    active_tool_name.clear();
                                    active_tool_json.clear();
                                }
                            }
                            "message_delta" => {
                                if let Ok(evt) = serde_json::from_str::<StreamEvent>(data) {
                                    let finish = evt
                                        .delta
                                        .as_ref()
                                        .and_then(|d| d.stop_reason.as_deref())
                                        .map(parse_finish_reason);
                                    let usage = evt.usage.as_ref().map(|u| {
                                        let output = u.output_tokens.unwrap_or(0);
                                        Usage {
                                            prompt_tokens: 0,
                                            completion_tokens: output,
                                            total_tokens: output,
                                        }
                                    });
                                    yield StreamChunk {
                                        delta: String::new(),
                                        tool_calls: vec![],
                                        finish_reason: finish,
                                        usage,
                                    };
                                }
                            }
                            "message_stop" => {
                                return;
                            }
                            "error" => {
                                if let Ok(evt) = serde_json::from_str::<StreamEvent>(data) {
                                    if let Some(err) = evt.error {
                                        Err(MtwError::Internal(format!("anthropic stream error: {}", err.message)))?;
                                    }
                                }
                            }
                            _ => {}
                        }
                        current_event_type.clear();
                    }
                }
            }
        })
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        Ok(vec![
            ModelInfo {
                id: CLAUDE_SONNET.to_string(),
                name: "Claude Sonnet 4".to_string(),
                max_context: 200_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: CLAUDE_HAIKU.to_string(),
                name: "Claude Haiku 4.5".to_string(),
                max_context: 200_000,
                supports_tools: true,
                supports_vision: true,
            },
            ModelInfo {
                id: CLAUDE_OPUS.to_string(),
                name: "Claude Opus 4".to_string(),
                max_context: 200_000,
                supports_tools: true,
                supports_vision: true,
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
        let config = AnthropicConfig::new("sk-test-key");
        assert_eq!(config.api_key, "sk-test-key");
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.default_model, CLAUDE_SONNET);
    }

    #[test]
    fn test_provider_name() {
        let provider = AnthropicProvider::new(AnthropicConfig::new("test"));
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_capabilities() {
        let provider = AnthropicProvider::new(AnthropicConfig::new("test"));
        let caps = provider.capabilities();
        assert!(caps.streaming);
        assert!(caps.tool_calling);
        assert!(caps.vision);
        assert!(!caps.embeddings);
        assert_eq!(caps.max_context, 200_000);
    }

    #[tokio::test]
    async fn test_models_list() {
        let provider = AnthropicProvider::new(AnthropicConfig::new("test"));
        let models = provider.models().await.unwrap();
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.id == CLAUDE_SONNET));
        assert!(models.iter().any(|m| m.id == CLAUDE_OPUS));
    }

    #[test]
    fn test_system_message_extraction() {
        let req = CompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![
                Message::system("You are helpful"),
                Message::user("Hello"),
                Message::assistant("Hi!"),
                Message::user("How are you?"),
            ],
            ..Default::default()
        };
        let anthropic_req = build_anthropic_request(&req, CLAUDE_SONNET, false);
        assert_eq!(anthropic_req.system, Some("You are helpful".to_string()));
        // System message should NOT appear in messages array
        assert_eq!(anthropic_req.messages.len(), 3);
        assert_eq!(anthropic_req.messages[0].role, "user");
    }

    #[test]
    fn test_default_max_tokens() {
        let req = CompletionRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![Message::user("Hello")],
            max_tokens: None,
            ..Default::default()
        };
        let anthropic_req = build_anthropic_request(&req, CLAUDE_SONNET, false);
        assert_eq!(anthropic_req.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn test_parse_finish_reason() {
        assert_eq!(parse_finish_reason("end_turn"), FinishReason::Stop);
        assert_eq!(parse_finish_reason("max_tokens"), FinishReason::Length);
        assert_eq!(parse_finish_reason("tool_use"), FinishReason::ToolUse);
    }
}
