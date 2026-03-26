# mtwRequest - Architecture Design

## Vision

A Rust-based, modular real-time framework that unifies WebSocket, HTTP/API, and AI agents
into a single high-performance core with bindings for Node.js, Python, PHP, and Browser (WASM).
Users can create, install, and share modules through a marketplace.

---

## Core Principles

1. **Rust core, polyglot surface** — write once, bind everywhere
2. **Module-first** — everything is a module, even built-in features
3. **Protocol-agnostic** — WebSocket, HTTP, SSE, or custom transports
4. **AI-native** — agents, streaming, tool calling as first-class citizens
5. **Zero-opinion frontend** — works with React, Svelte, Vue, Three.js, or raw JS

---

## System Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                        mtwRequest Core (Rust)                    │
│                                                                  │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌───────────┐  │
│  │ Transport  │  │  Router    │  │  AI Engine │  │  Module   │  │
│  │  Layer     │  │            │  │            │  │  Runtime  │  │
│  │            │  │ - paths    │  │ - agents   │  │           │  │
│  │ - ws       │  │ - channels │  │ - streams  │  │ - load    │  │
│  │ - http     │  │ - rooms    │  │ - tools    │  │ - isolate │  │
│  │ - sse      │  │ - middleware│ │ - memory   │  │ - sandbox │  │
│  │ - quic     │  │            │  │ - providers│  │ - hooks   │  │
│  └────────────┘  └────────────┘  └────────────┘  └───────────┘  │
│                                                                  │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌───────────┐  │
│  │ Codec      │  │  State     │  │  Auth      │  │  Registry │  │
│  │            │  │  Store     │  │            │  │  Client   │  │
│  │ - json     │  │            │  │ - jwt      │  │           │  │
│  │ - msgpack  │  │ - memory   │  │ - api keys │  │ - resolve │  │
│  │ - protobuf │  │ - redis    │  │ - oauth    │  │ - install │  │
│  │ - binary   │  │ - custom   │  │ - custom   │  │ - update  │  │
│  └────────────┘  └────────────┘  └────────────┘  └───────────┘  │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │                    Module API (Trait System)                 │ │
│  │  MtwModule · MtwTransport · MtwMiddleware · MtwAIProvider   │ │
│  │  MtwCodec · MtwAuth · MtwStorage · MtwAgent                 │ │
│  └──────────────────────────────────────────────────────────────┘ │
└──────────┬──────────┬──────────┬──────────┬─────────────────────┘
           │          │          │          │
    ┌──────▼───┐ ┌────▼────┐ ┌──▼─────┐ ┌─▼──────┐
    │ NAPI-RS  │ │  PyO3   │ │PHP FFI │ │  WASM  │
    │ Node.js  │ │ Python  │ │  PHP   │ │Browser │
    └──────────┘ └─────────┘ └────────┘ └────────┘
