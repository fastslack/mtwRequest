# Roadmap

mtwRequest evolves in public. This document tracks what's shipped and what's next — for release history see [CHANGELOG.md](CHANGELOG.md), and for the full design see [ARCHITECTURE.md](ARCHITECTURE.md).

> Current version: **v0.1.0** · License: Apache-2.0 · [README](README.md) · [Contributing](CONTRIBUTING.md)

## Shipped

### Core runtime
- [x] Module system with lifecycle hooks
- [x] WebSocket transport with binary frame protocol
- [x] Channel pub/sub with glob matching and middleware
- [x] MsgPack wire format (opt-in via `mtw.msgpack.v1` subprotocol)
- [x] Unix socket bridge (client + server, MessagePack RPC)

### AI & agents
- [x] AI agent system (providers, streaming, tool calling)
- [x] Agent flows, chains, schedules, event triggers
- [x] Agent executor with loop detection, timeout, token budget
- [x] Reactive engine (event-driven agent execution)

### Auth & security
- [x] JWT / API key authentication
- [x] Rate limiting, approval gates, device pairing

### HTTP & integrations
- [x] HTTP response pipeline (16 stages)
- [x] 20 API integrations + OAuth2

### Trading
- [x] 15 technical analysis formulas + trade monitor

### Federation & comms
- [x] Federation: P2P sync, peer discovery, conflict resolution
- [x] Multi-channel notifications
- [x] Email: templates, campaigns, account management

### Data
- [x] Graph database client with analytics

### Extensibility & tooling
- [x] Skills / plugin system with marketplace
- [x] MCP server for Claude Code (8 agent-management tools)
- [x] Frontend SDKs (React, Svelte, Vue, Three.js)
- [x] npm package published

## Next

### Tooling
- [ ] CLI tool (`mtw init`, `mtw add`, `mtw publish`)
- [ ] Module marketplace web UI

### Bindings
- [ ] NAPI-RS binding for Node.js (compiled)
- [ ] PyO3 binding for Python (compiled)
- [ ] WASM build for browsers

### Transport & codecs
- [ ] QUIC transport
- [ ] Multi-node clustering
- [ ] Protobuf codec

---

Want to help? See [CONTRIBUTING.md](CONTRIBUTING.md). Issues tagged `roadmap` and `good-first-issue` are good places to start.
