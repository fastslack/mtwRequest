# mtwRequest Documentation

**mtwRequest** is a Rust-based modular real-time framework that unifies WebSocket, HTTP/API, and AI agents into a single high-performance core with bindings for Node.js, Python, PHP, and Browser (WASM).

---

## Table of Contents

| Document | Description |
|----------|-------------|
| [Getting Started](./getting-started.md) | Installation, quick start guides, project overview |
| [Server Guide](./server-guide.md) | Server configuration, builder pattern, lifecycle, shutdown |
| [Modules Guide](./modules-guide.md) | Creating modules, the MtwModule trait, manifests, marketplace |
| [Protocol Guide](./protocol-guide.md) | Wire format, message types, binary frames, streaming |
| [Channels Guide](./channels-guide.md) | Pub/sub channels, patterns, history, presence |
| [AI Agents Guide](./ai-agents-guide.md) | AI providers, agents, tool calling, orchestration |
| [Auth Guide](./auth-guide.md) | JWT, API keys, OAuth2, auth middleware |
| [Frontend Guide](./frontend-guide.md) | React, Svelte, Vue, Three.js client SDKs |
| [HTTP Pipeline Guide](./http-pipeline-guide.md) | Response pipeline, stages, retry, caching |
| [Integrations Guide](./integrations-guide.md) | 20 API integrations, OAuth2, AI providers |
| [Bindings Guide](./bindings-guide.md) | Node.js, Python, PHP, WASM bindings |
| [API Reference](./api-reference.md) | Complete trait, struct, and enum reference by crate |

---

## Architecture Overview

```
+------------------------------------------------------------------+
|                    mtwRequest Core (Rust)                         |
|                                                                  |
|  +------------+  +------------+  +------------+  +-----------+   |
|  | Transport  |  |  Router    |  |  AI Engine |  |  Module   |   |
|  |  Layer     |  |            |  |            |  |  Runtime  |   |
|  |            |  | - paths    |  | - agents   |  |           |   |
|  | - ws       |  | - channels |  | - streams  |  | - load    |   |
|  | - http     |  | - rooms    |  | - tools    |  | - isolate |   |
|  | - sse      |  | - middleware| | - memory   |  | - sandbox |   |
|  | - quic     |  |            |  | - providers|  | - hooks   |   |
|  +------------+  +------------+  +------------+  +-----------+   |
|                                                                  |
|  +------------+  +------------+  +------------+  +-----------+   |
|  | Codec      |  |  State     |  |  Auth      |  |  Registry |   |
|  |            |  |  Store     |  |            |  |  Client   |   |
|  | - json     |  |            |  | - jwt      |  |           |   |
|  | - msgpack  |  | - memory   |  | - api keys |  | - resolve |   |
|  | - protobuf |  | - redis    |  | - oauth    |  | - install |   |
|  | - binary   |  | - custom   |  | - custom   |  | - update  |   |
|  +------------+  +------------+  +------------+  +-----------+   |
|                                                                  |
|  +------------------------------------------------------------+  |
|  |                Module API (Trait System)                    |  |
|  |  MtwModule - MtwTransport - MtwMiddleware - MtwAIProvider  |  |
|  |  MtwCodec - MtwAuth - MtwStateStore - MtwAgent             |  |
|  +------------------------------------------------------------+  |
+----------+----------+----------+----------+-------------------+
           |          |          |          |
    +------v---+ +----v----+ +--v-----+ +-v------+
    | NAPI-RS  | |  PyO3   | |PHP FFI | |  WASM  |
    | Node.js  | | Python  | |  PHP   | |Browser |
    +----------+ +---------+ +--------+ +--------+
```

---

## Quick Links

- **Source code**: `crates/` (Rust), `packages/` (TypeScript), `bindings/` (FFI)
- **Examples**: `examples/demo_server.rs`, `examples/demo_client.rs`
- **Configuration**: `mtw.toml` (see [Server Guide](./server-guide.md))
- **Module manifest**: `mtw-module.toml` (see [Modules Guide](./modules-guide.md))

---

## Current Status

| Phase | Status |
|-------|--------|
| Phase 1 -- Foundation (protocol, core, transport, router, codec) | Complete |
| Phase 2 -- AI and Auth (providers, agents, JWT, state) | Complete |
| Phase 3 -- Ecosystem (registry, SDK, test harness, marketplace) | Complete |
| Phase 4 -- Bindings and Scale (Node.js, Python, PHP, WASM, frontend SDKs) | In Progress |