```

---

## Crate Structure

```
mtw-request/
├── Cargo.toml                    # workspace root
│
├── crates/
│   ├── mtw-core/                 # kernel: event loop, module loader, lifecycle
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── server.rs         # main server orchestration
│   │   │   ├── module.rs         # Module trait + registry
│   │   │   ├── config.rs         # TOML config loader
│   │   │   ├── hooks.rs          # lifecycle hooks system
│   │   │   └── error.rs
│   │   └── Cargo.toml
│   │
│   ├── mtw-transport/            # transport abstraction + built-in transports
│   │   ├── src/
│   │   │   ├── lib.rs            # Transport trait
│   │   │   ├── ws.rs             # WebSocket (tokio-tungstenite)
│   │   │   ├── http.rs           # HTTP/REST (hyper)
│   │   │   ├── sse.rs            # Server-Sent Events
│   │   │   └── quic.rs           # QUIC (future)
│   │   └── Cargo.toml
│   │
│   ├── mtw-router/               # message routing, channels, rooms
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── channel.rs        # pub/sub channels
│   │   │   ├── room.rs           # rooms with presence
│   │   │   ├── middleware.rs     # middleware chain
│   │   │   └── path.rs           # path-based routing
│   │   └── Cargo.toml
│   │
│   ├── mtw-ai/                   # AI engine: agents, providers, streaming
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── provider.rs       # AIProvider trait (Anthropic, OpenAI, etc.)
│   │   │   ├── agent.rs          # Agent runtime
│   │   │   ├── tool.rs           # Tool calling system
│   │   │   ├── memory.rs         # Agent memory/context
│   │   │   ├── stream.rs         # Token streaming over transport
│   │   │   ├── orchestrator.rs   # Multi-agent orchestration
│   │   │   └── providers/
│   │   │       ├── anthropic.rs
│   │   │       ├── openai.rs
│   │   │       └── ollama.rs     # local models
│   │   └── Cargo.toml
│   │
│   ├── mtw-codec/                # serialization: JSON, MessagePack, Protobuf
│   │   ├── src/
│   │   │   ├── lib.rs            # Codec trait
│   │   │   ├── json.rs
│   │   │   ├── msgpack.rs
│   │   │   └── binary.rs         # raw binary for 3D/audio
│   │   └── Cargo.toml
│   │
│   ├── mtw-auth/                 # authentication modules
│   │   ├── src/
│   │   │   ├── lib.rs            # Auth trait
│   │   │   ├── jwt.rs
│   │   │   ├── apikey.rs
│   │   │   └── oauth.rs
│   │   └── Cargo.toml
│   │
│   ├── mtw-state/                # state management
│   │   ├── src/
│   │   │   ├── lib.rs            # StateStore trait
│   │   │   ├── memory.rs         # in-memory store
│   │   │   └── redis.rs          # Redis adapter
│   │   └── Cargo.toml
│   │
│   ├── mtw-registry/             # marketplace client + module resolver
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── resolver.rs       # dependency resolution
│   │   │   ├── manifest.rs       # module manifest parser
│   │   │   ├── client.rs         # registry API client
│   │   │   └── sandbox.rs        # WASM sandbox for untrusted modules
│   │   └── Cargo.toml
│   │
│   ├── mtw-cli/                  # CLI tool: mtw init, mtw add, mtw publish
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── commands/
│   │   │   │   ├── init.rs       # mtw init
│   │   │   │   ├── add.rs        # mtw add <module>
│   │   │   │   ├── remove.rs     # mtw remove <module>
│   │   │   │   ├── publish.rs    # mtw publish
│   │   │   │   ├── dev.rs        # mtw dev (hot reload)
│   │   │   │   └── search.rs     # mtw search <query>
│   │   │   └── scaffold.rs      # project templates
│   │   └── Cargo.toml
│   │
│   ├── mtw-sdk/                  # SDK for module developers
│   │   ├── src/
│   │   │   ├── lib.rs            # re-exports + proc macros
│   │   │   └── macros.rs         # #[mtw_module], #[mtw_handler], etc.
│   │   └── Cargo.toml
│   │
│   │── mtw-protocol/             # wire protocol definition
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── message.rs        # MtwMessage enum
│   │   │   ├── frame.rs          # wire frame format
│   │   │   └── version.rs        # protocol versioning
│   │   └── Cargo.toml
│   │
│   └── mtw-test/                 # testing utilities for module devs
│       ├── src/
│       │   ├── lib.rs
│       │   ├── mock_transport.rs
│       │   ├── mock_agent.rs
│       │   └── harness.rs        # test server harness
│       └── Cargo.toml
│
├── bindings/
│   ├── node/                     # NAPI-RS binding
│   │   ├── src/lib.rs
│   │   ├── package.json
│   │   └── index.d.ts
│   ├── python/                   # PyO3 binding
│   │   ├── src/lib.rs
│   │   └── pyproject.toml
│   ├── php/                      # PHP FFI binding
│   │   ├── src/lib.rs
│   │   └── composer.json
│   └── wasm/                     # WASM binding
│       ├── src/lib.rs
│       └── package.json
│
├── packages/                     # Frontend SDKs (TypeScript)
│   ├── client/                   # @mtw/client - universal JS client
│   │   ├── src/
│   │   │   ├── index.ts
│   │   │   ├── connection.ts
│   │   │   ├── channel.ts
│   │   │   └── agent.ts          # AI agent client
│   │   └── package.json
│   ├── react/                    # @mtw/react
│   │   ├── src/
│   │   │   ├── index.ts
│   │   │   ├── useMtw.ts         # connection hook
│   │   │   ├── useChannel.ts     # channel subscription
│   │   │   ├── useAgent.ts       # AI agent hook
│   │   │   ├── useStream.ts      # token streaming
│   │   │   └── MtwProvider.tsx   # context provider
│   │   └── package.json
│   ├── svelte/                   # @mtw/svelte
│   │   └── ...
│   ├── vue/                      # @mtw/vue
│   │   └── ...
│   └── three/                    # @mtw/three - Three.js real-time sync
│       ├── src/
│       │   ├── index.ts
│       │   ├── useMtwScene.ts    # scene sync over binary channel
│       │   └── useMtwAsset.ts    # real-time asset streaming
│       └── package.json
│
├── registry/                     # Marketplace backend (Rust)
│   ├── src/
│   │   ├── main.rs
│   │   ├── api.rs                # REST API for registry
│   │   ├── storage.rs            # module storage (S3/R2)
│   │   ├── search.rs             # module search
│   │   ├── auth.rs               # publisher auth
│   │   └── verify.rs             # module verification/scanning
│   └── Cargo.toml
│
└── docs/
    ├── getting-started.md
    ├── creating-modules.md
    ├── ai-agents.md
    └── marketplace.md
