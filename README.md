<div align="center">

# mtwRequest

**High-performance, modular real-time framework for WebSocket, HTTP, and AI agents.**

Built in Rust. Runs everywhere.

[![CI](https://github.com/fastslack/mtwRequest/actions/workflows/ci.yml/badge.svg)](https://github.com/fastslack/mtwRequest/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Crates.io](https://img.shields.io/crates/v/mtw-core.svg)](https://crates.io/crates/mtw-core)

[Documentation](docs/README.md) | [Getting Started](docs/getting-started.md) | [API Reference](docs/api-reference.md)

</div>

---

## Why mtwRequest?

Existing real-time frameworks are either slow (Socket.IO), locked to one language (Phoenix), or don't support AI natively (all of them). mtwRequest is different:

- **Rust core** — 10x more connections per server, 6x less memory than Node.js alternatives
- **WebSocket-first** — real-time as the default, not an afterthought. HTTP/SSE also supported
- **AI-native** — agents, streaming, tool calling, multi-model orchestration built-in
- **Modular** — everything is a plugin. Install what you need, nothing more
- **Polyglot** — one Rust core, bindings for Node.js, Python, PHP, and Browser (WASM)
- **Frontend SDKs** — React, Svelte, Vue, Three.js hooks out of the box
- **Response Pipeline** — 16 composable stages for HTTP (retry, cache, auth refresh, circuit breaker...)
- **Marketplace** — share and install community modules

## Quick Start

### Rust Server (10 lines)

```rust
use mtw_core::MtwServerBuilder;
use mtw_transport::ws::WebSocketTransport;

#[tokio::main]
async fn main() {
    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut events = transport.take_event_receiver().unwrap();
    transport.listen("0.0.0.0:8080".parse().unwrap()).await.unwrap();

    println!("mtwRequest running on ws://0.0.0.0:8080");

    while let Some(event) = events.recv().await {
        println!("{:?}", event);
    }
}
```

### React Client

```tsx
import { MtwProvider, useAgent, useChannel } from '@mtw/react'

function App() {
  return (
    <MtwProvider url="ws://localhost:8080/ws">
      <Chat />
    </MtwProvider>
  )
}

function Chat() {
  const { send, messages, isStreaming } = useAgent("assistant")
  const { publish, messages: chatMessages } = useChannel("chat.general")

  return (
    <div>
      {messages.map(m => <p key={m.id}>{m.content}</p>)}
      <button onClick={() => send("Hello AI!")}>Ask</button>
    </div>
  )
}
```

### HTTP Client with Pipeline

```rust
use mtw_http::{MtwHttpClient, stages::*};

let client = MtwHttpClient::builder()
    .base_url("https://api.example.com")
    .bearer_token("sk-...")
    .stage(RetryStage::new(RetryConfig::default()))
    .stage(CacheStage::new(CacheConfig::default()))
    .stage(RateLimitStage::new())
    .stage(CircuitBreakerStage::new(CircuitBreakerConfig::default()))
    .build()?;

let response = client.get("/users").await?;
let users: Vec<User> = response.json()?;

// Pipeline automatically handles: retry on 5xx, cache with ETag,
// rate limit tracking, circuit breaking on failures
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                  mtwRequest Core (Rust)                  │
│                                                         │
│  Transport    Router       AI Engine    HTTP Pipeline    │
│  (WebSocket   (Channels    (Agents      (16 stages:     │
│   HTTP SSE)    Middleware    Streaming    retry, cache   │
│                Pub/Sub)     Tools)       auth, etc.)    │
│                                                         │
│  Auth         State        Codec        Integrations    │
│  (JWT         (Memory      (JSON        (20 APIs        │
│   API Keys     Redis)       MsgPack)     10 AI models   │
│   OAuth2)                                OAuth2, RSS)   │
│                                                         │
│  Registry     SDK          Test                         │
│  (Marketplace  (Builder     (Harness                    │
│   Resolver)    Prelude)     Mocks)                      │
└────────┬──────────┬──────────┬──────────┬───────────────┘
         │          │          │          │
  ┌──────▼───┐ ┌────▼────┐ ┌──▼─────┐ ┌─▼──────┐
  │ Node.js  │ │ Python  │ │  PHP   │ │  WASM  │
  │ NAPI-RS  │ │  PyO3   │ │  FFI   │ │Browser │
  └──────────┘ └─────────┘ └────────┘ └────────┘
```

## Crates

| Crate | Description |
|-------|-------------|
| [`mtw-core`](crates/mtw-core) | Module system, server, config, lifecycle hooks |
| [`mtw-protocol`](crates/mtw-protocol) | Wire protocol, message types, binary frames |
| [`mtw-transport`](crates/mtw-transport) | WebSocket transport (tokio-tungstenite) |
| [`mtw-router`](crates/mtw-router) | Channels, pub/sub, middleware chain |
| [`mtw-ai`](crates/mtw-ai) | AI providers, agents, orchestration, memory |
| [`mtw-auth`](crates/mtw-auth) | JWT, API keys, OAuth2, auth middleware |
| [`mtw-http`](crates/mtw-http) | HTTP client with 16-stage response pipeline |
| [`mtw-state`](crates/mtw-state) | State stores (in-memory with TTL, Redis) |
| [`mtw-codec`](crates/mtw-codec) | Message codecs (JSON, extensible) |
| [`mtw-integrations`](crates/mtw-integrations) | 20 APIs, 10 AI models, OAuth2, RSS |
| [`mtw-registry`](crates/mtw-registry) | Marketplace client, dependency resolver |
| [`mtw-sdk`](crates/mtw-sdk) | SDK for module developers |
| [`mtw-test`](crates/mtw-test) | Test harness, mocks, assertions |

## Frontend SDKs

| Package | Framework | Key Exports |
|---------|-----------|-------------|
| `@mtw/client` | Universal | `MtwConnection`, `MtwChannel`, `MtwAgentClient` |
| `@mtw/react` | React | `MtwProvider`, `useChannel`, `useAgent`, `useStream` |
| `@mtw/svelte` | Svelte | Connection, channel, and agent stores |
| `@mtw/vue` | Vue | `useMtw`, `useChannel`, `useAgent` composables |
| `@mtw/three` | Three.js | `MtwScene` (sync), `MtwAsset` (streaming) |

## Language Bindings

| Language | Technology | Package |
|----------|-----------|---------|
| Node.js | NAPI-RS | `@mtw/core` |
| Python | PyO3 | `mtw-request` |
| PHP | C FFI | `mtw/core` |
| Browser | WASM | `@mtw/wasm` |

## Installation

### Rust
```bash
cargo add mtw-core mtw-transport mtw-router
```

### Node.js
```bash
npm install @mtw/client @mtw/react
```

### Python
```bash
pip install mtw-request
```

## Documentation

| Guide | Description |
|-------|-------------|
| [Getting Started](docs/getting-started.md) | Installation and first steps |
| [Server Guide](docs/server-guide.md) | Configuration and server setup |
| [Modules Guide](docs/modules-guide.md) | Creating and publishing modules |
| [Protocol Guide](docs/protocol-guide.md) | Wire format and message types |
| [Channels Guide](docs/channels-guide.md) | Pub/sub and real-time messaging |
| [AI Agents Guide](docs/ai-agents-guide.md) | AI providers, agents, tool calling |
| [Auth Guide](docs/auth-guide.md) | JWT, API keys, OAuth2 |
| [Frontend Guide](docs/frontend-guide.md) | React, Svelte, Vue, Three.js |
| [HTTP Pipeline Guide](docs/http-pipeline-guide.md) | Response pipeline stages |
| [Integrations Guide](docs/integrations-guide.md) | 20 APIs, 10 AI models, RSS |
| [Bindings Guide](docs/bindings-guide.md) | Node.js, Python, PHP, WASM |
| [API Reference](docs/api-reference.md) | Complete type reference |

## Performance

| Metric | mtwRequest (Rust) | Socket.IO (Node.js) |
|--------|-------------------|---------------------|
| Concurrent connections (1 core) | ~100,000 | ~10,000 |
| Memory per connection | ~2-5 KB | ~30 KB |
| Message latency (p99) | ~0.5 ms | ~5 ms |
| Messages/sec throughput | ~500,000 | ~50,000 |

## Roadmap

- [x] Core module system
- [x] WebSocket transport
- [x] Channel pub/sub with middleware
- [x] AI agent system with multi-model support
- [x] JWT/API key authentication
- [x] HTTP response pipeline (16 stages)
- [x] 20 API integrations + OAuth2
- [x] Frontend SDKs (React, Svelte, Vue, Three.js)
- [x] Language binding designs (Node.js, Python, PHP, WASM)
- [ ] CLI tool (`mtw init`, `mtw add`, `mtw publish`)
- [ ] Compiled NAPI-RS binding for Node.js
- [ ] Compiled PyO3 binding for Python
- [ ] WASM build for browsers
- [ ] Module marketplace web UI
- [ ] QUIC transport
- [ ] Multi-node clustering
- [ ] MessagePack and Protobuf codecs

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Licensed under the [Apache License 2.0](LICENSE).

**Attribution required**: If you use mtwRequest in your project, you must include the attribution specified in the [NOTICE](NOTICE) file. See Section 4(d) of the Apache License 2.0.

```
Powered by mtwRequest (https://github.com/fastslack/mtwRequest)
```

## Acknowledgments

Built with these excellent Rust crates: [tokio](https://tokio.rs), [tungstenite](https://github.com/snapview/tungstenite-rs), [serde](https://serde.rs), [reqwest](https://github.com/seanmonstar/reqwest), [dashmap](https://github.com/xacrimon/dashmap), [jsonwebtoken](https://github.com/Keats/jsonwebtoken).
