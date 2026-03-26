# API Reference

Complete reference for all public traits, structs, enums, and functions organized by crate.

---

## mtw-protocol

Wire protocol definitions. Source: `crates/mtw-protocol/src/`

### MtwMessage

The core wire message format.

```rust
pub struct MtwMessage {
    pub id: String,                              // ULID
    pub msg_type: MsgType,                       // Serialized as "type"
    pub channel: Option<String>,
    pub payload: Payload,
    pub metadata: HashMap<String, Value>,
    pub timestamp: u64,                          // Unix ms
    pub ref_id: Option<String>,
}
```

**Factory methods:**
| Method | Returns |
|--------|---------|
| `MtwMessage::new(msg_type, payload)` | New message with ULID and timestamp |
| `MtwMessage::event(text)` | Event with text payload |
| `MtwMessage::request(payload)` | Request message |
| `MtwMessage::response(ref_id, payload)` | Response correlated to a request |
| `MtwMessage::error(code, message)` | Error message with JSON payload |
| `MtwMessage::agent_task(agent, content)` | Agent task with agent name in metadata |
| `MtwMessage::stream_chunk(ref_id, text)` | Streaming chunk |
| `MtwMessage::stream_end(ref_id)` | Stream end marker |

**Builder methods:**
| Method | Description |
|--------|-------------|
| `.with_channel(name)` | Set target channel |
| `.with_ref(ref_id)` | Set reference ID |
| `.with_metadata(key, value)` | Add metadata entry |

### MsgType

```rust
pub enum MsgType {
    Connect, Disconnect, Ping, Pong,
    Request, Response, Event, Stream, StreamEnd,
    Subscribe, Unsubscribe, Publish,
    AgentTask, AgentChunk, AgentToolCall, AgentToolResult, AgentComplete,
    Error, Ack,
}
```

### Payload

```rust
pub enum Payload {              // Serialized with tag="kind", content="data"
    None,
    Text(String),
    Json(serde_json::Value),
    Binary(Vec<u8>),            // base64-encoded on wire
}
```

**Methods:** `as_text()`, `as_json()`, `as_binary()`, `is_none()`

### ConnId

```rust
pub type ConnId = String;
```

### ConnMetadata

```rust
pub struct ConnMetadata {
    pub conn_id: ConnId,
    pub remote_addr: Option<String>,
    pub user_agent: Option<String>,
    pub auth: Option<AuthInfo>,
    pub connected_at: u64,
}
```

### AuthInfo

```rust
pub struct AuthInfo {
    pub user_id: Option<String>,
    pub token: Option<String>,
    pub roles: Vec<String>,
    pub claims: HashMap<String, Value>,
}
```

### DisconnectReason

```rust
pub enum DisconnectReason {
    Normal, Timeout, Error(String), Kicked(String), ServerShutdown,
}
```

### TransportEvent

```rust
pub enum TransportEvent {
    Connected(ConnId, ConnMetadata),
    Disconnected(ConnId, DisconnectReason),
    Message(ConnId, MtwMessage),
    Binary(ConnId, Vec<u8>),
    Error(ConnId, String),
}
```

### Frame (binary framing)

```rust
pub struct Frame;
```

| Method | Description |
|--------|-------------|
| `Frame::encode_message(msg)` | MtwMessage -> Bytes |
| `Frame::encode_binary(data)` | &[u8] -> Bytes |
| `Frame::encode_ping()` | Ping frame |
| `Frame::encode_pong()` | Pong frame |
| `Frame::decode(data)` | Bytes -> (FrameType, Bytes) |
| `Frame::decode_message(data)` | Bytes -> MtwMessage |

**Constants:** `PROTOCOL_VERSION = 1`, `MAX_FRAME_SIZE = 10MB`

---

## mtw-core

Kernel: module system, configuration, hooks, server. Source: `crates/mtw-core/src/`

### MtwModule (trait)

```rust
#[async_trait]
pub trait MtwModule: Send + Sync {
    fn manifest(&self) -> &ModuleManifest;
    async fn on_load(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;
    async fn on_start(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;
    async fn on_stop(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;
    async fn health(&self) -> HealthStatus;   // default: Healthy
}
```