```

---

## Core Trait System

### MtwModule — The Universal Module Interface

Every module implements this single trait. This is what makes the system pluggable.

```rust
use async_trait::async_trait;

/// Module metadata, loaded from mtw-module.toml
#[derive(Debug, Clone)]
pub struct ModuleManifest {
    pub name: String,           // "mtw-auth-jwt"
    pub version: String,        // "1.2.0"
    pub module_type: ModuleType,
    pub description: String,
    pub author: String,
    pub license: String,
    pub repository: Option<String>,
    pub dependencies: Vec<ModuleDep>,
    pub config_schema: Option<serde_json::Value>,  // JSON Schema for config
    pub permissions: Vec<Permission>,               // what the module needs
}

#[derive(Debug, Clone)]
pub enum ModuleType {
    Transport,      // WebSocket, HTTP, QUIC, custom
    Middleware,     // request/response pipeline
    AIProvider,     // Anthropic, OpenAI, Ollama, custom
    AIAgent,        // pre-built agent behaviors
    Codec,          // JSON, MessagePack, Protobuf
    Auth,           // JWT, OAuth, API keys
    Storage,        // Redis, Postgres, S3, custom
    Channel,        // custom channel logic
    Integration,    // third-party service connectors
    UI,             // frontend components (shipped as npm)
}

#[derive(Debug, Clone)]
pub enum Permission {
    Network,            // can make outbound HTTP
    FileSystem,         // can read/write files
    Environment,        // can read env vars
    Subprocess,         // can spawn processes
    Database,           // can connect to databases
    Custom(String),     // module-defined permission
}

/// The core module trait - everything implements this
#[async_trait]
pub trait MtwModule: Send + Sync {
    /// Module manifest
    fn manifest(&self) -> &ModuleManifest;

    /// Called when the module is loaded
    async fn on_load(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Called when the server starts
    async fn on_start(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Called when the server stops
    async fn on_stop(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Health check
    async fn health(&self) -> HealthStatus {
        HealthStatus::Healthy
    }
}
```

### MtwTransport — Transport Layer

```rust
#[async_trait]
pub trait MtwTransport: MtwModule {
    /// Start listening for connections
    async fn listen(&mut self, addr: SocketAddr) -> Result<(), MtwError>;

    /// Send a message to a specific connection
    async fn send(&self, conn_id: &ConnId, msg: MtwMessage) -> Result<(), MtwError>;

    /// Send binary data to a specific connection
    async fn send_binary(&self, conn_id: &ConnId, data: &[u8]) -> Result<(), MtwError>;

    /// Broadcast to all connections
    async fn broadcast(&self, msg: MtwMessage) -> Result<(), MtwError>;

    /// Close a connection
    async fn close(&self, conn_id: &ConnId) -> Result<(), MtwError>;

    /// Stream of incoming events
    fn events(&self) -> Pin<Box<dyn Stream<Item = TransportEvent> + Send>>;
}

pub enum TransportEvent {
    Connected(ConnId, ConnMetadata),
    Disconnected(ConnId, DisconnectReason),
    Message(ConnId, MtwMessage),
    Binary(ConnId, Vec<u8>),
    Error(ConnId, MtwError),
}
```

### MtwAIProvider — AI Provider Abstraction

```rust
#[async_trait]
pub trait MtwAIProvider: MtwModule {
    /// Get provider capabilities
    fn capabilities(&self) -> ProviderCapabilities;

    /// Send a completion request
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError>;

    /// Stream a completion (token by token)
    fn stream(&self, req: CompletionRequest)
        -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>>;

    /// List available models
    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError>;
}

pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_calling: bool,
    pub vision: bool,
    pub embeddings: bool,
    pub max_context: usize,
}

pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Option<Vec<ToolDef>>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub metadata: HashMap<String, Value>,
}
```

### MtwAgent — AI Agent System

```rust
#[async_trait]
pub trait MtwAgent: MtwModule {
    /// Agent description for routing
    fn description(&self) -> &AgentDescription;

