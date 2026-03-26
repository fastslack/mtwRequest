# AI Agents Guide

mtwRequest treats AI agents as first-class citizens. This guide covers AI providers, the agent system, tool calling, streaming, and multi-agent orchestration.

---

## What Are AI Agents in mtwRequest?

An AI agent is a module that receives tasks (typically user messages), processes them through an AI provider (like Claude or GPT), and returns responses -- either as a single message or a token-by-token stream. Agents can call tools, maintain conversation context, and be orchestrated together in pipelines or fan-out patterns.

Key crate: `crates/mtw-ai/`

---

## The MtwAIProvider Trait

Providers are the abstraction over AI model APIs. Defined in `crates/mtw-ai/src/provider.rs`:

```rust
#[async_trait]
pub trait MtwAIProvider: Send + Sync {
    /// Provider name (e.g., "anthropic", "openai")
    fn name(&self) -> &str;

    /// What this provider supports
    fn capabilities(&self) -> ProviderCapabilities;

    /// Send a completion request and get a full response
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError>;

    /// Stream a completion token by token
    fn stream(
        &self,
        req: CompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>>;

    /// List available models
    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError>;
}
```

### Provider Capabilities

```rust
pub struct ProviderCapabilities {
    pub streaming: bool,       // Can stream token-by-token
    pub tool_calling: bool,    // Supports function/tool calling
    pub vision: bool,          // Supports image inputs
    pub embeddings: bool,      // Can generate embeddings
    pub max_context: usize,    // Maximum context window (tokens)
}
```

### CompletionRequest

```rust
pub struct CompletionRequest {
    pub model: String,                             // "claude-sonnet-4-6"
    pub messages: Vec<Message>,                    // Conversation history
    pub tools: Option<Vec<ToolDef>>,               // Available tools
    pub temperature: Option<f32>,                  // 0.0 - 2.0
    pub max_tokens: Option<u32>,                   // Max response tokens
    pub metadata: HashMap<String, Value>,          // Provider-specific options
}
```

### Message Types

```rust
let sys = Message::system("You are a helpful assistant.");
let usr = Message::user("What is mtwRequest?");
let asst = Message::assistant("mtwRequest is a real-time framework...");
```

---

## Built-in Providers

### Core Providers (mtw-ai crate)

These are defined as traits and stubs in the core AI crate:
- **Anthropic** (Claude) -- `crates/mtw-ai/src/providers/anthropic.rs`
- **OpenAI** (GPT) -- `crates/mtw-ai/src/providers/openai.rs`
- **Ollama** (local models) -- `crates/mtw-ai/src/providers/ollama.rs`

### Integration Providers (mtw-integrations crate)

Ten additional providers via `crates/mtw-integrations/src/ai/`:

| Provider | Module | Key Models |
|----------|--------|------------|
| Anthropic | `ai::anthropic` | Claude Opus 4, Claude Sonnet 4 |
| OpenAI | `ai::openai` | GPT-4o, GPT-4 Turbo |
| Google | `ai::google` | Gemini Pro, Gemini Ultra |
| Mistral | `ai::mistral` | Mistral Large, Mixtral |
| Cohere | `ai::cohere` | Command R+ |
| DeepSeek | `ai::deepseek` | DeepSeek V2 |
| Meta | `ai::meta` | Llama 3 |
| xAI | `ai::xai` | Grok |
| HuggingFace | `ai::huggingface` | Various open models |
| Ollama | `ai::ollama` | Any local model |

Each provides an `AiProviderInfo` struct, `AiProviderConfig`, and model constants.

### Provider Configuration

```toml
# mtw.toml
[[modules]]
name = "mtw-ai-anthropic"
version = "0.5"
config = {
    api_key = "${ANTHROPIC_API_KEY}",
    default_model = "claude-sonnet-4-6",
    default_temperature = 0.7,
    default_max_tokens = 4096,
    timeout_secs = 120
}
```

---

## The MtwAgent Trait

Agents are modules that handle tasks using AI providers. Defined in `crates/mtw-ai/src/agent.rs`:

```rust
#[async_trait]
pub trait MtwAgent: Send + Sync {
    /// Agent description for routing
    fn description(&self) -> &AgentDescription;

    /// Handle a task and return a complete response
    async fn handle(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError>;

    /// Handle a task with streaming response
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
```

### AgentDescription

```rust
pub struct AgentDescription {
    pub name: String,                    // "code-reviewer"
    pub role: String,                    // System prompt / description
    pub capabilities: Vec<String>,       // ["code-analysis", "suggestions"]
    pub accepts: Vec<String>,            // Channels: ["code-review", "chat.*"]
    pub max_concurrent: Option<usize>,   // Max parallel tasks
}
```

### AgentTask