### ModuleManifest

```rust
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub module_type: ModuleType,
    pub description: String,
    pub author: String,
    pub license: String,
    pub repository: Option<String>,
    pub dependencies: Vec<ModuleDep>,
    pub config_schema: Option<Value>,
    pub permissions: Vec<Permission>,
    pub minimum_core: Option<String>,
}
```

### ModuleType

```rust
pub enum ModuleType {
    Transport, Middleware, AIProvider, AIAgent, Codec,
    Auth, Storage, Channel, Integration, UI,
}
```

### Permission

```rust
pub enum Permission {
    Network, FileSystem, Environment, Subprocess, Database, Custom(String),
}
```

### HealthStatus

```rust
pub enum HealthStatus {
    Healthy, Degraded(String), Unhealthy(String),
}
```

### ModuleContext

```rust
pub struct ModuleContext {
    pub config: serde_json::Value,
    pub shared: Arc<SharedState>,
}
```

### SharedState

```rust
pub struct SharedState { /* DashMap<String, Value> */ }
```

| Method | Description |
|--------|-------------|
| `new()` | Create empty state |
| `get(key)` | Get a value |
| `set(key, value)` | Set a value |
| `remove(key)` | Remove a value |

### ModuleRegistry

| Method | Description |
|--------|-------------|
| `new()` | Create empty registry |
| `register(module)` | Register (rejects duplicates) |
| `load_all(ctx)` | Call on_load on all modules |
| `start_all(ctx)` | Call on_start on all modules |
| `stop_all(ctx)` | Call on_stop in reverse order |
| `get(name)` | Get module by name |
| `list()` | List registered names |
| `health_check()` | Check all modules |

### MtwConfig

```rust
pub struct MtwConfig {
    pub server: ServerConfig,
    pub transport: TransportConfig,
    pub codec: CodecConfig,
    pub modules: Vec<ModuleEntry>,
    pub agents: Vec<AgentEntry>,
    pub channels: Vec<ChannelConfig>,
    pub orchestrator: Option<OrchestratorConfig>,
}
```

| Method | Description |
|--------|-------------|
| `from_file(path)` | Load from TOML file |
| `from_str(content)` | Parse TOML string |
| `default_config()` | All defaults |

### LifecycleHooks (trait)

```rust
#[async_trait]
pub trait LifecycleHooks: Send + Sync {
    async fn on_connect(&self, conn_id: &ConnId, meta: &ConnMetadata) -> Result<(), MtwError>;
    async fn on_disconnect(&self, conn_id: &ConnId, reason: &DisconnectReason) -> Result<(), MtwError>;
    async fn before_message(&self, conn_id: &ConnId, msg: MtwMessage) -> Result<Option<MtwMessage>, MtwError>;
    async fn after_message(&self, conn_id: &ConnId, msg: &MtwMessage) -> Result<(), MtwError>;
    async fn on_error(&self, conn_id: Option<&ConnId>, error: &MtwError);
}
```

### HookRegistry

| Method | Description |
|--------|-------------|
| `new()` | Create empty |
| `register(hook)` | Add a hook implementation |
| `on_connect(...)` | Dispatch to all hooks |
| `on_disconnect(...)` | Dispatch to all hooks |
| `before_message(...)` | Chain through all (None = reject) |
| `after_message(...)` | Dispatch to all hooks |
| `on_error(...)` | Dispatch to all hooks |

### MtwServer

| Method | Description |
|--------|-------------|
| `new(config)` | Create from config |
| `from_config_file(path)` | Create from TOML file |
| `module(m)` | Register module (builder-style) |
| `hooks()` | Get HookRegistry |
| `shared()` | Get SharedState |
| `config()` | Get config |
| `start()` | Load + start all modules |
| `shutdown()` | Stop all modules |
| `run()` | Start + wait for Ctrl+C + shutdown |

### MtwServerBuilder

| Method | Description |
|--------|-------------|
| `new()` | Default config |
| `config_file(path)` | Load TOML |
| `config(cfg)` | Set config |
| `port(p)` | Set port |
| `host(h)` | Set host |
| `module(m)` | Add module |
| `build()` | Build MtwServer |