    /// Handle an incoming message/task
    async fn handle(&mut self, task: AgentTask, ctx: &AgentContext) -> Result<AgentResponse, MtwError>;

    /// Stream response chunks
    fn handle_stream(&mut self, task: AgentTask, ctx: &AgentContext)
        -> Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>>;

    /// List tools this agent can use
    fn tools(&self) -> Vec<ToolDef>;

    /// Called when a tool execution completes
    async fn on_tool_result(&mut self, result: ToolResult, ctx: &AgentContext)
        -> Result<AgentResponse, MtwError>;
}

pub struct AgentDescription {
    pub name: String,
    pub role: String,                    // "You are a code reviewer..."
    pub capabilities: Vec<String>,
    pub accepts: Vec<String>,            // message types this agent handles
    pub max_concurrent: Option<usize>,
}

pub struct AgentTask {
    pub id: String,
    pub from: ConnId,
    pub channel: Option<String>,
    pub content: AgentContent,
    pub context: Vec<Message>,           // conversation history
    pub metadata: HashMap<String, Value>,
}

pub enum AgentContent {
    Text(String),
    Structured(Value),
    Binary(Vec<u8>),
    Multi(Vec<AgentContent>),
}

pub struct AgentContext {
    pub provider: Arc<dyn MtwAIProvider>,
    pub state: Arc<dyn MtwStateStore>,
    pub transport: Arc<dyn MtwTransport>,
    pub tools: ToolRegistry,
    pub memory: AgentMemory,
}

/// Multi-agent orchestrator
pub struct AgentOrchestrator {
    agents: HashMap<String, Arc<dyn MtwAgent>>,
    routing_strategy: RoutingStrategy,
}

pub enum RoutingStrategy {
    /// Route by message type/channel
    ChannelBased,
    /// AI decides which agent handles the message
    AIRouter { provider: Arc<dyn MtwAIProvider> },
    /// User-defined routing function
    Custom(Box<dyn Fn(&AgentTask) -> String + Send + Sync>),
    /// Pipeline: message goes through agents in sequence
    Pipeline(Vec<String>),
    /// Fan-out: message goes to all agents, merge results
    FanOut { merge: MergeStrategy },
}
```

### MtwMiddleware — Request/Response Pipeline

```rust
#[async_trait]
pub trait MtwMiddleware: MtwModule {
    /// Process incoming message (before handler)
    async fn on_inbound(&self, msg: &mut MtwMessage, ctx: &MiddlewareContext)
        -> Result<MiddlewareAction, MtwError>;

