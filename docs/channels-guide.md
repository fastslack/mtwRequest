# Channels Guide

Channels are the pub/sub messaging backbone of mtwRequest. Clients subscribe to channels and publish messages that are delivered to all other subscribers in real time.

---

## What Are Channels?

A channel is a named topic that connections can subscribe to. When a message is published to a channel, it is delivered to all subscribers (optionally excluding the sender). Channels support:

- **Glob pattern matching** (`chat.*` matches `chat.general`, `chat.random`)
- **Message history** (ring buffer of recent messages)
- **Maximum member limits**
- **Authentication requirements** (per-channel)
- **Codec overrides** (e.g., binary for 3D/audio channels)

Source: `crates/mtw-router/src/channel.rs`

---

## Creating Channels

### In mtw.toml

```toml
[[channels]]
name = "chat.general"
auth = true
max_members = 100
history = 50

[[channels]]
name = "chat.*"          # Glob pattern -- matches any chat.* channel
auth = true
max_members = 100
history = 50

[[channels]]
name = "3d-sync"
codec = "binary"
auth = true

[[channels]]
name = "notifications"
history = 10
```

### Programmatically

```rust
use mtw_router::ChannelManager;

let mut channel_mgr = ChannelManager::new();

// create_channel(name, auth_required, max_members, history_size)
channel_mgr.create_channel("chat.general", false, Some(100), 50);
channel_mgr.create_channel("chat.random", false, None, 20);
channel_mgr.create_channel("notifications", false, None, 10);
```

### Auto-creation

If a client subscribes to a channel that does not exist, `get_or_create()` will create it with default settings (no auth, no member limit, history of 50):

```rust
let channel = channel_mgr.get_or_create("new-channel");
```

---

## Subscribing and Unsubscribing

### Server-side

```rust
// Subscribe a connection
channel_mgr.subscribe("chat.general", &conn_id)?;

// Unsubscribe
channel_mgr.unsubscribe("chat.general", &conn_id);

// Remove from ALL channels (call on disconnect)
channel_mgr.remove_connection(&conn_id);
```

### Client-side (JavaScript)

```typescript
const conn = new MtwConnection({ url: 'ws://localhost:8080/ws' });
await conn.connect();

// Subscribe
conn.send(createMessage('subscribe', emptyPayload(), { channel: 'chat.general' }));

// Unsubscribe
conn.send(createMessage('unsubscribe', emptyPayload(), { channel: 'chat.general' }));
```

### Client-side (React)

```tsx
function ChatRoom() {
  // Auto-subscribes on mount, auto-unsubscribes on unmount
  const { messages, publish, subscribed } = useChannel("chat.general");

  // Manual control:
  const { subscribe, unsubscribe } = useChannel("chat.general", {
    autoSubscribe: false,
  });
}
```

---

## Publishing Messages

### Server-side

```rust
if let Some(channel) = channel_mgr.get("chat.general") {
    let msg = MtwMessage::new(MsgType::Publish, Payload::Text("Hello!".into()))
        .with_channel("chat.general");

    // Publish to all subscribers, excluding the sender
    let recipients = channel.publish(msg, Some(&sender_conn_id)).await?;
    tracing::info!("delivered to {} subscribers", recipients);
}
```

### Client-side (JavaScript)

```typescript
conn.send(createMessage('publish', textPayload('Hello!'), {
  channel: 'chat.general',
}));
```

### Client-side (React)

```tsx
const { publish } = useChannel("chat.general");

// Text message
publish("Hello everyone!");

// JSON message
publish({ user: "alice", text: "Hello!", emoji: "wave" });
```

---

## Channel Patterns (Glob Matching)

Channels support dot-separated glob patterns using `*` as a wildcard for a single segment:

| Pattern | Matches | Does NOT Match |
|---------|---------|----------------|
| `chat.*` | `chat.general`, `chat.random` | `chat.sub.deep`, `other.channel` |
| `*.*` | `any.thing`, `foo.bar` | `single`, `a.b.c` |
| `exact` | `exact` | `other` |

The matching is implemented in `ChannelManager::glob_match()`:

```rust
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('.').collect();
    let text_parts: Vec<&str> = text.split('.').collect();

    if pattern_parts.len() != text_parts.len() {
        return false;
    }

    pattern_parts.iter().zip(text_parts.iter())
        .all(|(p, t)| *p == "*" || p == t)
}
```

Use `find_matching()` to get all channels matching a pattern:

```rust
let chat_channels = channel_mgr.find_matching("chat.*");
for ch in chat_channels {
    println!("Channel: {} ({} subscribers)", ch.name(), ch.subscriber_count());
}
```

---

## Channel History

Each channel maintains a ring buffer of recent messages. When the buffer is full, the oldest message is removed:

```rust
// Create a channel with history of 50 messages
channel_mgr.create_channel("chat.general", false, None, 50);

// Retrieve history
let channel = channel_mgr.get("chat.general").unwrap();
let last_10 = channel.get_history(Some(10)).await;
let all_history = channel.get_history(None).await;
```

History is stored in a `tokio::sync::RwLock<Vec<MtwMessage>>` and is kept in chronological order. `get_history(Some(n))` returns the N most recent messages.

---

## Maximum Members

Set a member limit to control channel capacity:

```rust
channel_mgr.create_channel("small-room", false, Some(5), 0);

// Subscription will fail when the channel is full
match channel_mgr.subscribe("small-room", &conn_id) {
    Ok(()) => println!("subscribed"),
    Err(e) => println!("error: {}", e),  // "channel 'small-room' is full (max: 5)"
}
```