---

## mtw-transport

Transport abstraction. Source: `crates/mtw-transport/src/`

### MtwTransport (trait)

```rust
#[async_trait]
pub trait MtwTransport: Send + Sync {
    fn name(&self) -> &str;
    async fn listen(&mut self, addr: SocketAddr) -> Result<(), MtwError>;
    async fn send(&self, conn_id: &ConnId, msg: MtwMessage) -> Result<(), MtwError>;
    async fn send_binary(&self, conn_id: &ConnId, data: &[u8]) -> Result<(), MtwError>;
    async fn broadcast(&self, msg: MtwMessage) -> Result<(), MtwError>;
    async fn close(&self, conn_id: &ConnId) -> Result<(), MtwError>;
    fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<TransportEvent>>;
    fn connection_count(&self) -> usize;
    fn has_connection(&self, conn_id: &ConnId) -> bool;
    async fn shutdown(&self) -> Result<(), MtwError>;
}
```

### WebSocketTransport

```rust
pub struct WebSocketTransport { /* ... */ }
```

| Method | Description |
|--------|-------------|
| `new(path, ping_interval)` | Create with path and ping interval (secs) |

Implements `MtwTransport`. Uses `tokio-tungstenite` for WebSocket handling.

---

## mtw-router

Message routing, channels, middleware. Source: `crates/mtw-router/src/`

### MtwMiddleware (trait)

```rust
#[async_trait]
pub trait MtwMiddleware: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> i32;  // default: 100
    async fn on_inbound(&self, msg: MtwMessage, ctx: &MiddlewareContext) -> Result<MiddlewareAction, MtwError>;
    async fn on_outbound(&self, msg: MtwMessage, ctx: &MiddlewareContext) -> Result<MiddlewareAction, MtwError>;
}
```

### MiddlewareAction

```rust
pub enum MiddlewareAction {
    Continue(MtwMessage),
    Halt,
    Transform(MtwMessage),
    Redirect { channel: String, msg: MtwMessage },
}
```

### MiddlewareChain

| Method | Description |
|--------|-------------|
| `new()` | Create empty chain |
| `add(middleware)` | Add and sort by priority |
| `process_inbound(msg, ctx)` | Run inbound chain |
| `process_outbound(msg, ctx)` | Run outbound chain (reverse) |
| `len()`, `is_empty()` | Size queries |

### Channel

| Method | Description |
|--------|-------------|
| `new(name, auth, max, history, tx)` | Create |
| `name()` | Get name |
| `subscribe(conn_id)` | Subscribe |
| `unsubscribe(conn_id)` | Unsubscribe |
| `is_subscribed(conn_id)` | Check |
| `publish(msg, exclude)` | Publish to all |
| `get_history(limit)` | Get history |
| `subscribers()` | List subscriber IDs |
| `subscriber_count()` | Count |
| `remove_connection(conn_id)` | Cleanup |

### ChannelManager

| Method | Description |
|--------|-------------|
| `new()` | Create |
| `create_channel(name, auth, max, history)` | Create channel |
| `get_or_create(name)` | Get or create with defaults |
| `get(name)` | Get by exact name |
| `find_matching(pattern)` | Glob match |
| `subscribe(channel, conn_id)` | Subscribe |
| `unsubscribe(channel, conn_id)` | Unsubscribe |
| `remove_connection(conn_id)` | Remove from all |
| `list_channels()` | List all |
| `delete_channel(name)` | Delete |

---

## mtw-ai

AI providers, agents, orchestration. Source: `crates/mtw-ai/src/`

### MtwAIProvider (trait)

```rust
#[async_trait]
pub trait MtwAIProvider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError>;
    fn stream(&self, req: CompletionRequest) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>>;
    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError>;
}
```

### Key Structs

