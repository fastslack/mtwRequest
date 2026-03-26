# Modules Guide

Everything in mtwRequest is a module. The authentication layer, AI providers, codecs, transports -- all are modules that implement the same `MtwModule` trait. This guide covers creating, configuring, testing, and publishing your own modules.

---

## What is a Module?

A module is a self-contained unit of functionality that plugs into the mtwRequest runtime. Modules have:

- A **manifest** describing name, version, type, permissions, and dependencies
- A **lifecycle** (on_load, on_start, on_stop) managed by the module registry
- **Configuration** supplied via `mtw.toml` or programmatically
- Optional **additional traits** (MtwMiddleware, MtwAIProvider, MtwAgent, etc.)

---

## Module Types

| Type | Description | Example |
|------|-------------|---------|
| `Transport` | Connection protocol (WebSocket, HTTP, SSE, QUIC) | `mtw-transport-ws` |
| `Middleware` | Request/response pipeline interceptor | Rate limiter, logger |
| `AIProvider` | AI model provider (Anthropic, OpenAI, Ollama) | `mtw-ai-anthropic` |
| `AIAgent` | Pre-built agent behavior | Code reviewer, assistant |
| `Codec` | Message serialization (JSON, MsgPack, Protobuf) | `mtw-codec-msgpack` |
| `Auth` | Authentication strategy (JWT, API keys, OAuth) | `mtw-auth-jwt` |
| `Storage` | Persistent state backend (memory, Redis, Postgres) | `mtw-store-redis` |
| `Channel` | Custom channel logic | Priority channel |
| `Integration` | Third-party service connector | GitHub, Slack, Stripe |
| `UI` | Frontend components (shipped as npm package) | Chat widget |

Source: `crates/mtw-core/src/module.rs` -- `ModuleType` enum

---

## The MtwModule Trait

Every module implements this trait, defined in `crates/mtw-core/src/module.rs`:

```rust
#[async_trait]
pub trait MtwModule: Send + Sync {
    /// Return the module's manifest (name, version, type, etc.)
    fn manifest(&self) -> &ModuleManifest;

    /// Called when the module is loaded into the runtime.
    /// Use this to initialize resources, validate config, set up connections.
    async fn on_load(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Called when the server starts accepting connections.
    /// Use this to start background tasks, open listeners.
    async fn on_start(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Called when the server is shutting down.
    /// Use this to flush buffers, close connections, clean up.
    async fn on_stop(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Health check -- override for custom health reporting.
    /// Default: returns HealthStatus::Healthy
    async fn health(&self) -> HealthStatus {
        HealthStatus::Healthy
    }
}
```

The `ModuleContext` provides:
- `config: serde_json::Value` -- the module's configuration from `mtw.toml`
- `shared: Arc<SharedState>` -- shared key-value state accessible to all modules

---

## Creating Your First Module (Step by Step)

### Step 1: Create the crate

```bash
cargo new my-rate-limiter --lib
cd my-rate-limiter
```

Add dependencies to `Cargo.toml`:

```toml
[dependencies]
mtw-sdk = "0.1"
mtw-core = "0.1"
mtw-protocol = "0.1"
mtw-router = "0.1"
async-trait = "0.1"
dashmap = "6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

### Step 2: Define the module struct and config

```rust
use mtw_sdk::prelude::*;
use dashmap::DashMap;
use std::time::Instant;

/// Configuration for the rate limiter
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimiterConfig {
    /// Maximum requests per window
    pub max_requests: u32,
    /// Window duration in seconds (default: 60)
    #[serde(default = "default_window")]
    pub window_secs: u64,
}

fn default_window() -> u64 { 60 }

/// Rate limiter module
pub struct RateLimiter {
    manifest: ModuleManifest,
    config: RateLimiterConfig,
    counters: DashMap<ConnId, (u32, Instant)>,
}

impl RateLimiter {
    pub fn new(config: RateLimiterConfig) -> Self {
        let manifest = ModuleManifestBuilder::new()
            .name("my-rate-limiter")
            .version("0.1.0")
            .module_type(ModuleType::Middleware)
            .description("Per-connection rate limiting")
            .author("your-name")
            .license("MIT")
            .build()
            .unwrap();

        Self {
            manifest,
            config,
            counters: DashMap::new(),
        }
    }
}
```

### Step 3: Implement MtwModule

```rust
#[async_trait]
impl MtwModule for RateLimiter {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    async fn on_load(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
        tracing::info!(
            max = self.config.max_requests,
            window = self.config.window_secs,
            "rate limiter loaded"
        );
        Ok(())
    }

