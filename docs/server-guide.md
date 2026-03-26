# Server Guide

This guide covers configuring and running an mtwRequest server, including the TOML configuration format, the builder pattern API, lifecycle hooks, and graceful shutdown.

---

## Server Configuration (mtw.toml)

The server reads its configuration from a TOML file, typically named `mtw.toml`. Every section has sensible defaults, so you can start with an empty file and override only what you need.

### Complete Annotated Configuration

```toml
# =============================================================================
# Server settings
# =============================================================================
[server]
host = "0.0.0.0"           # Bind address (default: "0.0.0.0")
port = 8080                 # Listen port (default: 8080)
max_connections = 10000     # Maximum concurrent connections (default: 10000)

# =============================================================================
# Transport layer
# =============================================================================
[transport]
default = "websocket"       # Default transport (default: "websocket")

[transport.websocket]
path = "/ws"                # WebSocket endpoint path (default: "/ws")
ping_interval = 30          # Ping interval in seconds (default: 30)
max_message_size = "10MB"   # Max message size (default: "10MB")

[transport.http]
enabled = false             # Enable HTTP/REST transport (default: false)
prefix = "/api"             # HTTP route prefix (default: "/api")

# =============================================================================
# Codec (serialization)
# =============================================================================
[codec]
default = "json"            # Default codec (default: "json")
binary_channels = [         # Channels that use binary codec instead of JSON
    "3d-sync",
    "audio",
]

# =============================================================================
# Modules
# =============================================================================
# Each [[modules]] entry loads a module. Config values support ${ENV_VAR}
# expansion for secrets.

[[modules]]
name = "mtw-auth-jwt"
version = "1.2"
enabled = true                                    # Optional, default: true
config = { secret = "${JWT_SECRET}", expiration = 7200 }

[[modules]]
name = "mtw-ai-anthropic"
version = "0.5"
config = { api_key = "${ANTHROPIC_API_KEY}", default_model = "claude-sonnet-4-6" }

[[modules]]
name = "mtw-store-redis"
version = "1.0"
config = { url = "${REDIS_URL}" }

# =============================================================================
# AI Agents
# =============================================================================

[[agents]]
name = "assistant"
provider = "mtw-ai-anthropic"
model = "claude-sonnet-4-6"
system = "You are a helpful assistant."
tools = ["search", "calculator"]
channels = ["chat.*"]           # Handles messages on chat.* channels
max_concurrent = 10             # Max concurrent tasks (optional)

[[agents]]
name = "code-reviewer"
provider = "mtw-ai-anthropic"
model = "claude-opus-4-6"
system = "You are a senior code reviewer."
channels = ["code-review"]

# =============================================================================
# Agent Orchestration
# =============================================================================
[orchestrator]
strategy = "channel-based"      # "channel-based", "pipeline", "fan-out", "round-robin"

# =============================================================================
# Channels
# =============================================================================

[[channels]]
name = "chat.*"                 # Supports glob patterns
auth = true                     # Require authentication (default: false)
max_members = 100               # Max subscribers (default: unlimited)
history = 50                    # Messages to keep in history (default: 0)

[[channels]]
name = "3d-sync"
codec = "binary"                # Override the default codec for this channel
auth = true

[[channels]]
name = "notifications"
history = 10
```

### Configuration Defaults

| Setting | Default |
|---------|---------|
| `server.host` | `"0.0.0.0"` |
| `server.port` | `8080` |
| `server.max_connections` | `10000` |
| `transport.default` | `"websocket"` |
| `transport.websocket.path` | `"/ws"` |
| `transport.websocket.ping_interval` | `30` (seconds) |
| `transport.websocket.max_message_size` | `"10MB"` |
| `transport.http.enabled` | `false` |
| `transport.http.prefix` | `"/api"` |
| `codec.default` | `"json"` |

---

## Environment Variable Expansion

String values in `mtw.toml` support `${ENV_VAR}` syntax for environment variable expansion. This is critical for secrets:

```toml
[[modules]]
name = "mtw-auth-jwt"
config = { secret = "${JWT_SECRET}" }
```

If `JWT_SECRET` is set to `"my-signing-key"`, the config will resolve to `secret = "my-signing-key"`. If the variable is not set, it resolves to an empty string.

The expansion happens at parse time inside `MtwConfig::from_str()` (see `crates/mtw-core/src/config.rs`).

---

## Creating a Server Programmatically

### Using MtwConfig directly

```rust
use mtw_core::config::MtwConfig;
use mtw_core::server::MtwServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load from file
    let config = MtwConfig::from_file("mtw.toml")?;
    let mut server = MtwServer::new(config);

    // Or parse from a string
    let config = MtwConfig::from_str(r#"
        [server]
        port = 3000
    "#)?;
    let mut server = MtwServer::new(config);

    server.run().await?;
    Ok(())
}
```

### Using the Builder Pattern

The `MtwServerBuilder` provides a fluent API for server construction:

```rust
use mtw_core::server::MtwServerBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = MtwServerBuilder::new()
        .host("127.0.0.1")
        .port(3000)
        // .config_file("mtw.toml")?   // Load full config from file
        // .module(Box::new(my_module)) // Register a module
        .build()?;

    server.run().await?;
    Ok(())
}
```