| Struct | Description |
|--------|-------------|
| `ProviderCapabilities` | streaming, tool_calling, vision, embeddings, max_context |
| `Message` | role (System/User/Assistant/Tool), content |
| `CompletionRequest` | model, messages, tools, temperature, max_tokens, metadata |
| `CompletionResponse` | id, model, content, tool_calls, usage, finish_reason |
| `StreamChunk` | delta, tool_calls, finish_reason, usage |
| `ToolDef` | name, description, parameters (JSON Schema) |
| `ToolCall` | id, name, arguments |
| `ToolResult` | tool_call_id, name, result, is_error |
| `ModelInfo` | id, name, max_context, supports_tools, supports_vision |

### MtwAgent (trait)

```rust
#[async_trait]
pub trait MtwAgent: Send + Sync {
    fn description(&self) -> &AgentDescription;
    async fn handle(&self, task: AgentTask, ctx: &AgentContext) -> Result<AgentResponse, MtwError>;
    fn handle_stream(&self, task: AgentTask, ctx: &AgentContext) -> Pin<Box<dyn Stream<...> + Send>>;
    fn tools(&self) -> Vec<ToolDef>;
    async fn on_tool_result(&self, result: ToolResult, ctx: &AgentContext) -> Result<AgentResponse, MtwError>;
}
```

### Key Agent Structs

| Struct | Description |
|--------|-------------|
| `AgentDescription` | name, role, capabilities, accepts, max_concurrent |
| `AgentTask` | id, from, channel, content, context, metadata |
| `AgentContent` | Text, Structured, Binary, Multi |
| `AgentContext` | metadata |
| `AgentResponse` | content, tool_calls, done, metadata |
| `AgentChunk` | delta, tool_calls, done |

### AgentOrchestrator

| Method | Description |
|--------|-------------|
| `new(strategy)` | Create with routing strategy |
| `register_agent(agent)` | Register an agent |
| `remove_agent(name)` | Remove by name |
| `get_agent(name)` | Get by name |
| `agent_names()` | List all names |
| `route(task, ctx)` | Route task to agent(s) |
| `route_stream(task, ctx)` | Route with streaming |

### RoutingStrategy

```rust
pub enum RoutingStrategy {
    ChannelBased,
    Pipeline(Vec<String>),
    FanOut,
    RoundRobin,
}
```

---

## mtw-auth

Authentication. Source: `crates/mtw-auth/src/`

### MtwAuth (trait)

```rust
#[async_trait]
pub trait MtwAuth: Send + Sync {
    fn name(&self) -> &str;
    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken, MtwError>;
    async fn validate(&self, token: &str) -> Result<AuthClaims, MtwError>;
    async fn refresh(&self, token: &str) -> Result<AuthToken, MtwError>;
}
```

### Key Structs

| Struct | Description |
|--------|-------------|
| `Credentials` | Token(String), ApiKey(String), Basic{username, password} |
| `AuthToken` | token, token_type, expires_at, refresh_token |
| `AuthClaims` | sub, iat, exp, roles, custom |
| `JwtConfig` | secret, algorithm, expiration_secs, refresh_expiration_secs, issuer, audience |
| `JwtAuth` | JWT implementation of MtwAuth |
| `AuthMiddleware` | Middleware that validates tokens on inbound messages |

---

## mtw-state

State store. Source: `crates/mtw-state/src/`

### MtwStateStore (trait)

```rust
#[async_trait]
pub trait MtwStateStore: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<Value>, MtwError>;
    async fn set(&self, key: &str, value: Value) -> Result<(), MtwError>;
    async fn delete(&self, key: &str) -> Result<bool, MtwError>;
    async fn exists(&self, key: &str) -> Result<bool, MtwError>;
    async fn keys(&self, pattern: &str) -> Result<Vec<String>, MtwError>;
    async fn ttl_set(&self, key: &str, value: Value, ttl_secs: u64) -> Result<(), MtwError>;
}
```

---

## mtw-codec

Serialization codecs. Source: `crates/mtw-codec/src/`

### MtwCodec (trait)

```rust
#[async_trait]
pub trait MtwCodec: Send + Sync {
    fn name(&self) -> &str;
    fn encode(&self, msg: &MtwMessage) -> Result<Bytes, MtwError>;
    fn decode(&self, data: &[u8]) -> Result<MtwMessage, MtwError>;
    fn content_type(&self) -> &str;
}
```

### CodecRegistry