    async fn on_start(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
        Ok(())
    }

    async fn on_stop(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
        self.counters.clear();
        Ok(())
    }

    async fn health(&self) -> HealthStatus {
        HealthStatus::Healthy
    }
}
```

### Step 4: Implement MtwMiddleware

```rust
#[async_trait]
impl MtwMiddleware for RateLimiter {
    fn name(&self) -> &str {
        "rate-limiter"
    }

    fn priority(&self) -> i32 {
        20  // Run early in the chain
    }

    async fn on_inbound(
        &self,
        msg: MtwMessage,
        ctx: &MiddlewareContext,
    ) -> Result<MiddlewareAction, MtwError> {
        let now = Instant::now();

        let mut entry = self.counters
            .entry(ctx.conn_id.clone())
            .or_insert((0, now));

        // Reset window if expired
        if now.duration_since(entry.1).as_secs() > self.config.window_secs {
            *entry = (1, now);
            return Ok(MiddlewareAction::Continue(msg));
        }

        entry.0 += 1;

        if entry.0 > self.config.max_requests {
            tracing::warn!(conn = %ctx.conn_id, "rate limited");
            return Ok(MiddlewareAction::Halt);
        }

        Ok(MiddlewareAction::Continue(msg))
    }
}
```

### Step 5: Register with the server

```rust
use mtw_router::middleware::MiddlewareChain;

let mut middleware = MiddlewareChain::new();
middleware.add(Arc::new(RateLimiter::new(RateLimiterConfig {
    max_requests: 100,
    window_secs: 60,
})));
```

---

## Module Manifest (mtw-module.toml)

Every publishable module ships with a `mtw-module.toml` file:

```toml
[module]
name = "mtw-auth-jwt"
version = "1.0.0"
type = "auth"                      # One of: transport, middleware, ai_provider,
                                   #   ai_agent, codec, auth, storage, channel,
                                   #   integration, ui
description = "JWT authentication for mtwRequest"
author = "fastslack"
license = "MIT"
repository = "https://github.com/fastslack/mtw-auth-jwt"
minimum_core = "0.1.0"

[permissions]
network = false                    # Can make outbound HTTP requests
filesystem = false                 # Can read/write files
environment = true                 # Can read environment variables
subprocess = false                 # Can spawn processes

[dependencies]
mtw-core = "0.1"
mtw-redis = { version = "0.2", optional = true }

# Optional: JSON Schema for the module's config
[config]
# ... JSON Schema properties ...

# Optional: companion npm package
[ui]
package = "@mtw/auth-jwt-react"
version = "1.0.0"
```

The manifest is parsed by `RegistryManifest::from_toml()` in `crates/mtw-registry/src/manifest.rs`. It validates:
- Non-empty name and version
- Valid semver version format
- Valid module type string

---

## Module Configuration and Config Schema

Modules receive their configuration through the `ModuleContext.config` field as a `serde_json::Value`. This value comes from the `config` map in the `[[modules]]` section of `mtw.toml`:

```toml
[[modules]]
name = "my-rate-limiter"
version = "0.1.0"
config = { max_requests = 100, window_secs = 60 }
```

You can define a JSON Schema in your manifest to document and validate the config:

```toml
[config.properties.max_requests]
type = "integer"
description = "Maximum requests per window"
required = true

[config.properties.window_secs]
type = "integer"
description = "Window duration in seconds"
default = 60
```

---

## Module Permissions

Permissions declare what resources a module needs. They are enforced when running in WASM sandbox mode:

| Permission | Description |
|-----------|-------------|
| `Network` | Can make outbound HTTP/TCP connections |
| `FileSystem` | Can read/write files |
| `Environment` | Can read environment variables |
| `Subprocess` | Can spawn child processes |
| `Database` | Can connect to databases |
| `Custom(String)` | Module-defined permission |

```rust
let manifest = ModuleManifestBuilder::new()
    .name("my-module")
    .version("1.0.0")
    .module_type(ModuleType::Integration)
    .permission(Permission::Network)
    .permission(Permission::Environment)
    .build()?;
```

---

## Module Lifecycle

```
  register()     on_load()       on_start()          on_stop()
      |              |               |                    |
      v              v               v                    v
  +--------+    +--------+    +-----------+    +----------+
  | Module |    | Module |    |  Module   |    |  Module  |
  | stored |    | config |    |  active   |    |  stopped |
  | in     |--->| loaded |--->|  serving  |--->|  cleaned |
  | registry|   | ready  |    |  traffic  |    |  up      |
  +--------+    +--------+    +-----------+    +----------+