    /// Process outgoing message (before send)
    async fn on_outbound(&self, msg: &mut MtwMessage, ctx: &MiddlewareContext)
        -> Result<MiddlewareAction, MtwError>;
}

pub enum MiddlewareAction {
    Continue,                    // pass to next middleware
    Halt,                        // stop the chain
    Redirect(String),            // redirect to different channel
    Transform(MtwMessage),       // replace the message
}
```

---

## Wire Protocol

```rust
/// Every message on the wire follows this format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtwMessage {
    pub id: String,              // unique message ID (ULID)
    pub msg_type: MsgType,
    pub channel: Option<String>, // target channel/room
    pub payload: Payload,
    pub metadata: HashMap<String, Value>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MsgType {
    // Transport
    Connect,
    Disconnect,
    Ping,
    Pong,

    // Data
    Request,                     // client → server, expects response
    Response,                    // server → client, response to request
    Event,                       // one-way event (no response expected)
    Stream,                      // streaming chunk
    StreamEnd,                   // end of stream

    // Channels
    Subscribe,
    Unsubscribe,
    Publish,

    // AI Agent
    AgentTask,                   // send task to agent
    AgentChunk,                  // streaming agent response
    AgentToolCall,               // agent wants to call a tool
    AgentToolResult,             // tool result back to agent
    AgentComplete,               // agent finished

    // System
    Error,
    Ack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Payload {
    None,
    Text(String),
    Json(Value),
    Binary(Vec<u8>),
}
```

---

## Module Manifest (mtw-module.toml)

Every installable module ships with this file:

```toml
[module]
name = "mtw-auth-jwt"
version = "1.0.0"
type = "auth"
description = "JWT authentication for mtwRequest"
author = "fastslack"
license = "MIT"
repository = "https://github.com/fastslack/mtw-auth-jwt"
minimum_core = "0.1.0"

[permissions]
network = false
filesystem = false
environment = true     # needs to read JWT_SECRET

[dependencies]
mtw-core = "0.1"

[config]
# JSON Schema for this module's config
[config.properties.secret]
type = "string"
description = "JWT signing secret"
required = true

[config.properties.expiration]
type = "integer"
description = "Token expiration in seconds"
default = 3600

# Optional: npm package to install alongside (for frontend components)
[ui]
package = "@mtw/auth-jwt-react"
version = "1.0.0"
```

---

## Server Configuration (mtw.toml)

```toml
[server]
host = "0.0.0.0"
port = 8080
max_connections = 10000

[transport]
default = "websocket"

[transport.websocket]
path = "/ws"
ping_interval = 30
max_message_size = "10MB"

[transport.http]
enabled = true
prefix = "/api"

[codec]
default = "json"
binary_channels = ["3d-sync", "audio"]

# Modules to load
[[modules]]
name = "mtw-auth-jwt"
version = "1.2"
config = { secret = "${JWT_SECRET}", expiration = 7200 }

[[modules]]
name = "mtw-ai-anthropic"
version = "0.5"
config = { api_key = "${ANTHROPIC_API_KEY}", default_model = "claude-sonnet-4-6" }

[[modules]]
name = "mtw-store-redis"
version = "1.0"
config = { url = "${REDIS_URL}" }

# AI Agents
[[agents]]
name = "assistant"
provider = "mtw-ai-anthropic"
model = "claude-sonnet-4-6"
system = "You are a helpful assistant."
tools = ["search", "calculator"]
channels = ["chat.*"]              # handles messages on chat.* channels

[[agents]]
name = "code-reviewer"
provider = "mtw-ai-anthropic"
model = "claude-opus-4-6"
system = "You are a senior code reviewer."
channels = ["code-review"]

# Agent orchestration
[orchestrator]
strategy = "channel-based"         # or "ai-router", "pipeline", "fan-out"

# Channels
[[channels]]
name = "chat.*"
auth = true
max_members = 100
history = 50

[[channels]]
name = "3d-sync"
codec = "binary"
auth = true
```

---

## CLI (mtw)

```bash
# Project management
mtw init                          # scaffold new project
mtw init --template chat          # from template
mtw init --template ai-agent      # AI agent project
mtw dev                           # start dev server with hot reload

# Module management
mtw add mtw-auth-jwt              # install from marketplace
mtw add mtw-auth-jwt@1.2          # specific version
mtw add ./my-local-module         # install local module
mtw remove mtw-auth-jwt
mtw list                          # list installed modules
mtw update                        # update all modules

# Marketplace
mtw search "auth"                 # search modules
mtw search --type ai-provider     # search by type
mtw publish                       # publish to marketplace
mtw info mtw-auth-jwt             # module details

# Code generation
mtw generate module my-module     # scaffold a new module
mtw generate agent my-agent       # scaffold a new AI agent
mtw generate middleware my-mw     # scaffold middleware
mtw generate provider my-prov     # scaffold AI provider

# Testing
mtw test                          # run module tests
mtw bench                         # benchmark
```

---

## Creating a Module (Developer Experience)

### 1. Scaffold

```bash
mtw generate module my-rate-limiter
```

### 2. Implement

```rust
// my-rate-limiter/src/lib.rs
use mtw_sdk::prelude::*;

#[mtw_module]
pub struct RateLimiter {
    max_requests: u32,
    window_secs: u64,
    counters: DashMap<ConnId, (u32, Instant)>,
}

#[mtw_config]
pub struct RateLimiterConfig {
    /// Max requests per window
    pub max_requests: u32,
    /// Window duration in seconds
    #[default = 60]
    pub window_secs: u64,
}

#[async_trait]
impl MtwMiddleware for RateLimiter {
    async fn on_inbound(&self, msg: &mut MtwMessage, ctx: &MiddlewareContext)
        -> Result<MiddlewareAction, MtwError>
    {
        let conn_id = ctx.connection_id();
        let now = Instant::now();

        let mut entry = self.counters
            .entry(conn_id.clone())
            .or_insert((0, now));

        if now.duration_since(entry.1).as_secs() > self.window_secs {
            *entry = (1, now);
            return Ok(MiddlewareAction::Continue);
        }

        entry.0 += 1;
        if entry.0 > self.max_requests {
            return Ok(MiddlewareAction::Halt);
        }

        Ok(MiddlewareAction::Continue)
    }

    async fn on_outbound(&self, _msg: &mut MtwMessage, _ctx: &MiddlewareContext)
        -> Result<MiddlewareAction, MtwError>
    {
        Ok(MiddlewareAction::Continue)
    }
}
```

### 3. Test

```rust
#[cfg(test)]
mod tests {
    use mtw_test::prelude::*;

    #[mtw_test]
    async fn test_rate_limiting() {
        let server = TestServer::new()
            .with_module(RateLimiter::new(RateLimiterConfig {
                max_requests: 5,
                window_secs: 60,
            }))
            .start()
            .await;

        let client = server.connect().await;

        for _ in 0..5 {
            let res = client.send_text("hello").await;
            assert!(res.is_ok());
        }

        // 6th request should be rate limited
        let res = client.send_text("hello").await;
        assert!(res.is_err());
    }
}
```

### 4. Publish

```bash
mtw publish
# > Publishing my-rate-limiter@0.1.0 to mtwRequest marketplace...
# > Published! https://marketplace.mtw.dev/modules/my-rate-limiter
```

---

## Creating an AI Agent Module

```rust
use mtw_sdk::prelude::*;

#[mtw_agent(
    name = "code-explainer",
    description = "Explains code in simple terms",
    channels = ["code-help"]
)]
pub struct CodeExplainer;

