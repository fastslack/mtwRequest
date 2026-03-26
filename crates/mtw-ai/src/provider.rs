use async_trait::async_trait;
use futures::Stream;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;

/// AI provider capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_calling: bool,
    pub vision: bool,
    pub embeddings: bool,
    pub max_context: usize,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            streaming: false,
            tool_calling: false,
            vision: false,
            embeddings: false,
            max_context: 4096,
        }
    }
}

/// A message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

/// Message role in a conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Tool definition for function calling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolDef {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// A tool call requested by the AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result of a tool invocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub result: serde_json::Value,
    pub is_error: bool,
}

/// Completion request sent to a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            model: String::new(),
            messages: Vec::new(),
            tools: None,
            temperature: None,
            max_tokens: None,
            metadata: HashMap::new(),
        }
    }
}

/// Completion response from a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub model: String,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: FinishReason,
}

/// Token usage information
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Reason the model stopped generating
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    ContentFilter,
}

/// A streaming chunk from the provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub delta: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<FinishReason>,
    pub usage: Option<Usage>,
}

/// Information about an available model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub max_context: usize,
    pub supports_tools: bool,
    pub supports_vision: bool,
}

/// AI provider trait -- abstraction over different AI services
#[async_trait]
pub trait MtwAIProvider: Send + Sync {
    /// Provider name
    fn name(&self) -> &str;

    /// Provider capabilities
    fn capabilities(&self) -> ProviderCapabilities;

    /// Send a completion request and get a full response
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError>;

    /// Stream a completion response token by token
    fn stream(
        &self,
        req: CompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>>;

    /// List available models
    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_constructors() {
        let sys = Message::system("You are helpful");
        assert_eq!(sys.role, MessageRole::System);
        assert_eq!(sys.content, "You are helpful");

        let usr = Message::user("Hello");
        assert_eq!(usr.role, MessageRole::User);

        let asst = Message::assistant("Hi there");
        assert_eq!(asst.role, MessageRole::Assistant);
    }

    #[test]
    fn test_tool_def() {
        let tool = ToolDef::new(
            "search",
            "Search the web",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        );
        assert_eq!(tool.name, "search");
    }

    #[test]
    fn test_completion_request_default() {
        let req = CompletionRequest::default();
        assert!(req.model.is_empty());
        assert!(req.messages.is_empty());
        assert!(req.tools.is_none());
        assert!(req.temperature.is_none());
    }

    #[test]
    fn test_provider_capabilities_default() {
        let caps = ProviderCapabilities::default();
        assert!(!caps.streaming);
        assert!(!caps.tool_calling);
        assert_eq!(caps.max_context, 4096);
    }

    #[test]
    fn test_serialization() {
        let msg = Message::user("Hello");
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.role, MessageRole::User);
        assert_eq!(deserialized.content, "Hello");
    }
}
