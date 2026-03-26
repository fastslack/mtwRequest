# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