#[async_trait]
impl MtwAgent for CodeExplainer {
    fn tools(&self) -> Vec<ToolDef> {
        vec![
            tool! {
                name: "read_file",
                description: "Read a file from the project",
                params: {
                    path: String => "File path to read"
                }
            },
            tool! {
                name: "search_code",
                description: "Search for code patterns",
                params: {
                    query: String => "Search query"
                }
            },
        ]
    }

    async fn handle(&mut self, task: AgentTask, ctx: &AgentContext)
        -> Result<AgentResponse, MtwError>
    {
        let response = ctx.provider.complete(CompletionRequest {
            model: "claude-sonnet-4-6".into(),
            messages: vec![
                Message::system("You explain code in simple terms. Be concise."),
                Message::user(task.content.as_text()?),
            ],
            tools: Some(self.tools()),
            ..Default::default()
        }).await?;

        Ok(AgentResponse::text(response.content))
    }

    fn handle_stream(&mut self, task: AgentTask, ctx: &AgentContext)
        -> Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>>
    {
        let provider = ctx.provider.clone();
        let tools = self.tools();

        Box::pin(async_stream::stream! {
            let stream = provider.stream(CompletionRequest {
                model: "claude-sonnet-4-6".into(),
                messages: vec![
                    Message::system("You explain code in simple terms."),
                    Message::user(task.content.as_text().unwrap()),
                ],
                tools: Some(tools),
                ..Default::default()
            });

            pin_mut!(stream);
            while let Some(chunk) = stream.next().await {
                yield chunk.map(|c| AgentChunk::from(c));
            }
        })
    }
}
```

---

## Frontend Usage (React)

```tsx
import { MtwProvider, useAgent, useChannel, useStream } from '@mtw/react'