```rust
pub struct AgentTask {
    pub id: String,                              // Unique task ID (ULID)
    pub from: String,                            // Source connection ID
    pub channel: Option<String>,                 // Target channel
    pub content: AgentContent,                   // Task content
    pub context: Vec<Message>,                   // Conversation history
    pub metadata: HashMap<String, Value>,        // Extra metadata
}

// Content variants
pub enum AgentContent {
    Text(String),
    Structured(Value),
    Binary(Vec<u8>),
    Multi(Vec<AgentContent>),
}
```

### AgentResponse and AgentChunk

```rust
// Complete response
pub struct AgentResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub done: bool,
    pub metadata: HashMap<String, Value>,
}

// Streaming chunk
pub struct AgentChunk {
    pub delta: String,           // Partial text
    pub tool_calls: Vec<ToolCall>,
    pub done: bool,              // Is this the final chunk?
}
```

---

## Creating a Custom Agent

```rust
use mtw_ai::agent::*;
use mtw_ai::provider::*;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

pub struct CodeExplainer {
    desc: AgentDescription,
}

impl CodeExplainer {
    pub fn new() -> Self {
        Self {
            desc: AgentDescription {
                name: "code-explainer".to_string(),
                role: "Explains code in simple terms".to_string(),
                capabilities: vec!["code-analysis".to_string()],
                accepts: vec!["code-help".to_string(), "code-help.*".to_string()],
                max_concurrent: Some(10),
            },
        }
    }
}

#[async_trait]
impl MtwAgent for CodeExplainer {
    fn description(&self) -> &AgentDescription {
        &self.desc
    }

    async fn handle(
        &self,
        task: AgentTask,
        _ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        let question = task.content.as_text()
            .ok_or_else(|| MtwError::Agent("expected text content".into()))?;

        // In a real agent, you would call ctx.provider.complete() here
        Ok(AgentResponse::text(format!(
            "Explanation of: {}",
            question
        )))
    }

    fn handle_stream(
        &self,
        task: AgentTask,
        _ctx: &AgentContext,
    ) -> Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>> {
        let text = task.content.as_text()
            .unwrap_or("no content")
            .to_string();

        Box::pin(futures::stream::iter(vec![
            Ok(AgentChunk::text(format!("Explaining: {}", text))),
            Ok(AgentChunk::done()),
        ]))
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::new(
                "read_file",
                "Read a source file",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path" }
                    },
                    "required": ["path"]
                }),
            ),
        ]
    }

    async fn on_tool_result(
        &self,
        result: ToolResult,
        _ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        Ok(AgentResponse::text(format!(
            "Tool {} returned: {}",
            result.name,
            result.result
        )))
    }
}
```

---

## Agent Tool Calling

### Defining Tools

```rust
let tool = ToolDef::new(
    "search",
    "Search the codebase",
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search query"
            },
            "max_results": {
                "type": "integer",
                "description": "Maximum results to return",
                "default": 10
            }
        },
        "required": ["query"]
    }),
);
```

### Tool Call Flow

```rust
pub struct ToolCall {
    pub id: String,                    // Unique call ID
    pub name: String,                  // Tool name
    pub arguments: serde_json::Value,  // Parameters from the LLM
}

pub struct ToolResult {
    pub tool_call_id: String,          // Matches ToolCall.id
    pub name: String,                  // Tool name
    pub result: serde_json::Value,     // The result
    pub is_error: bool,                // Whether it failed
}
```

### Client-side Tool Handling (React)

```tsx
const { send, messages, isStreaming } = useAgent("assistant", {
  tools: {
    search: async (params) => {
      const response = await fetch(`/api/search?q=${params.query}`);
      const data = await response.json();
      return JSON.stringify(data);
    },
    read_file: async (params) => {
      const response = await fetch(`/api/files?path=${params.path}`);
      return await response.text();
    },
  },
});
```

---

## Multi-Agent Orchestration

The `AgentOrchestrator` (in `crates/mtw-ai/src/orchestrator.rs`) routes tasks to the right agent(s) using configurable strategies.

### Routing Strategies

| Strategy | Description |
|----------|-------------|
| `ChannelBased` | Route based on which agent's `accepts` list matches the task's channel |
| `Pipeline(Vec<String>)` | Pass through agents in sequence; each agent's output feeds the next |
| `FanOut` | Send to ALL agents, merge results (concatenated with separators) |
| `RoundRobin` | Distribute tasks evenly across agents |

### ChannelBased Example

```rust
use mtw_ai::orchestrator::{AgentOrchestrator, RoutingStrategy};

let mut orch = AgentOrchestrator::new(RoutingStrategy::ChannelBased);

// Register agents with different channel patterns
orch.register_agent(Arc::new(chat_agent));       // accepts: ["chat.*"]
orch.register_agent(Arc::new(code_agent));       // accepts: ["code-review"]

// Route a task -- automatically goes to chat_agent
let task = AgentTask::text("conn-1", "hello")
    .with_channel("chat.general");
let response = orch.route(task, &ctx).await?;
```

