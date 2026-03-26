# Getting Started with mtwRequest

## What is mtwRequest?

mtwRequest is a Rust-based modular real-time framework that unifies WebSocket communication, HTTP APIs, and AI agents into a single high-performance core. It provides:

- **Real-time WebSocket server** with pub/sub channels, middleware, and presence tracking
- **AI-native design** with first-class support for streaming LLM responses, tool calling, and multi-agent orchestration
- **Module marketplace** -- install, create, and share modules (auth, AI providers, integrations)
- **Polyglot bindings** -- use from Rust, Node.js, Python, PHP, or the browser (WASM)
- **Frontend SDKs** -- React, Svelte, Vue, and Three.js hooks out of the box

---

## Installation

### Rust (server-side)

Add the crates you need to your `Cargo.toml`:

```toml
[dependencies]
mtw-core = "0.1"
mtw-protocol = "0.1"
mtw-transport = "0.1"
mtw-router = "0.1"
mtw-codec = "0.1"

# Optional -- add as needed:
mtw-ai = "0.1"          # AI providers and agents
mtw-auth = "0.1"        # JWT, API keys, OAuth2
mtw-state = "0.1"       # State store (memory, Redis)
mtw-sdk = "0.1"         # Module development SDK
mtw-http = "0.1"        # HTTP client with pipeline
mtw-integrations = "0.1" # Third-party API integrations

# Async runtime (required)
tokio = { version = "1", features = ["full"] }
```

### JavaScript / TypeScript (client-side)

```bash
npm install @mtw/client          # Universal JS client
npm install @mtw/react           # React hooks
npm install @mtw/svelte          # Svelte stores
npm install @mtw/vue             # Vue composables
npm install @mtw/three           # Three.js sync
```

### Python

```bash
pip install mtw-request
```

### PHP

```bash
composer require mtw/request
```

---

## Quick Start: Create a Server in Rust

This is the minimal server that accepts WebSocket connections and handles pub/sub channels:

```rust
// examples/demo_server.rs
use mtw_core::MtwServerBuilder;
use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;
use mtw_router::{ChannelManager, MiddlewareChain, MtwRouter};
use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let addr: SocketAddr = "127.0.0.1:8080".parse()?;

    // 1. Create the WebSocket transport
    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut event_rx = transport.take_event_receiver().unwrap();
    transport.listen(addr).await?;

    // 2. Create the channel manager
    let mut channel_mgr = ChannelManager::new();
    let mut channel_rx = channel_mgr.take_message_receiver().unwrap();
    channel_mgr.create_channel("chat.general", false, Some(100), 50);

    let router = Arc::new(MtwRouter::new(channel_mgr, MiddlewareChain::new()));
    let transport = Arc::new(transport);

    // 3. Forward published messages to subscribers
    let transport_fwd = transport.clone();
    tokio::spawn(async move {
        while let Some((conn_id, msg)) = channel_rx.recv().await {
            let _ = transport_fwd.send(&conn_id, msg).await;
        }
    });

    // 4. Main event loop
    tracing::info!("Server running on ws://{}", addr);
    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match &event {
                    TransportEvent::Connected(id, _) => {
                        tracing::info!(conn = %id, "connected");
                    }
                    TransportEvent::Disconnected(id, _) => {
                        router.channels().remove_connection(id);
                    }
                    TransportEvent::Message(id, msg) => {
                        match &msg.msg_type {
                            MsgType::Subscribe => {
                                if let Some(ch) = &msg.channel {
                                    let _ = router.channels().subscribe(ch, id);
                                }
                            }
                            MsgType::Publish => {
                                if let Some(ch_name) = &msg.channel {
                                    if let Some(ch) = router.channels().get(ch_name) {
                                        ch.publish(msg.clone(), Some(id)).await.ok();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                transport.shutdown().await?;
                break;
            }
        }
    }
    Ok(())
}
```

Run with:

```bash
cargo run --example demo_server
```

---

## Quick Start: Connect from React

