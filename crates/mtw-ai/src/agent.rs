use async_trait::async_trait;
use futures::Stream;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;

use crate::provider::{Message, ToolCall, ToolDef, ToolResult};

/// Agent description for routing and identification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDescription {
    /// Agent name (unique identifier)
    pub name: String,
    /// Agent role / system prompt description
    pub role: String,
    /// List of capabilities this agent offers
    pub capabilities: Vec<String>,
    /// Message types / channels this agent handles
    pub accepts: Vec<String>,
    /// Maximum concurrent tasks (None = unlimited)
    pub max_concurrent: Option<usize>,
}

/// A task submitted to an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    /// Unique task identifier
    pub id: String,
    /// Source connection or caller ID
    pub from: String,
    /// Target channel (optional)
    pub channel: Option<String>,
    /// Task content
    pub content: AgentContent,
    /// Conversation history / context
    pub context: Vec<Message>,
    /// Arbitrary metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl AgentTask {
    pub fn new(from: impl Into<String>, content: AgentContent) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            from: from.into(),
            channel: None,
            content,
            context: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn text(from: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(from, AgentContent::Text(text.into()))
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    pub fn with_context(mut self, context: Vec<Message>) -> Self {
        self.context = context;
        self
    }
}

/// Content variants for agent tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum AgentContent {
    Text(String),
    Structured(serde_json::Value),
    Binary(Vec<u8>),
    Multi(Vec<AgentContent>),
}

impl AgentContent {
    /// Get content as text if it is a text variant
    pub fn as_text(&self) -> Option<&str> {
        match self {
            AgentContent::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Context available to an agent during task execution
pub struct AgentContext {
    /// Metadata about the current execution
    pub metadata: HashMap<String, serde_json::Value>,
}

impl AgentContext {
    pub fn new() -> Self {
        Self {
            metadata: HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

impl Default for AgentContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Full response from an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// Response content
    pub content: String,
    /// Tool calls requested by the agent
    pub tool_calls: Vec<ToolCall>,
    /// Whether the agent is done or needs more input
    pub done: bool,
    /// Additional metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl AgentResponse {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            tool_calls: Vec::new(),
            done: true,
            metadata: HashMap::new(),
        }
    }

    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self.done = false;
        self
    }
}

/// A streaming chunk from an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChunk {
    /// Partial text content
    pub delta: String,
    /// Tool calls in this chunk
    pub tool_calls: Vec<ToolCall>,
    /// Whether this is the final chunk
    pub done: bool,
}

impl AgentChunk {
    pub fn text(delta: impl Into<String>) -> Self {
        Self {
            delta: delta.into(),
            tool_calls: Vec::new(),
            done: false,
        }
    }

    pub fn done() -> Self {
        Self {
            delta: String::new(),
            tool_calls: Vec::new(),
            done: true,
        }
    }
}

/// Agent trait -- defines an AI agent that can handle tasks
#[async_trait]
pub trait MtwAgent: Send + Sync {
    /// Agent description for routing and identification
    fn description(&self) -> &AgentDescription;

    /// Handle an incoming task and produce a full response
    async fn handle(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError>;

    /// Handle an incoming task with streaming response
    fn handle_stream(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>>;

    /// List tools this agent can use
    fn tools(&self) -> Vec<ToolDef>;

    /// Called when a tool execution completes
    async fn on_tool_result(
        &self,
        result: ToolResult,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_task_creation() {
        let task = AgentTask::text("conn-1", "Hello agent");
        assert_eq!(task.from, "conn-1");
        assert_eq!(task.content.as_text(), Some("Hello agent"));
        assert!(task.channel.is_none());
    }

    #[test]
    fn test_agent_task_with_channel() {
        let task = AgentTask::text("conn-1", "Hello").with_channel("chat.general");
        assert_eq!(task.channel, Some("chat.general".to_string()));
    }

    #[test]
    fn test_agent_response_text() {
        let resp = AgentResponse::text("Hello back");
        assert_eq!(resp.content, "Hello back");
        assert!(resp.done);
        assert!(resp.tool_calls.is_empty());
    }

    #[test]
    fn test_agent_chunk() {
        let chunk = AgentChunk::text("partial");
        assert_eq!(chunk.delta, "partial");
        assert!(!chunk.done);

        let done = AgentChunk::done();
        assert!(done.done);
        assert!(done.delta.is_empty());
    }

    #[test]
    fn test_agent_content_as_text() {
        let text = AgentContent::Text("hello".into());
        assert_eq!(text.as_text(), Some("hello"));

        let structured = AgentContent::Structured(serde_json::json!({"key": "val"}));
        assert!(structured.as_text().is_none());
    }

    #[test]
    fn test_agent_context() {
        let ctx = AgentContext::new()
            .with_metadata("key", serde_json::json!("value"));
        assert_eq!(
            ctx.metadata.get("key"),
            Some(&serde_json::json!("value"))
        );
    }
}
