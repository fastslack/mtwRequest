# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-05-04

### Added

#### `mtw-net` crate — central outbound networking
- New `crates/mtw-net` factory builds every `reqwest::Client` from a
  single, profile-driven config so a single `[net].default_profile`
  flag can route every outbound HTTP call (AI providers, exchange,
  integrations, registry, torrent webseeds, federation HTTP) through
  a VPN/Tor SOCKS5.
- Profiles in this release: `Direct`, `Proxy` (HTTP, HTTPS, SOCKS5,
  SOCKS5h). Reserved (gated behind cargo features, not built):
  `Wireguard` via `boringtun`, `Tor` via `arti`.
- HTTPS-only mode, configurable TLS minimum-version floor (1.2 or
  1.3), shared user-agent, optional proxy bypass list.
- `NetFactory` caches one client per profile name (cheap clone).
- Process-wide `OnceLock`-backed global so call sites can opt in with
  a one-line change (`mtw_net::default_client()` /
  `default_client_builder()`); falls back to a stock `reqwest::Client`
  when no factory is installed (zero-impact on callers that don't care).
- `NetConfig::from_mtw_toml` parses `[net]` out of the same `mtw.toml`
  used by `MtwConfig` without coupling `mtw-core` to `mtw-net`.

#### Outbound traffic now honours `[net]` everywhere
- Every cloud-facing `reqwest::Client` is now built via mtw-net:
  `mtw-ai` cloud providers (Anthropic, OpenAI), `mtw-exchange/bitvavo`,
  `mtw-integrations` (cloud AI providers + OAuth2), `mtw-registry`,
  `mtw-federation` HTTP fallback, `mtw-http`'s `MtwHttpClient`.
- `mtw-server` reads `[net]` at boot and installs the factory before
  any provider is constructed.
- Local providers (`mtw-ai/ollama`, `mtw-ai/lmstudio` and their
  `mtw-integrations` counterparts) deliberately bypass the profile —
  routing localhost requests through a SOCKS5/Tor proxy would break
  them and offers no privacy upside.

#### `mtw-torrent` crate + `torrent.*` bridge tools
- New `crates/mtw-torrent` exposes `torrent.add`, `list`, `get`,
  `remove`, `pause`, `resume`, `health`, `encryption.profiles`,
  `encryption.set_default` over the bridge for mtwKernel.
- `librqbit` 8.x backend behind cargo feature `librqbit-engine`; an
  in-memory `MockEngine` is the default fallback so the crate
  compiles without librqbit's heavy deps.
- **Async `add`** semantics: `librqbit::Session::add_torrent` for a
  magnet awaits `resolve_magnet` for ~30 s+ on poorly-seeded
  torrents — too long for the bridge. `add()` returns a synthetic
  detail (`status: metadata`) immediately and resolves the magnet in
  a background task. Side-map sidecar persists the user-supplied
  fields (encryption profile, category, tags, description, ext)
  that librqbit doesn't track, surviving the handle's lifetime.
- Single shared 2 s-tick **progress pump** emits
  `torrent.metadata_ready` (one-shot when files resolve),
  `torrent.progress` (active states only), `torrent.done` (one-shot
  at finished), and `torrent.error` through the bridge event bus.
- Embedded HTTP **data plane** is librqbit's own `HttpApi` with
  native HTTP `Range` support, bound to `TORRENT_HTTP_LISTEN`
  (default `127.0.0.1:9999`). `stream_url(infohash, file_idx)`
  returns `http://<listen>/torrents/{ih}/stream/{idx}` for the
  kernel proxy.
- Server bin opt-in via `cargo build -p mtw-server --features
  torrent`. Env vars: `TORRENT_ENABLED`, `TORRENT_STORAGE_PATH`,
  `TORRENT_HTTP_LISTEN`, `TORRENT_DEFAULT_PROFILE`,
  `TORRENT_PROFILES_FILE`, `TORRENT_STORAGE_QUOTA_BYTES`.

#### `mtw-bridge` — server-pushed events
- `BridgeEventBus` lets any tool handler push frames to every
  connected client through the same Unix socket as responses,
  distinguished by a `type: "event"` field instead of `id`.
  Backwards compatible: clients that only know about responses can
  ignore unknown frames.
- Per-connection writer is mpsc-fed so events and responses don't
  interleave mid-frame. Capacity 256 / subscriber; slow consumers
  see `Lagged` and skip ahead — emitters never block.
- Bridge socket auto-`chmod 0666` after bind so non-root clients
  (e.g. mtwKernel container as uid 1000) connect without manual
  chmod. Override with `MTW_BRIDGE_SOCKET_MODE` (e.g. `0660` for
  hardened deployments with shared gid).

### Changed
- Workspace `serde` gains the `rc` feature flag — required by
  librqbit 8.1.1's HTTP API, harmless elsewhere.
- `Dockerfile` exposes port 9999 (torrent data plane); container
  defaults bind on `0.0.0.0:9999` and store data at
  `/var/lib/mtwrequest/torrents`.
- `docker-compose.yml` adds named volume `mtw-torrent-data` so
  downloads + librqbit `_meta/state.json` (resume data) survive
  `docker compose up -d --build`. Sets
  `MTW_BRIDGE_SOCKET_MODE=0666` so the kernel container connects
  without per-restart manual chmod.

### Fixed
- `Payload` wire format pinned back to PascalCase (`"None"`,
  `"Text"`, `"Json"`, `"Binary"`) so existing TS clients
  (`@matware/mtw-request-ts-client` v0.1.x — kernel + dashboard)
  keep decoding frames. Lowercase variants kept as `serde(alias)`
  for forward-compat.

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