```tsx
import { MtwProvider, useChannel, useAgent } from '@mtw/react';

function App() {
  return (
    <MtwProvider url="ws://localhost:8080/ws">
      <ChatRoom />
    </MtwProvider>
  );
}

function ChatRoom() {
  const { messages, publish, members, subscribed } = useChannel("chat.general");
  const { send, messages: aiMessages, isStreaming } = useAgent("assistant");

  return (
    <div>
      <p>Members: {members.length} | Subscribed: {subscribed ? 'yes' : 'no'}</p>

      {messages.map(msg => (
        <div key={msg.id}>
          {msg.payload.kind === 'Text' ? msg.payload.data : JSON.stringify(msg.payload)}
        </div>
      ))}

      <button onClick={() => publish("Hello from React!")}>Send</button>
      <button onClick={() => send("What is mtwRequest?")}>Ask AI</button>

      {isStreaming && <p>AI is typing...</p>}
    </div>
  );
}
```

---

## Quick Start: Connect from Node.js

```typescript
import { MtwConnection, createMessage, textPayload } from '@mtw/client';

const conn = new MtwConnection({ url: 'ws://localhost:8080/ws' });

conn.on('connected', (meta) => {
  console.log('Connected:', meta.conn_id);

  // Subscribe to a channel
  conn.send(createMessage('subscribe', { kind: 'None' }, { channel: 'chat.general' }));

  // Publish a message
  conn.send(createMessage('publish', textPayload('Hello from Node.js!'), {
    channel: 'chat.general',
  }));
});

conn.on('message', (msg) => {
  console.log('Received:', msg.type, msg.payload);
});

await conn.connect();
```

---

## Quick Start: Connect from Python

```python
import mtw_request

# Create a connection
conn = mtw_request.Connection("ws://localhost:8080/ws")
conn.connect()

# Subscribe to a channel
conn.subscribe("chat.general")

# Publish a message
conn.publish("chat.general", {"text": "Hello from Python!"})

# Listen for messages
@conn.on("message")
def handle_message(msg):
    print(f"Received: {msg['type']} - {msg['payload']}")

conn.run()
```

---

## Project Structure Overview

```
mtw-request/
+-- Cargo.toml                    # Workspace root
+-- mtw.toml                      # Server configuration (optional)
|
+-- crates/                       # Rust workspace crates
|   +-- mtw-protocol/             # Wire protocol: MtwMessage, MsgType, Payload
|   +-- mtw-core/                 # Kernel: module system, config, hooks, server
|   +-- mtw-codec/                # Serialization: JSON (+ MsgPack, Protobuf planned)
|   +-- mtw-transport/            # Transport abstraction + WebSocket impl
|   +-- mtw-router/               # Channels, middleware chain, message routing
|   +-- mtw-ai/                   # AI providers, agents, orchestrator, tool calling
|   +-- mtw-auth/                 # JWT, API keys, OAuth2, auth middleware
|   +-- mtw-state/                # State store trait + memory/Redis backends
|   +-- mtw-http/                 # HTTP client with response pipeline
|   +-- mtw-integrations/         # 20 API integrations + 10 AI providers + OAuth2
|   +-- mtw-registry/             # Module marketplace client + manifest parser
|   +-- mtw-sdk/                  # SDK for module developers (prelude, builders)
|   +-- mtw-test/                 # Test harness, mocks
|
+-- packages/                     # Frontend SDKs (TypeScript)
|   +-- client/                   # @mtw/client -- universal WebSocket client
|   +-- react/                    # @mtw/react -- React hooks
|   +-- svelte/                   # @mtw/svelte -- Svelte stores
|   +-- vue/                      # @mtw/vue -- Vue composables
|   +-- three/                    # @mtw/three -- Three.js real-time sync
|
+-- bindings/                     # Language bindings
|   +-- node/                     # NAPI-RS (Node.js)
|   +-- python/                   # PyO3 (Python)
|   +-- php/                      # PHP FFI
|   +-- wasm/                     # WASM (browser)
|
+-- examples/                     # Working examples
|   +-- demo_server.rs            # WebSocket server with channels
|   +-- demo_client.rs            # WebSocket client demo
|
+-- docs/                         # This documentation
```

---

## Building and Testing

```bash
# Build all crates
cargo build

# Run all tests (38 tests across all crates)
cargo test

# Test a single crate
cargo test -p mtw-core

# Check without building
cargo check

# Lint
cargo clippy

# Format
cargo fmt
```

---

## Next Steps

- [Server Guide](./server-guide.md) -- configure and run a production server
- [Modules Guide](./modules-guide.md) -- create your own modules
- [AI Agents Guide](./ai-agents-guide.md) -- build AI-powered features
- [Frontend Guide](./frontend-guide.md) -- integrate with React/Svelte/Vue