Set to `None` for unlimited members.

---

## Presence Tracking

The `Subscriber` struct tracks when each connection subscribed:

```rust
pub struct Subscriber {
    pub conn_id: ConnId,
    pub subscribed_at: u64,   // Unix timestamp (ms)
}
```

Get the list of subscribers:

```rust
let channel = channel_mgr.get("chat.general").unwrap();
let members: Vec<ConnId> = channel.subscribers();
let count = channel.subscriber_count();
let is_member = channel.is_subscribed(&conn_id);
```

The React `useChannel` hook provides `members` as reactive state with join/leave callbacks:

```tsx
const { members } = useChannel("chat.general");
// members: ChannelMember[]
```

---

## Binary Channels (3D, Audio)

For high-throughput binary data (3D scene sync, audio streaming), configure a channel to use the binary codec:

```toml
[[channels]]
name = "3d-sync"
codec = "binary"
auth = true
```

Binary data bypasses JSON encoding. On the server:

```rust
transport.send_binary(&conn_id, &raw_bytes).await?;
```

The client receives binary frames through the `Binary` transport event:

```rust
TransportEvent::Binary(conn_id, data) => {
    // data: Vec<u8>
}
```

---

## ChannelManager Reference

| Method | Description |
|--------|-------------|
| `new()` | Create a new manager |
| `take_message_receiver()` | Get the mpsc receiver for published messages |
| `create_channel(name, auth, max, history)` | Create a channel |
| `get_or_create(name)` | Get existing or create with defaults |
| `get(name)` | Get a channel by exact name |
| `find_matching(pattern)` | Find channels matching a glob pattern |
| `subscribe(channel, conn_id)` | Subscribe a connection |
| `unsubscribe(channel, conn_id)` | Unsubscribe a connection |
| `remove_connection(conn_id)` | Remove from all channels (on disconnect) |
| `list_channels()` | List all channel names |
| `delete_channel(name)` | Delete a channel |

## Channel Reference

| Method | Description |
|--------|-------------|
| `name()` | Channel name |
| `auth_required()` | Whether auth is required |
| `subscriber_count()` | Number of active subscribers |
| `subscribe(conn_id)` | Add a subscriber |
| `unsubscribe(conn_id)` | Remove a subscriber |
| `is_subscribed(conn_id)` | Check if subscribed |
| `publish(msg, exclude)` | Publish to all (optionally excluding one) |
| `get_history(limit)` | Get recent messages |
| `subscribers()` | List all subscriber IDs |
| `remove_connection(conn_id)` | Remove tracking for a connection |

---

## Complete Example: Building a Chat App

### Server

```rust
use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;
use mtw_router::ChannelManager;
use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let addr = "127.0.0.1:8080".parse()?;
    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut event_rx = transport.take_event_receiver().unwrap();
    transport.listen(addr).await?;

    let mut channel_mgr = ChannelManager::new();
    let mut channel_rx = channel_mgr.take_message_receiver().unwrap();
    channel_mgr.create_channel("chat.general", false, Some(100), 50);
    channel_mgr.create_channel("chat.random", false, None, 20);

    let transport = Arc::new(transport);

    // Forward published messages to subscribers via transport
    let fwd = transport.clone();
    tokio::spawn(async move {
        while let Some((conn_id, msg)) = channel_rx.recv().await {
            let _ = fwd.send(&conn_id, msg).await;
        }
    });

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    TransportEvent::Connected(id, _) => {
                        tracing::info!("connected: {}", id);
                    }
                    TransportEvent::Disconnected(id, _) => {
                        channel_mgr.remove_connection(&id);
                    }
                    TransportEvent::Message(id, msg) => match msg.msg_type {
                        MsgType::Subscribe => {
                            if let Some(ch) = &msg.channel {
                                let _ = channel_mgr.subscribe(ch, &id);
                            }
                        }
                        MsgType::Publish => {
                            if let Some(ch_name) = &msg.channel {
                                if let Some(ch) = channel_mgr.get(ch_name) {
                                    ch.publish(msg, Some(&id)).await.ok();
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}
```

### React Client

```tsx
import { MtwProvider, useChannel } from '@mtw/react';

function App() {
  return (
    <MtwProvider url="ws://localhost:8080/ws">
      <ChatRoom channel="chat.general" />
    </MtwProvider>
  );
}

function ChatRoom({ channel }: { channel: string }) {
  const { messages, publish, members, subscribed } = useChannel(channel);
  const [input, setInput] = useState('');

  return (
    <div>
      <h2>#{channel} ({members.length} online)</h2>
      <div>
        {messages.map(msg => (
          <p key={msg.id}>
            {msg.payload.kind === 'Text' ? msg.payload.data : JSON.stringify(msg.payload.data)}
          </p>
        ))}
      </div>
      <form onSubmit={(e) => { e.preventDefault(); publish(input); setInput(''); }}>
        <input value={input} onChange={(e) => setInput(e.target.value)} />
        <button type="submit" disabled={!subscribed}>Send</button>
      </form>
    </div>
  );
}
```

---

## Next Steps

- [Protocol Guide](./protocol-guide.md) -- message format details
- [AI Agents Guide](./ai-agents-guide.md) -- agents that listen on channels
- [Frontend Guide](./frontend-guide.md) -- React/Svelte/Vue integration