```

1. **register()** -- module is added to the `ModuleRegistry`. Duplicate names are rejected.
2. **on_load(ctx)** -- called with the module's config. Validate config, open connections, allocate resources.
3. **on_start(ctx)** -- server is ready. Start background tasks, begin processing.
4. **on_stop(ctx)** -- server is shutting down. Flush buffers, close connections. Called in **reverse** registration order.

If `on_load` or `on_start` returns an error, the server startup fails.

---

## Building Manifests with mtw-sdk

The `mtw-sdk` crate provides convenient builders (from `crates/mtw-sdk/src/builder.rs`):

```rust
use mtw_sdk::prelude::*;

// Quick manifest
let manifest = create_manifest("my-module", "1.0.0", ModuleType::Middleware)?;

// Default manifest with common fields
let manifest = default_manifest("my-module")
    .description("My cool module")
    .author("fastslack")
    .permission(Permission::Network)
    .build()?;

// Full builder
let manifest = ModuleManifestBuilder::new()
    .name("my-module")
    .version("1.0.0")
    .module_type(ModuleType::Auth)
    .description("Custom auth module")
    .author("fastslack")
    .license("MIT")
    .repository("https://github.com/fastslack/my-module")
    .dependency("mtw-core", "^0.1.0")
    .optional_dependency("mtw-redis", "^1.0.0")
    .permission(Permission::Environment)
    .config_schema(serde_json::json!({
        "type": "object",
        "properties": {
            "secret": { "type": "string" }
        }
    }))
    .minimum_core("0.1.0")
    .build()?;
```

---

## Testing Your Module

Use `mtw-test` for module testing (harness planned):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mtw_core::module::{ModuleContext, SharedState};
    use std::sync::Arc;

    fn make_ctx() -> ModuleContext {
        ModuleContext {
            config: serde_json::json!({}),
            shared: Arc::new(SharedState::new()),
        }
    }

    #[tokio::test]
    async fn test_module_lifecycle() {
        let mut module = RateLimiter::new(RateLimiterConfig {
            max_requests: 5,
            window_secs: 60,
        });

        let ctx = make_ctx();
        module.on_load(&ctx).await.unwrap();
        module.on_start(&ctx).await.unwrap();

        // Test middleware behavior
        let msg = MtwMessage::event("hello");
        let mw_ctx = MiddlewareContext {
            conn_id: "test-conn".to_string(),
            channel: None,
        };

        // First 5 requests should pass
        for _ in 0..5 {
            let result = module.on_inbound(msg.clone(), &mw_ctx).await.unwrap();
            assert!(matches!(result, MiddlewareAction::Continue(_)));
        }

        // 6th request should be halted
        let result = module.on_inbound(msg.clone(), &mw_ctx).await.unwrap();
        assert!(matches!(result, MiddlewareAction::Halt));

        module.on_stop(&ctx).await.unwrap();
    }
}
```

---

## Publishing to the Marketplace

```bash
# Ensure your mtw-module.toml is complete
# Build and test
cargo test

# Publish
mtw publish
# > Publishing my-rate-limiter@0.1.0 to mtwRequest marketplace...
# > Published! https://marketplace.mtw.dev/modules/my-rate-limiter
```

### Distribution Formats

| Format | When | How |
|--------|------|-----|
| Rust crate | Server modules, high performance | `mtw add <name>` (compiles locally) |
| WASM binary | Sandboxed/untrusted modules | `mtw add <name> --wasm` (pre-compiled) |
| npm package | Frontend UI components | Auto-installed alongside server module |

---

## Complete Example: Rate Limiter Middleware

The full rate limiter example is shown above in the step-by-step section. Key files:

- Module trait: `crates/mtw-core/src/module.rs`
- Middleware trait: `crates/mtw-router/src/middleware.rs`
- SDK prelude: `crates/mtw-sdk/src/prelude.rs`
- Manifest builder: `crates/mtw-sdk/src/builder.rs`
- Registry manifest parser: `crates/mtw-registry/src/manifest.rs`

---

## Next Steps

- [Protocol Guide](./protocol-guide.md) -- understand the message format your middleware will process
- [Channels Guide](./channels-guide.md) -- learn about the channel system
- [API Reference](./api-reference.md) -- full trait and struct documentation