function App() {
  return (
    <MtwProvider url="ws://localhost:8080/ws" auth={{ token: "..." }}>
      <Chat />
    </MtwProvider>
  )
}

function Chat() {
  // AI Agent interaction
  const { send, messages, isStreaming } = useAgent("assistant")

  // Real-time channel
  const { members, publish } = useChannel("chat.general")

  // Raw streaming
  const { data, subscribe } = useStream("3d-sync")

  return (
    <div>
      {messages.map(msg => <Message key={msg.id} {...msg} />)}

      <input onSubmit={(text) => send(text)} />

      {isStreaming && <TypingIndicator />}
    </div>
  )
}
```

---

## Marketplace Architecture

```
┌─────────────────────────────────────────────┐
│           marketplace.mtw.dev               │
│                                             │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐ │
│  │ Registry │  │ Search   │  │ Analytics │ │
│  │ API      │  │ (Meilisearch)│ (downloads│ │
│  │ (Rust)   │  │          │  │  ratings) │ │
│  └────┬─────┘  └──────────┘  └───────────┘ │
│       │                                     │
│  ┌────▼─────────────────────────────────┐   │
│  │  Storage (S3/R2)                     │   │
│  │  - WASM modules (sandboxed)          │   │
│  │  - Source packages (Rust crates)     │   │
│  │  - npm packages (UI components)      │   │
│  └──────────────────────────────────────┘   │
│                                             │
│  ┌──────────────────────────────────────┐   │
│  │  Verification Pipeline               │   │
│  │  - Security scan                     │   │
│  │  - Permission audit                  │   │
│  │  - Build verification                │   │
│  │  - WASM sandbox testing              │   │
│  └──────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

### Module Distribution Formats

| Format | When | How |
|--------|------|-----|
| **Rust crate** | server modules, high perf | `mtw add <name>` (compiles) |
| **WASM binary** | sandboxed/untrusted modules | `mtw add <name> --wasm` (pre-compiled) |
| **npm package** | frontend UI components | auto-installed with server module |
| **Python wheel** | Python binding modules | `pip install mtw-<name>` |

### WASM Sandbox for Third-Party Modules

Untrusted modules run in a WASM sandbox (wasmtime) with explicit permissions:

```toml
# mtw-module.toml
[permissions]
network = ["api.example.com"]    # only allowed outbound host
filesystem = false
environment = ["API_KEY"]        # only this env var
```

The runtime enforces these — a module that tries to access anything not in its
permissions list gets an error, not access.

---

## Roadmap

### Phase 1 — Foundation
- [ ] mtw-core: module loader, lifecycle, config
- [ ] mtw-protocol: wire format
- [ ] mtw-transport: WebSocket (tokio-tungstenite)
- [ ] mtw-router: channels, rooms, middleware chain
- [ ] mtw-codec: JSON
- [ ] mtw-cli: init, dev
- [ ] @mtw/client: JS WebSocket client

### Phase 2 — AI & Auth
- [ ] mtw-ai: provider trait, streaming
- [ ] mtw-ai-anthropic: Claude provider
- [ ] mtw-ai-openai: OpenAI provider
- [ ] mtw-auth: JWT, API keys
- [ ] mtw-state: in-memory store
- [ ] Agent system: task routing, tool calling
- [ ] @mtw/react: hooks

### Phase 3 — Ecosystem
- [ ] mtw-registry: marketplace client
- [ ] mtw-sdk: proc macros for module devs
- [ ] mtw-test: testing harness
- [ ] WASM sandbox for untrusted modules
- [ ] Registry backend + web UI
- [ ] `mtw publish` flow

### Phase 4 — Bindings & Scale
- [ ] Node.js binding (NAPI-RS)
- [ ] Python binding (PyO3)
- [ ] PHP binding (FFI)
- [ ] WASM binding for browser
- [ ] @mtw/svelte, @mtw/vue
- [ ] Multi-node clustering
- [ ] QUIC transport