| Method | Description |
|--------|-------------|
| `new(default)` | Create with default codec name |
| `register(codec)` | Register a codec |
| `get(name)` | Get by name |
| `default_codec()` | Get the default codec |

---

## mtw-http

HTTP client with pipeline. Source: `crates/mtw-http/src/`

### PipelineStage (trait)

```rust
#[async_trait]
pub trait PipelineStage: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> i32;  // default: 100
    async fn process(&self, response: MtwResponse, context: &mut PipelineContext) -> Result<PipelineAction, MtwError>;
}
```

### PipelineAction

```rust
pub enum PipelineAction {
    Continue(MtwResponse), Retry(MtwRequest), Error(MtwError), Cached(MtwResponse),
}
```

### Key Structs

| Struct | Description |
|--------|-------------|
| `MtwRequest` | method, url, headers, query, body, timeout, metadata |
| `MtwResponse` | status, headers, body, timing, metadata, rate_limit, pagination, cache_info |
| `ResponsePipeline` | Ordered chain of stages |
| `PipelineContext` | request, attempt, max_retries, metadata |
| `AuthStrategy` | Bearer, Basic, ApiKey, OAuth2, Custom |

---

## mtw-sdk

Module developer SDK. Source: `crates/mtw-sdk/src/`

### Prelude (re-exports)

```rust
use mtw_sdk::prelude::*;
// Imports: MtwModule, ModuleManifest, ModuleType, Permission, HealthStatus,
//   ModuleContext, MtwError, MtwMessage, MsgType, Payload, ConnId, TransportEvent,
//   MtwMiddleware, MiddlewareAction, MiddlewareContext, MtwCodec,
//   ModuleManifestBuilder, create_manifest, default_manifest,
//   async_trait, Serialize, Deserialize, HashMap, Arc
```

### ModuleManifestBuilder

Fluent builder for `ModuleManifest`. Methods: `name()`, `version()`, `module_type()`, `description()`, `author()`, `license()`, `repository()`, `dependency()`, `optional_dependency()`, `config_schema()`, `permission()`, `minimum_core()`, `build()`.

### Helper Functions

| Function | Description |
|----------|-------------|
| `create_manifest(name, version, type)` | Quick manifest creation |
| `default_manifest(name)` | Builder with defaults (v0.1.0, Middleware, MIT) |

---

## mtw-registry

Module marketplace. Source: `crates/mtw-registry/src/`

### RegistryManifest

Parsed from `mtw-module.toml`.

| Method | Description |
|--------|-------------|
| `from_toml(str)` | Parse TOML string |
| `from_file(path)` | Parse file |
| `validate()` | Validate fields |
| `to_module_manifest()` | Convert to core ModuleManifest |

### PermissionSet

```rust
pub struct PermissionSet {
    pub network: bool,
    pub filesystem: bool,
    pub environment: bool,
    pub subprocess: bool,
}
```

| Method | Description |
|--------|-------------|
| `to_permissions()` | Convert to Vec<Permission> |

---

## mtw-integrations

Third-party integrations. Source: `crates/mtw-integrations/src/`

### OAuth2Client

| Method | Description |
|--------|-------------|
| `new(config)` | Create from OAuth2Config |
| `config()` | Get config reference |
| `authorization_url(state)` | Generate auth URL |
| `exchange_code(code)` | Exchange code for token |
| `refresh_token(token)` | Refresh expired token |
| `current_token()` | Get stored token |
| `set_token(token)` | Store a token |

### Pre-configured OAuth2

`github_oauth2()`, `gitlab_oauth2()`, `slack_oauth2()`, `discord_oauth2()`, `stripe_oauth2()`, `paypal_oauth2()`, `google_oauth2()`, `notion_oauth2()`, `airtable_oauth2()`, `jira_oauth2()`, `linear_oauth2()`, `vercel_oauth2()`

### AiProviderConfig

```rust
pub struct AiProviderConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: Option<f32>,
    pub default_max_tokens: Option<u32>,
    pub timeout_secs: u64,            // default: 120
}
```

### IntegrationInfo

```rust
pub struct IntegrationInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub docs_url: &'static str,
    pub oauth2_supported: bool,
}
```
