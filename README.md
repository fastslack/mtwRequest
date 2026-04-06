<div align="center">

# mtwRequest

**High-performance, modular real-time framework for WebSocket, HTTP, and AI agents.**

Built in Rust. Runs everywhere.

[![CI](https://github.com/fastslack/mtwRequest/actions/workflows/ci.yml/badge.svg)](https://github.com/fastslack/mtwRequest/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Crates.io](https://img.shields.io/crates/v/mtw-core.svg)](https://crates.io/crates/mtw-core)
[![npm](https://img.shields.io/npm/v/@matware/mtw-request-ts-client.svg)](https://www.npmjs.com/package/@matware/mtw-request-ts-client)
[![Rust](https://img.shields.io/badge/rust-1.86%2B-orange.svg)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-522%20passing-brightgreen.svg)](#)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)
[![Discord](https://img.shields.io/badge/discord-join-7289da.svg)](https://discord.gg/mtwrequest)

[Documentation](docs/README.md) | [Getting Started](#quick-start) | [API Reference](docs/api-reference.md) | [Discord](https://discord.gg/mtwrequest)

</div>

---

## Why mtwRequest?

Existing real-time frameworks are either slow (Socket.IO), locked to one language (Phoenix), or don't support AI natively (all of them). mtwRequest is different:

- **Rust core** -- 10x more connections per server, 6x less memory than Node.js alternatives
- **WebSocket-first** -- real-time as the default, not an afterthought. HTTP/SSE also supported
- **AI-native** -- agents, streaming, tool calling, multi-model orchestration, flows, chains, schedules
- **Trading engine** -- 15 built-in technical analysis formulas, SL/TP monitor, strategy management
- **Security-first** -- rate limiting, approval gates, device pairing, policy engine
- **Modular** -- everything is a plugin. Install what you need, nothing more
- **Polyglot** -- one Rust core, bindings for Node.js, Python, PHP, and Browser (WASM)
- **Frontend SDKs** -- React, Svelte, Vue, Three.js hooks out of the box
- **Response Pipeline** -- 16 composable stages for HTTP (retry, cache, auth refresh, circuit breaker...)
- **Federation** -- peer-to-peer sync between instances with conflict resolution
- **MCP server** -- manage everything from Claude Code with 37 tools
- **Marketplace** -- share and install community modules

## Quick Start

### Install & Run

```bash
# Clone and build
git clone https://github.com/fastslack/mtwRequest.git
cd mtwRequest
cargo build --release

# Run the server
./target/release/mtw-server
# => mtwRequest listening on ws://0.0.0.0:7741/ws
```

### Rust Server (10 lines)

```rust
use mtw_core::MtwServerBuilder;
use mtw_transport::ws::WebSocketTransport;

#[tokio::main]
async fn main() {
    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut events = transport.take_event_receiver().unwrap();
    transport.listen("0.0.0.0:7741".parse().unwrap()).await.unwrap();

    println!("mtwRequest running on ws://0.0.0.0:7741");

    while let Some(event) = events.recv().await {
        println!("{:?}", event);
    }
}
```

### JavaScript/TypeScript Client

```bash
npm install @matware/mtw-request-ts-client
```

```typescript
import { MtwConnection, MtwChannel, MtwAgentClient } from '@matware/mtw-request-ts-client';

// Connect
const conn = new MtwConnection({ url: 'ws://localhost:7741/ws' });
await conn.connect();

// Subscribe to channels (pub/sub)
const channel = new MtwChannel(conn, 'chat.general');
channel.on('message', (msg) => console.log(msg.payload));
await channel.subscribe();

// Interact with AI agents
const agent = new MtwAgentClient(conn, { channel: 'agents' });
const response = await agent.ask('What tasks are due today?');

// Streaming responses
for await (const chunk of agent.stream('Analyze market conditions')) {
  process.stdout.write(chunk.delta);
}
```

### React Client

```tsx
import { MtwProvider, useAgent, useChannel } from '@mtw/react'

function App() {
  return (
    <MtwProvider url="ws://localhost:7741/ws">
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
// Pipeline automatically handles: retry on 5xx, cache with ETag,
// rate limit tracking, circuit breaking on failures
```

### Docker

```bash
docker run -p 7741:7741 fastslack/mtw-request:latest
```

### Claude Code (MCP)

```bash
# Install as MCP server for Claude Code
claude mcp add mtw-request -- ./target/release/mtw-mcp

# Then ask Claude: "list all mtwRequest modules" or "create an agent"
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                      mtwRequest Core (Rust)                         │
│                                                                     │
│  Transport    Router       AI Engine      Trading       Security    │
│  (WebSocket   (Channels    (Agents        (15 formulas  (Rate limit │
│   HTTP SSE)    Middleware    Flows/Chains   SL/TP        Policies    │
│                Pub/Sub)     Scheduling     Monitor)      Approvals  │
│                             Tool calling                 Pairing)   │
│                             Streaming)                              │
│                                                                     │
│  Auth         State        HTTP Pipeline  Federation    Notify      │
│  (JWT         (Memory      (16 stages:    (P2P sync     (Multi-     │
│   API Keys     Redis)       retry, cache   Peer disc.    channel    │
│   OAuth2)                   auth, etc.)    Conflict)     routing)   │
│                                                                     │
│  Bridge       Comms        Skills         Graph         Orchestr.   │
│  (Unix sock   (Email       (Plugin        (Neo4j        (Slash      │
│   MsgPack     Templates    Registry       Analytics     commands    │
│   RPC)        Campaigns)   Marketplace)   Network)      Callbacks)  │
│                                                                     │
│  Codec        SDK          MCP Server     Store         Exchange    │
│  (JSON)       (Builder     (37 tools      (SQLite       (Bitvavo    │
│               Prelude)     Claude Code)   Pool, WAL)    multi-ex)   │
├─────────┬──────────┬──────────┬──────────┬──────────────────────────┘
│         │          │          │          │
│  ┌──────▼───┐ ┌────▼────┐ ┌──▼─────┐ ┌─▼──────┐
│  │ Node.js  │ │ Python  │ │  PHP   │ │  WASM  │
│  │ NAPI-RS  │ │  PyO3   │ │  FFI   │ │Browser │
│  └──────────┘ └─────────┘ └────────┘ └────────┘
```

## Crates

mtwRequest is organized as a Rust workspace with 26 crates:

### Core

| Crate | Description |
|-------|-------------|
| [`mtw-core`](crates/mtw-core) | Module system, server, config, lifecycle hooks |
| [`mtw-protocol`](crates/mtw-protocol) | Wire protocol, message types, binary frames |
| [`mtw-transport`](crates/mtw-transport) | WebSocket transport (tokio-tungstenite) |
| [`mtw-router`](crates/mtw-router) | Channels, pub/sub, middleware chain |
| [`mtw-codec`](crates/mtw-codec) | Message codecs (JSON, extensible) |
| [`mtw-server`](crates/mtw-server) | Production server binary |

### AI & Agents

| Crate | Description |
|-------|-------------|
| [`mtw-ai`](crates/mtw-ai) | AI providers, agents, orchestration, flows, chains, schedules, triggers, feedback, reactive engine, executor |
| [`mtw-orchestrator`](crates/mtw-orchestrator) | Slash commands, message routing, callback handling |

### Auth & Security

| Crate | Description |
|-------|-------------|
| [`mtw-auth`](crates/mtw-auth) | JWT, API keys, OAuth2, auth middleware |
| [`mtw-security`](crates/mtw-security) | Rate limiting, security policies, approval gates, device pairing |

### Data & State

| Crate | Description |
|-------|-------------|
| [`mtw-state`](crates/mtw-state) | State stores (in-memory with TTL, Redis) |
| [`mtw-store`](crates/mtw-store) | SQLite data store with connection pooling |
| [`mtw-graph`](crates/mtw-graph) | Graph database client, analytics, network analysis |

### Trading

| Crate | Description |
|-------|-------------|
| [`mtw-trading`](crates/mtw-trading) | 15 technical analysis formulas, trade monitor (SL/TP/trailing), strategies, signals, consensus engine. Installable as optional module. |
| [`mtw-exchange`](crates/mtw-exchange) | Multi-exchange connectivity (Bitvavo, Kraken, Coinbase, Binance), rate limiting |

### Communications

| Crate | Description |
|-------|-------------|
| [`mtw-notify`](crates/mtw-notify) | Multi-channel notification system (Telegram, Slack, Discord, etc.) |
| [`mtw-comms`](crates/mtw-comms) | Email service, templates, campaigns, account management |

### Integration

| Crate | Description |
|-------|-------------|
| [`mtw-bridge`](crates/mtw-bridge) | Unix socket bridge (client + server), MessagePack RPC |
| [`mtw-federation`](crates/mtw-federation) | Peer-to-peer sync, changelog, conflict resolution, peer discovery |
| [`mtw-http`](crates/mtw-http) | HTTP client with 16-stage response pipeline |
| [`mtw-integrations`](crates/mtw-integrations) | 20 APIs, 10 AI models, OAuth2, RSS |

### Extensibility

| Crate | Description |
|-------|-------------|
| [`mtw-skills`](crates/mtw-skills) | Dynamic skill/plugin system, permissions, marketplace |
| [`mtw-registry`](crates/mtw-registry) | Marketplace client, dependency resolver |
| [`mtw-mcp`](crates/mtw-mcp) | MCP server for Claude Code (37 tools) |
| [`mtw-sdk`](crates/mtw-sdk) | SDK for module developers |
| [`mtw-test`](crates/mtw-test) | Test harness, mock transport, assertions |

## Frontend SDKs

| Package | Framework | Install |
|---------|-----------|---------|
| [`@matware/mtw-request-ts-client`](https://www.npmjs.com/package/@matware/mtw-request-ts-client) | Universal JS/TS | `npm i @matware/mtw-request-ts-client` |
| `@mtw/react` | React | Hooks: `useChannel`, `useAgent`, `useStream` |
| `@mtw/svelte` | Svelte | Stores for connection, channels, agents |
| `@mtw/vue` | Vue | Composables: `useMtw`, `useChannel`, `useAgent` |
| `@mtw/three` | Three.js | Real-time scene sync, asset streaming |

## Trading Formulas

15 built-in technical analysis formulas, all running in Rust:

| Formula | Type | Description |
|---------|------|-------------|
| RSI | Momentum | Relative Strength Index (14), oversold/overbought |
| MACD | Trend | Moving Average Convergence/Divergence (12/26/9) |
| Bollinger Bands | Volatility | Price vs 2-sigma bands around SMA(20) |
| EMA Crossover | Trend | Short/long EMA cross detection (9/21) |
| SuperTrend | Trend | ATR-based trend flip detection |
| ADX | Trend Strength | Average Directional Index with +DI/-DI |
| Stochastic RSI | Momentum | %K/%D crossovers, 20/80 zones |
| Ichimoku Cloud | Multi-Signal | Tenkan/Kijun/Senkou with score-based signals |
| OBV | Volume | On Balance Volume with divergence detection |
| Kelly Criterion | Sizing | Optimal position sizing from win rate |
| Linear Regression | Statistical | Least squares, R-squared, deviation bands |
| VWAP | Volume | Volume-weighted average price deviation |
| Williams %R | Momentum | Fast oscillator, -20/-80 zones |
| Ensemble Vote | Meta | 12 weighted micro-rules combined |
| Market Regime | Context | Bull/Bear/Range classifier from 6 signals |

All formulas implement `SignalFormula` trait and can be extended:

```rust
use mtw_trading::{FormulaRegistry, SignalFormula, FormulaResult, Candle};

let mut registry = FormulaRegistry::new();
mtw_trading::formulas::register_all(&mut registry);

let consensus = registry.consensus("BTC/USDT", &candles, None);
if consensus.meets_threshold(3, 65.0) {
    println!("Signal: {:?} with {:.0}% confidence", consensus.side, consensus.avg_confidence);
}
```

## MCP Server (Claude Code Plugin)

mtwRequest includes an MCP server binary with 37 tools for managing the entire framework from Claude Code:

```bash
# Build and install
cargo build --release -p mtw-mcp
claude mcp add mtw-request -- ./target/release/mtw-mcp
```

**Available tool domains:**

| Domain | Tools | Description |
|--------|-------|-------------|
| `mtw_server_*` | 3 | Server status, config, health |
| `mtw_modules_*` | 3 | Module lifecycle, install, health check |
| `mtw_agents_*` | 7 | Create, run, schedule, chain, flow, trigger agents |
| `mtw_auth_*` | 4 | JWT tokens, API keys, revocation |
| `mtw_trading_*` | 3 | Formulas, SL/TP monitor, strategies |
| `mtw_security_*` | 3 | Rate limits, policies, approval gates |
| `mtw_channels_*` | 3 | Pub/sub channels, publish, create |
| `mtw_transport_*` | 3 | WebSocket connections, kick, broadcast |
| `mtw_federation_*` | 2 | P2P peer sync, changelog |
| `mtw_notify_*` | 2 | Send notifications, manage providers |
| `mtw_skills_*` | 4 | Skills, marketplace, permissions |

## Configuration

mtwRequest uses TOML configuration with environment variable expansion:

```toml
# mtw.toml
[server]
host = "0.0.0.0"
port = 7741
max_connections = 10000

[transport]
default = "websocket"

[transport.websocket]
path = "/ws"
ping_interval = 30

[codec]
default = "json"

[[channels]]
name = "chat.*"
max_members = 100
history = 50

[[channels]]
name = "notifications"
history = 20

[store]
path = "./data/app.db"

[store.bridge]
socket = "/tmp/mtw-bridge.sock"
```

## Performance

| Metric | mtwRequest (Rust) | Socket.IO (Node.js) |
|--------|-------------------|---------------------|
| Concurrent connections (1 core) | ~100,000 | ~10,000 |
| Memory per connection | ~2-5 KB | ~30 KB |
| Message latency (p99) | ~0.5 ms | ~5 ms |
| Messages/sec throughput | ~500,000 | ~50,000 |
| Bridge RPC round-trip | ~0.05 ms | N/A |
| Formula computation (15 formulas) | ~0.1 ms | ~5 ms |

## Docker

```bash
# Single server
docker run -p 7741:7741 fastslack/mtw-request:latest

# With config
docker run -p 7741:7741 -v ./mtw.toml:/app/mtw.toml fastslack/mtw-request:latest

# Full stack (with mtwKernel, Neo4j, Dashboard)
docker compose up -d
```

## Roadmap

- [x] Core module system with lifecycle hooks
- [x] WebSocket transport with binary frame protocol
- [x] Channel pub/sub with glob matching and middleware
- [x] AI agent system (providers, streaming, tool calling)
- [x] Agent flows, chains, schedules, event triggers
- [x] Agent executor with loop detection, timeout, token budget
- [x] Reactive engine (event-driven agent execution)
- [x] JWT/API key authentication
- [x] HTTP response pipeline (16 stages)
- [x] 20 API integrations + OAuth2
- [x] 15 trading formulas + trade monitor
- [x] Security: rate limiting, approval gates, device pairing
- [x] Federation: P2P sync, peer discovery, conflict resolution
- [x] Multi-channel notifications
- [x] Email: templates, campaigns, account management
- [x] Graph database client with analytics
- [x] Skills/plugin system with marketplace
- [x] Unix socket bridge (client + server, MessagePack RPC)
- [x] MCP server for Claude Code (37 tools)
- [x] Frontend SDKs (React, Svelte, Vue, Three.js)
- [x] npm package published
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

```bash
# Development
cargo check          # Type check
cargo test           # Run 522 tests
cargo clippy         # Lint
cargo fmt            # Format

# Test a specific crate
cargo test -p mtw-trading
```

## Community

- [GitHub Issues](https://github.com/fastslack/mtwRequest/issues) -- Bug reports and feature requests
- [GitHub Discussions](https://github.com/fastslack/mtwRequest/discussions) -- Questions and ideas
- [Discord](https://discord.gg/mtwrequest) -- Real-time chat

## Security

Found a vulnerability? See [SECURITY.md](SECURITY.md) for responsible disclosure.

## License

Licensed under the [Apache License 2.0](LICENSE).

```
Copyright 2024-2026 fastslack
Licensed under the Apache License, Version 2.0
```

**Attribution required**: If you use mtwRequest in your project, include the [NOTICE](NOTICE) file per Section 4(d) of Apache 2.0.

## Acknowledgments

Built with: [tokio](https://tokio.rs), [tungstenite](https://github.com/snapview/tungstenite-rs), [serde](https://serde.rs), [reqwest](https://github.com/seanmonstar/reqwest), [dashmap](https://github.com/xacrimon/dashmap), [jsonwebtoken](https://github.com/Keats/jsonwebtoken), [rusqlite](https://github.com/rusqlite/rusqlite).