**Builder methods:**

| Method | Description |
|--------|-------------|
| `new()` | Create with default config |
| `config_file(path)` | Load config from TOML file |
| `config(cfg)` | Set config directly |
| `host(addr)` | Set bind address |
| `port(port)` | Set listen port |
| `module(m)` | Register a module |
| `build()` | Build the `MtwServer` |

Source: `crates/mtw-core/src/server.rs`

---

## Loading Modules

Modules are registered before the server starts. Each module must implement the `MtwModule` trait:

```rust
use mtw_core::server::MtwServerBuilder;

let server = MtwServerBuilder::new()
    .module(Box::new(my_auth_module))
    .module(Box::new(my_rate_limiter))
    .module(Box::new(my_ai_provider))
    .build()?;
```

The `ModuleRegistry` (in `crates/mtw-core/src/module.rs`) enforces:

- **No duplicate names** -- registering a module with the same name twice returns an error
- **Ordered loading** -- modules are loaded in registration order
- **Reverse-order stopping** -- modules are stopped in reverse order during shutdown

---

## Lifecycle Hooks

The `LifecycleHooks` trait (in `crates/mtw-core/src/hooks.rs`) lets you intercept connection and message events:

```rust
use async_trait::async_trait;
use mtw_core::hooks::LifecycleHooks;
use mtw_core::MtwError;
use mtw_protocol::{ConnId, ConnMetadata, DisconnectReason, MtwMessage};

struct MyHooks;

#[async_trait]
impl LifecycleHooks for MyHooks {
    async fn on_connect(&self, conn_id: &ConnId, meta: &ConnMetadata) -> Result<(), MtwError> {
        tracing::info!(conn = %conn_id, addr = ?meta.remote_addr, "user connected");
        Ok(())
    }

    async fn on_disconnect(&self, conn_id: &ConnId, reason: &DisconnectReason) -> Result<(), MtwError> {
        tracing::info!(conn = %conn_id, reason = ?reason, "user disconnected");
        Ok(())
    }

    async fn before_message(
        &self,
        conn_id: &ConnId,
        msg: MtwMessage,
    ) -> Result<Option<MtwMessage>, MtwError> {
        // Return None to reject the message
        // Return Some(msg) to allow it (optionally transformed)
        tracing::debug!(conn = %conn_id, msg_type = ?msg.msg_type, "processing message");
        Ok(Some(msg))
    }

    async fn after_message(&self, _conn_id: &ConnId, _msg: &MtwMessage) -> Result<(), MtwError> {
        Ok(())
    }

    async fn on_error(&self, conn_id: Option<&ConnId>, error: &MtwError) {
        tracing::error!(conn = ?conn_id, error = %error, "error occurred");
    }
}
```

Register hooks with the `HookRegistry`:

```rust
let server = MtwServer::new(config);
server.hooks().register(Arc::new(MyHooks)).await;
```

The `HookRegistry` chains multiple hook implementations. All registered hooks are called in order. For `before_message`, if any hook returns `None`, the message is rejected.

---

## Graceful Shutdown

The server supports graceful shutdown via `Ctrl+C` or programmatic control:

```rust
// Option 1: run() blocks until Ctrl+C
server.run().await?;

// Option 2: manual start/stop
server.start().await?;
// ... do work ...
server.shutdown().await?;
```

During shutdown:
1. A shutdown signal is broadcast to all WebSocket connections
2. All active connections receive a `ServerShutdown` disconnect reason
3. Modules are stopped in **reverse registration order**
4. The transport layer closes all connections

The WebSocket transport uses `tokio::sync::broadcast` for shutdown signaling, ensuring all connection handler tasks receive the signal and clean up properly.

Source: `crates/mtw-transport/src/ws.rs` (shutdown broadcast), `crates/mtw-core/src/server.rs` (server lifecycle)

---

## Server Methods Reference

| Method | Description |
|--------|-------------|
| `MtwServer::new(config)` | Create server from config |
| `MtwServer::from_config_file(path)` | Create from TOML file |
| `server.module(m)` | Register a module (builder-style, returns `Result<Self>`) |
| `server.hooks()` | Get reference to the `HookRegistry` |
| `server.shared()` | Get reference to the shared state (`SharedState`) |
| `server.config()` | Get reference to the config |
| `server.start()` | Load and start all modules |
| `server.shutdown()` | Stop all modules in reverse order |
| `server.run()` | Start, then wait for `Ctrl+C`, then shutdown |

---

## Complete Example

See `examples/demo_server.rs` for a fully working server with WebSocket transport, channel management, subscribe/unsubscribe/publish handling, and graceful shutdown.

Run it:

```bash
cargo run --example demo_server
```

Then connect with the demo client:

```bash
cargo run --example demo_client
```

---

## Next Steps

- [Modules Guide](./modules-guide.md) -- create custom modules to extend the server
- [Channels Guide](./channels-guide.md) -- configure pub/sub channels
- [Auth Guide](./auth-guide.md) -- add authentication