### Pipeline Example

```rust
let mut orch = AgentOrchestrator::new(RoutingStrategy::Pipeline(vec![
    "translator".to_string(),
    "summarizer".to_string(),
]));

orch.register_agent(Arc::new(translator));
orch.register_agent(Arc::new(summarizer));

// Task goes through translator first, then summarizer
let task = AgentTask::text("conn-1", "Translate and summarize this article...");
let response = orch.route(task, &ctx).await?;
// response.content is the summarizer's output
```

### FanOut Example

```rust
let mut orch = AgentOrchestrator::new(RoutingStrategy::FanOut);

orch.register_agent(Arc::new(analyst_a));
orch.register_agent(Arc::new(analyst_b));

// Both agents process the task; results are merged
let task = AgentTask::text("conn-1", "Analyze this data...");
let response = orch.route(task, &ctx).await?;
// response.content contains both agents' outputs joined by "\n\n---\n\n"
```

### Streaming with Orchestrator

```rust
let stream = orch.route_stream(task, &ctx)?;
// Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>>
```

Note: Streaming is supported for `ChannelBased` and `RoundRobin` strategies. Pipeline and FanOut do not support streaming due to their sequential/parallel nature.

---

## Configuration in mtw.toml

```toml
# Define agents
[[agents]]
name = "assistant"
provider = "mtw-ai-anthropic"
model = "claude-sonnet-4-6"
system = "You are a helpful assistant."
tools = ["search", "calculator"]
channels = ["chat.*"]
max_concurrent = 10

[[agents]]
name = "code-reviewer"
provider = "mtw-ai-anthropic"
model = "claude-opus-4-6"
system = "You are a senior code reviewer. Focus on bugs, performance, and readability."
channels = ["code-review"]

# Orchestration strategy
[orchestrator]
strategy = "channel-based"   # Options: "channel-based", "pipeline", "fan-out", "round-robin"
```

---

## Complete Example: Building a Code Review Agent

```rust
use mtw_ai::agent::*;
use mtw_ai::provider::*;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;

pub struct CodeReviewAgent {
    desc: AgentDescription,
}

impl CodeReviewAgent {
    pub fn new() -> Self {
        Self {
            desc: AgentDescription {
                name: "code-reviewer".to_string(),
                role: "Senior code reviewer".to_string(),
                capabilities: vec![
                    "code-review".to_string(),
                    "bug-detection".to_string(),
                    "performance-analysis".to_string(),
                ],
                accepts: vec!["code-review".to_string()],
                max_concurrent: Some(5),
            },
        }
    }
}

#[async_trait]
impl MtwAgent for CodeReviewAgent {
    fn description(&self) -> &AgentDescription {
        &self.desc
    }

    async fn handle(
        &self,
        task: AgentTask,
        _ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        let code = task.content.as_text()
            .ok_or_else(|| MtwError::Agent("expected code as text".into()))?;

        // In production, call the AI provider:
        // let response = ctx.provider.complete(CompletionRequest {
        //     model: "claude-opus-4-6".into(),
        //     messages: vec![
        //         Message::system("You are a senior code reviewer..."),
        //         Message::user(code),
        //     ],
        //     tools: Some(self.tools()),
        //     ..Default::default()
        // }).await?;

        Ok(AgentResponse::text(format!(
            "Code review for {} bytes of code:\n\n\
             1. No critical issues found\n\
             2. Consider adding error handling\n\
             3. Good use of patterns",
            code.len()
        )))
    }

    fn handle_stream(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>> {
        Box::pin(futures::stream::iter(vec![
            Ok(AgentChunk::text("Reviewing code...\n")),
            Ok(AgentChunk::text("No critical issues found.\n")),
            Ok(AgentChunk::done()),
        ]))
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::new(
                "read_file",
                "Read a file from the repository",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            ),
            ToolDef::new(
                "search_code",
                "Search for patterns in the codebase",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "file_type": { "type": "string" }
                    },
                    "required": ["pattern"]
                }),
            ),
        ]
    }

    async fn on_tool_result(
        &self,
        result: ToolResult,
        _ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        Ok(AgentResponse::text(format!(
            "Analyzed tool result from '{}': {}",
            result.name, result.result
        )))
    }
}
```

---

## Next Steps

- [Protocol Guide](./protocol-guide.md) -- agent message format on the wire
- [Frontend Guide](./frontend-guide.md) -- useAgent() React hook
- [Auth Guide](./auth-guide.md) -- authenticate before agent access
