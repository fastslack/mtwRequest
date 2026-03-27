<project>
  <overview>
    <name>mtwRequest</name>
    <description>
      A Rust-based modular real-time framework that unifies WebSocket, HTTP/API, and AI agents
      into a single high-performance core with bindings for Node.js, Python, PHP, and Browser (WASM).
      Users can create, install, and share modules through a marketplace.
    </description>
    <version>0.1.0</version>
    <license>MIT</license>
    <author>fastslack</author>
    <repository>https://github.com/fastslack/mtwRequest</repository>
    <architecture-doc>ARCHITECTURE.md</architecture-doc>
  </overview>

  <build-and-test>
    <prerequisites>
      <item>Rust toolchain (edition 2021)</item>
      <item>cargo (workspace-aware)</item>
    </prerequisites>
    <commands>
      <build>cargo build</build>
      <test>cargo test</test>
      <test-single-crate>cargo test -p mtw-core</test-single-crate>
      <check>cargo check</check>
      <clippy>cargo clippy</clippy>
      <format>cargo fmt</format>
      <format-check>cargo fmt -- --check</format-check>
    </commands>
    <notes>
      <item>38 tests currently passing across all crates</item>
      <item>Configuration uses TOML format (mtw.toml) with ${ENV_VAR} expansion</item>
      <item>No CLI binary yet; the project is library-only at this stage</item>
    </notes>
  </build-and-test>

  <crate-structure>
    <workspace-root>Cargo.toml (resolver = "2")</workspace-root>

    <crate name="mtw-protocol" path="crates/mtw-protocol">
      <purpose>Wire protocol definitions: MtwMessage, MsgType, Payload, ConnId, TransportEvent</purpose>
      <key-files>
        <file path="src/message.rs">MtwMessage struct with ULID IDs, MsgType enum, Payload enum, TransportEvent enum, ConnMetadata, AuthInfo, DisconnectReason</file>
        <file path="src/frame.rs">Wire frame format</file>
        <file path="src/error.rs">Protocol-level errors</file>
      </key-files>
      <dependencies>serde, serde_json, ulid, base64</dependencies>
    </crate>

    <crate name="mtw-core" path="crates/mtw-core">
      <purpose>Kernel: module system, lifecycle management, configuration, hooks</purpose>
      <key-files>
        <file path="src/module.rs">MtwModule trait, ModuleRegistry, ModuleManifest, ModuleType enum, Permission enum, SharedState, HealthStatus</file>
        <file path="src/config.rs">MtwConfig loaded from mtw.toml, ServerConfig, TransportConfig, WebSocketConfig, env var expansion</file>
        <file path="src/hooks.rs">LifecycleHooks trait, HookRegistry (on_connect, on_disconnect, before_message, after_message, on_error)</file>
        <file path="src/server.rs">Server orchestration</file>
        <file path="src/error.rs">MtwError enum</file>
      </key-files>
      <dependencies>mtw-protocol, tokio, async-trait, serde, serde_json, dashmap, tracing, toml</dependencies>
    </crate>

    <crate name="mtw-codec" path="crates/mtw-codec">
      <purpose>Serialization codecs (JSON implemented)</purpose>
      <key-files>
        <file path="src/lib.rs">Codec trait definition</file>
        <file path="src/json.rs">JSON codec implementation</file>
      </key-files>
      <dependencies>mtw-protocol, serde, serde_json</dependencies>
    </crate>

    <crate name="mtw-transport" path="crates/mtw-transport">
      <purpose>Transport abstraction and WebSocket implementation</purpose>
      <key-files>
        <file path="src/lib.rs">MtwTransport trait (listen, send, send_binary, broadcast, close, shutdown, connection_count, has_connection, take_event_receiver)</file>
        <file path="src/ws.rs">WebSocket transport using tokio-tungstenite</file>
      </key-files>
      <dependencies>mtw-core, mtw-protocol, tokio, tokio-tungstenite, async-trait, dashmap</dependencies>
    </crate>

    <crate name="mtw-router" path="crates/mtw-router">
      <purpose>Message routing, pub/sub channels, middleware chain</purpose>
      <key-files>
        <file path="src/channel.rs">Channel (pub/sub with history, max members, glob pattern matching), ChannelManager</file>
        <file path="src/middleware.rs">MtwMiddleware trait, MiddlewareChain (priority-ordered, inbound/outbound), MiddlewareAction enum (Continue, Halt, Transform, Redirect)</file>
        <file path="src/router.rs">Message router</file>
      </key-files>
      <dependencies>mtw-core, mtw-protocol, tokio, async-trait, dashmap, tracing</dependencies>
    </crate>
  </crate-structure>

  <key-traits-and-patterns>
    <trait name="MtwModule" crate="mtw-core" file="src/module.rs">
      <description>Universal module interface. Every module implements this for lifecycle management.</description>
      <methods>
        <method>fn manifest() -> ModuleManifest -- module metadata</method>
        <method>async fn on_load(ctx) -- called when module is loaded</method>
        <method>async fn on_start(ctx) -- called when server starts</method>
        <method>async fn on_stop(ctx) -- called on shutdown</method>
        <method>async fn health() -> HealthStatus -- health check (default: Healthy)</method>
      </methods>
    </trait>

    <trait name="MtwTransport" crate="mtw-transport" file="src/lib.rs">
      <description>Transport layer abstraction over WebSocket, HTTP, SSE, etc.</description>
      <methods>
        <method>async fn listen(addr) -- start accepting connections</method>
        <method>async fn send(conn_id, msg) -- send to specific connection</method>
        <method>async fn send_binary(conn_id, data) -- send raw bytes</method>
        <method>async fn broadcast(msg) -- send to all connections</method>
        <method>async fn close(conn_id) -- close a connection</method>
        <method>fn take_event_receiver() -- get mpsc::UnboundedReceiver for TransportEvent</method>
        <method>fn connection_count() -- active connection count</method>
        <method>async fn shutdown() -- graceful shutdown</method>
      </methods>
    </trait>

    <trait name="MtwMiddleware" crate="mtw-router" file="src/middleware.rs">
      <description>Request/response pipeline interceptor with priority ordering.</description>
      <methods>
        <method>fn name() -- middleware identifier</method>
        <method>fn priority() -> i32 -- lower runs first (default: 100)</method>
        <method>async fn on_inbound(msg, ctx) -> MiddlewareAction -- process client-to-server</method>
        <method>async fn on_outbound(msg, ctx) -> MiddlewareAction -- process server-to-client (default: Continue)</method>
      </methods>
    </trait>

    <trait name="LifecycleHooks" crate="mtw-core" file="src/hooks.rs">
      <description>Connection and message lifecycle hooks with chaining via HookRegistry.</description>
      <methods>
        <method>async fn on_connect(conn_id, meta)</method>
        <method>async fn on_disconnect(conn_id, reason)</method>
        <method>async fn before_message(conn_id, msg) -> Option&lt;MtwMessage&gt; -- can reject or transform</method>
        <method>async fn after_message(conn_id, msg)</method>
        <method>async fn on_error(conn_id, error)</method>
      </methods>
    </trait>

    <pattern name="ModuleRegistry">
      <description>Manages module registration, ordered loading, starting, and reverse-order stopping. Prevents duplicate registrations. Supports health checks across all modules.</description>
    </pattern>

    <pattern name="ChannelManager">
      <description>Manages pub/sub channels with glob pattern matching (e.g., "chat.*"), subscriber limits, message history ring buffer, and connection cleanup on disconnect.</description>
    </pattern>

    <pattern name="MiddlewareChain">
      <description>Processes messages through priority-sorted middleware. Inbound runs low-to-high priority; outbound runs in reverse. Supports Continue, Halt, Transform, and Redirect actions.</description>
    </pattern>

    <pattern name="MtwMessage">
      <description>Wire message format using ULID IDs, typed payloads (None/Text/Json/Binary with base64 encoding), optional channel targeting, metadata HashMap, ref_id for request/response correlation, and millisecond timestamps. Builder pattern via with_channel(), with_ref(), with_metadata(). Factory methods: event(), request(), response(), error(), agent_task(), stream_chunk(), stream_end().</description>
    </pattern>
  </key-traits-and-patterns>

  <coding-conventions>
    <edition>Rust 2021</edition>
    <async-runtime>tokio (full features)</async-runtime>
    <async-traits>Use async-trait crate for async trait methods</async-traits>
    <error-handling>Custom MtwError enum via thiserror; propagate with Result and ? operator</error-handling>
    <serialization>serde with derive; use #[serde(rename_all = "snake_case")] on enums</serialization>
    <concurrency>DashMap for concurrent maps; tokio::sync::RwLock for ordered access; Arc for shared ownership; mpsc::unbounded_channel for event streams</concurrency>
    <logging>tracing crate (tracing::info!, tracing::debug!, tracing::error!) with structured fields</logging>
    <ids>ULID (universally unique lexicographically sortable identifiers) for message IDs</ids>
    <naming>
      <item>Crate names: mtw-* (kebab-case)</item>
      <item>Trait names: Mtw* prefix (MtwModule, MtwTransport, MtwMiddleware)</item>
      <item>Enum variants: PascalCase</item>
      <item>Config structs: *Config suffix</item>
      <item>Builder pattern: with_* methods returning Self</item>
    </naming>
    <testing>
      <item>Tests live in #[cfg(test)] mod tests at the bottom of each source file</item>
      <item>Use #[tokio::test] for async tests</item>
      <item>Test modules create lightweight mock implementations of traits</item>
      <item>Use factory functions (e.g., make_manager(), make_ctx()) for test setup</item>
    </testing>
    <defaults>Implement Default trait for key structs; use serde(default) with named default functions for config</defaults>
    <documentation>Doc comments (///) on public traits, methods, structs, and fields</documentation>
  </coding-conventions>

  <implementation-status>
    <phase name="Phase 1 - Foundation" status="COMPLETE">
      <done>mtw-protocol: MtwMessage, MsgType, Payload, ConnId, TransportEvent, ConnMetadata, AuthInfo, DisconnectReason, wire frame</done>
      <done>mtw-core: MtwModule trait, ModuleRegistry, ModuleManifest, ModuleType, Permission, SharedState, HealthStatus, ModuleContext</done>
      <done>mtw-core: MtwConfig from mtw.toml with env var expansion, ServerConfig, TransportConfig, WebSocketConfig, HttpConfig, CodecConfig</done>
      <done>mtw-core: LifecycleHooks trait, HookRegistry (on_connect, on_disconnect, before_message, after_message, on_error)</done>
      <done>mtw-core: Server orchestration (server.rs)</done>
      <done>mtw-codec: JSON codec</done>
      <done>mtw-transport: MtwTransport trait, WebSocket implementation (tokio-tungstenite)</done>
      <done>mtw-router: Channel pub/sub with history and glob matching, ChannelManager</done>
      <done>mtw-router: MtwMiddleware trait, MiddlewareChain with priority ordering</done>
      <done>mtw-router: Message router</done>
    </phase>

    <phase name="Phase 2 - AI and Auth" status="PLANNED">
      <planned>mtw-ai: MtwAIProvider trait, streaming, agent runtime</planned>
      <planned>AI providers: Anthropic (Claude), OpenAI, Ollama</planned>
      <planned>mtw-auth: JWT, API keys, OAuth2</planned>
      <planned>mtw-state: in-memory state store, Redis adapter</planned>
      <planned>Agent system: MtwAgent trait, AgentOrchestrator, tool calling, agent memory</planned>
    </phase>

    <phase name="Phase 3 - Ecosystem" status="PLANNED">
      <planned>mtw-registry: marketplace client, dependency resolution</planned>
      <planned>mtw-sdk: proc macros (#[mtw_module], #[mtw_handler], #[mtw_agent], #[mtw_config])</planned>
      <planned>mtw-test: testing harness, mock transport, mock agent</planned>
      <planned>WASM sandbox for untrusted modules (wasmtime)</planned>
      <planned>Registry backend and web UI</planned>
      <planned>mtw publish flow</planned>
    </phase>

    <phase name="Phase 4 - Bindings and Scale" status="PLANNED">
      <planned>Node.js binding (NAPI-RS) -- bindings/node/</planned>
      <planned>Python binding (PyO3) -- bindings/python/</planned>
      <planned>PHP binding (FFI) -- bindings/php/</planned>
      <planned>WASM binding for browser -- bindings/wasm/</planned>
      <planned>Frontend SDKs: @mtw/client, @mtw/react, @mtw/svelte, @mtw/vue, @mtw/three -- packages/</planned>
      <planned>Multi-node clustering</planned>
      <planned>QUIC transport</planned>
    </phase>
  </implementation-status>

  <workspace-dependencies>
    <dependency name="tokio" version="1" features="full">Async runtime</dependency>
    <dependency name="async-trait" version="0.1">Async trait support</dependency>
    <dependency name="futures" version="0.3">Future combinators</dependency>
    <dependency name="pin-project-lite" version="0.2">Pin projection</dependency>
    <dependency name="serde" version="1" features="derive">Serialization</dependency>
    <dependency name="serde_json" version="1">JSON support</dependency>
    <dependency name="tokio-tungstenite" version="0.24" features="native-tls">WebSocket</dependency>
    <dependency name="thiserror" version="2">Error derive macro</dependency>
    <dependency name="tracing" version="0.1">Structured logging</dependency>
    <dependency name="tracing-subscriber" version="0.3" features="env-filter">Log output</dependency>
    <dependency name="ulid" version="1">Sortable unique IDs</dependency>
    <dependency name="dashmap" version="6">Concurrent HashMap</dependency>
    <dependency name="bytes" version="1">Byte buffer utilities</dependency>
    <dependency name="toml" version="0.8">TOML config parsing</dependency>
  </workspace-dependencies>

  <directory-layout>
    <dir path="crates/">Rust workspace crates (mtw-protocol, mtw-core, mtw-codec, mtw-transport, mtw-router)</dir>
    <dir path="bindings/">Language binding stubs (node, python, php, wasm) -- Phase 4</dir>
    <dir path="packages/">Frontend SDK stubs (client, react, svelte, vue, three) -- Phase 4</dir>
    <file path="Cargo.toml">Workspace root with shared dependencies</file>
    <file path="Cargo.lock">Dependency lockfile</file>
    <file path="ARCHITECTURE.md">Full architecture design document with trait definitions, roadmap, and examples</file>
  </directory-layout>

  <configuration>
    <format>TOML (mtw.toml)</format>
    <sections>
      <section name="[server]">host, port, max_connections</section>
      <section name="[transport]">default transport, [transport.websocket] (path, ping_interval, max_message_size), [transport.http] (enabled, prefix)</section>
      <section name="[codec]">default codec, binary_channels list</section>
      <section name="[[modules]]">name, version, config map, enabled flag</section>
      <section name="[[agents]]">name, provider, model, system prompt, tools, channels, max_concurrent</section>
      <section name="[[channels]]">name (supports globs), auth, max_members, history, codec</section>
      <section name="[orchestrator]">strategy (channel-based, ai-router, pipeline, fan-out)</section>
    </sections>
    <env-vars>Use ${ENV_VAR} syntax in TOML string values for environment variable expansion</env-vars>
    <defaults>host=0.0.0.0, port=7741, max_connections=10000, transport=websocket, ws_path=/ws, ping_interval=30, codec=json</defaults>
  </configuration>
</project>
