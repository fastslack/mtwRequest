# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-04-24

### Added

#### Performance · fanout hot path (6.5× faster vs 0.2.0 baseline)
- `SharedEnvelope` in `mtw-protocol` caches all wire forms (JSON text,
  MTW binary, MsgPack) lazily so a fan-out to N subscribers serializes
  the message exactly once regardless of subscriber count.
- `ConnTarget` trait + `EnvelopeSink::resolve()` resolve each
  subscriber's writer handle once at subscribe time, so every
  broadcast skips the conn-id → sender lookup entirely.
- `ArcSwap<Vec<SubscriberEntry>>` subscriber snapshot in
  `mtw-router`: publish reads the sub list via a single atomic load
  with zero DashMap shard locks.
- Per-connection writer task batches WebSocket frames with
  `SinkExt::feed` then one `flush()` per burst, collapsing many TCP
  syscalls into one.
- `tokio-tungstenite` upgraded to 0.29 — the cached JSON bytes are
  now wrapped into `Utf8Bytes` via `from_bytes_unchecked`, making the
  text delivery path fully zero-copy.
- `bench-mtw-server` ships with `tikv-jemallocator` as the global
  allocator: p99 latency variance under burst patterns collapsed from
  ~6× spread to ~7 % spread.

Fanout 500 subs p50 dropped from **418.9 ms → 52.5 ms**. Under jemalloc
the p99 sits at **85 ms**, ahead of NATS (91 ms) and Centrifugo
(251 ms).

#### MsgPack wire format (opt-in)
- New subprotocol `mtw.msgpack.v1` negotiated via
  `Sec-WebSocket-Protocol` at handshake.
- `SharedEnvelope::msgpack()` caches an `rmp-serde`-encoded body
  alongside the JSON and MTW binary forms.
- `WsConnTarget::format` captures the wire choice once at subscribe
  time (JsonText / MtwBinary / MsgPack) so the broadcast hot path
  doesn't branch on a DashMap lookup.
- Bench-client honours `MTW_WIRE=msgpack` to opt in from the runner.

Measured over 10 alternating JSON/MsgPack runs (fanout 500 subs):
- p50 median: **54.14 ms → 44.65 ms** (−17.5 %)
- p99 median: 86.97 ms → 79.30 ms (−8.8 %)
- **p99 IQR: 24.45 ms → 5.17 ms** (4.7× tighter tail distribution)

#### bench-suite/
- Reproducible Rust workspace that benchmarks mtwRequest against
  NATS, Centrifugo, and Socket.IO under identical Docker caps
  (2 CPU / 2 GB). Includes fanout (50 / 500 subs), echo RTT, and
  connect-storm scenarios. HdrHistogram output, markdown summaries,
  optional matplotlib plots.
- `./bench` one-shot runner, `./bench check` health probe.

#### bench-suite/site/
- Static single-page report (index.html + style.css + script.js +
  ApexCharts via CDN) showcasing the benchmark matrix with
  interactive charts, optimization journey, wire-format section, and
  methodology. No build step, no server.

### Changed
- `MtwTransport` trait gains `send_envelope()` for the hot fan-out
  path; the default impl falls back to `send()` for non-WS
  transports.
- `ConnTarget::deliver()` and `EnvelopeSink::deliver()` now take
  `&Arc<SharedEnvelope>` by reference to avoid a per-subscriber
  `Arc::clone`.

### Fixed
- `mtw-whatsapp` accepts `"attachments": null` from the Go bridge
  (previously rejected deserialisation).

## [0.1.0] - 2026-03-26

### Added

#### Core
- Module system with `MtwModule` trait, lifecycle hooks, and module registry
- Server with builder pattern and TOML configuration (`mtw.toml`)
- Wire protocol with ULID message IDs, typed payloads, and binary frame format
- Environment variable expansion in config files

#### Transport
- WebSocket transport using tokio-tungstenite
- Auto ping/pong keep-alive
- Connection lifecycle management

#### Routing
- Channel-based pub/sub with glob pattern matching (`chat.*`)
- Middleware chain with priority ordering
- Message history per channel
- Channel member limits and presence tracking

#### AI
- `MtwAIProvider` trait with streaming support
- `MtwAgent` trait with tool calling
- Multi-agent orchestrator (ChannelBased, Pipeline, FanOut, RoundRobin)
- Agent memory and context management
- Built-in providers: Anthropic (Claude), OpenAI (GPT), Ollama (local)

#### Authentication
- JWT authentication with token creation, validation, and refresh
- API key authentication with generation and revocation
- Auth middleware for message pipeline
- OAuth2 client with 12 pre-configured providers

#### HTTP Client
- `MtwHttpClient` with response pipeline architecture
- 16 built-in pipeline stages:
  - StatusCheck, JsonParse, Retry, AuthRefresh
  - Cache (ETag/Last-Modified), RateLimit, Pagination
  - Timeout, Transform, Validate, HeaderExtraction
  - Logging, CircuitBreaker, Metrics, Decompression, StreamProcessing
- Auto-pagination iterator
- Configurable auth strategies (Bearer, Basic, ApiKey, OAuth2)

#### Integrations
- 20 API integrations: GitHub, GitLab, Slack, Discord, Telegram, Twilio, SendGrid, Stripe, PayPal, AWS S3, Google Cloud Storage, Firebase, Supabase, Notion, Airtable, Jira, Linear, Vercel, Cloudflare, Docker Hub
- 10 AI model providers: Anthropic, OpenAI, Google Gemini, Mistral, Cohere, Meta Llama, xAI Grok, DeepSeek, Ollama, HuggingFace
- RSS/Atom feed reader

#### Ecosystem
- Module manifest format (`mtw-module.toml`)
- Dependency resolver with topological sort and semver matching
- Marketplace registry client
- SDK with builder API and prelude for module developers
- Test harness with mock transport, mock client, and assertion macros

#### Frontend SDKs
- `@mtw/client` — Universal WebSocket client with auto-reconnect
- `@mtw/react` — MtwProvider, useChannel, useAgent, useStream hooks
- `@mtw/svelte` — Reactive stores for connection, channels, agents
- `@mtw/vue` — Composables: useMtw, useChannel, useAgent
- `@mtw/three` — Three.js scene sync and asset streaming

#### Language Bindings
- Node.js binding design (NAPI-RS)
- Python binding design (PyO3)
- PHP binding design (C FFI)
- WASM binding design (wasm-bindgen)
