# Protocol Guide

This guide documents the mtwRequest wire protocol -- the message format used for all communication between clients and servers, including JSON encoding, binary framing, streaming, and AI agent interactions.

---

## The MtwMessage Format

Every message exchanged between client and server uses the `MtwMessage` struct, defined in `crates/mtw-protocol/src/message.rs`:

```rust
pub struct MtwMessage {
    pub id: String,                              // Unique ULID
    pub msg_type: MsgType,                       // Message type (renamed to "type" on wire)
    pub channel: Option<String>,                 // Target channel (optional)
    pub payload: Payload,                        // Message content
    pub metadata: HashMap<String, Value>,        // Arbitrary key-value metadata
    pub timestamp: u64,                          // Unix timestamp in milliseconds
    pub ref_id: Option<String>,                  // Reference to another message ID
}
```

### JSON Wire Format

On the wire, `MtwMessage` serializes to JSON with `msg_type` renamed to `type`:

```json
{
  "id": "01HX7Z3K0N8M6RVCD2F5GQWT4P",
  "type": "publish",
  "channel": "chat.general",
  "payload": {
    "kind": "Json",
    "data": { "user": "alice", "text": "Hello everyone!" }
  },
  "metadata": {},
  "timestamp": 1711843200000
}
```

### Builder Pattern

Messages are constructed using factory methods and builder-style `with_*` methods:

```rust
// Simple event
let msg = MtwMessage::event("hello world");

// Message on a channel
let msg = MtwMessage::event("hello").with_channel("chat.general");

// Request expecting a response
let req = MtwMessage::request(Payload::Json(json!({"action": "get_users"})));

// Response correlated to a request
let res = MtwMessage::response(&req.id, Payload::Json(json!({"users": []})));

// Error message
let err = MtwMessage::error(404, "not found");

// Agent task
let task = MtwMessage::agent_task("assistant", "explain this code");

// Streaming chunk
let chunk = MtwMessage::stream_chunk(&task.id, "partial response text");
let end = MtwMessage::stream_end(&task.id);

// Attach metadata
let msg = MtwMessage::event("hello")
    .with_metadata("source", json!("api"))
    .with_metadata("priority", json!(1));
```

---

## Message Types

The `MsgType` enum defines all message types, organized into five groups:

### Transport Lifecycle

| Type | Direction | Description |
|------|-----------|-------------|
| `Connect` | client -> server | Connection handshake |
| `Disconnect` | client -> server | Graceful disconnect |
| `Ping` | either | Keep-alive ping |
| `Pong` | either | Keep-alive response |

### Data Exchange

| Type | Direction | Description |
|------|-----------|-------------|
| `Request` | client -> server | Request expecting a response |
| `Response` | server -> client | Response to a request (uses `ref_id`) |
| `Event` | either | One-way event (no response expected) |
| `Stream` | server -> client | Streaming data chunk (uses `ref_id`) |
| `StreamEnd` | server -> client | End-of-stream marker (uses `ref_id`) |

### Channel Operations

| Type | Direction | Description |
|------|-----------|-------------|
| `Subscribe` | client -> server | Subscribe to a channel |
| `Unsubscribe` | client -> server | Unsubscribe from a channel |
| `Publish` | either | Publish a message to a channel |

### AI Agent

| Type | Direction | Description |
|------|-----------|-------------|
| `AgentTask` | client -> server | Send a task to an AI agent |
| `AgentChunk` | server -> client | Streaming agent response chunk |
| `AgentToolCall` | server -> client | Agent requests a tool invocation |
| `AgentToolResult` | client -> server | Tool result back to agent |
| `AgentComplete` | server -> client | Agent finished processing |

### System

| Type | Direction | Description |
|------|-----------|-------------|
| `Error` | server -> client | Error message with code and description |
| `Ack` | server -> client | Acknowledgment |

---

## Payload Variants

The `Payload` enum uses tagged serialization (`#[serde(tag = "kind", content = "data")]`):

| Variant | Wire Format | Use Case |
|---------|------------|----------|
| `None` | `{"kind": "None"}` | Empty payload (ping, ack, subscribe) |
| `Text(String)` | `{"kind": "Text", "data": "hello"}` | Simple text messages |
| `Json(Value)` | `{"kind": "Json", "data": {...}}` | Structured data |
| `Binary(Vec<u8>)` | `{"kind": "Binary", "data": "base64..."}` | Binary data (base64-encoded) |

```rust
// Payload helper methods
payload.as_text()   // -> Option<&str>
payload.as_json()   // -> Option<&Value>
payload.as_binary() // -> Option<&[u8]>
payload.is_none()   // -> bool
```

---

## Binary Frame Format

For binary transport efficiency, mtwRequest defines a frame format in `crates/mtw-protocol/src/frame.rs`:

```
+--------+---------+------------+--------------+---------+
| MAGIC  | VERSION | FRAME_TYPE | PAYLOAD_LEN  | PAYLOAD |
| 3 bytes| 1 byte  | 1 byte     | 4 bytes (BE) | N bytes |
+--------+---------+------------+--------------+---------+
  'M''T''W'   0x01    0x01-0x04    big-endian u32
```

**Header: 9 bytes total**

### Frame Types

| ID | Name | Description |
|----|------|-------------|
| `0x01` | Json | JSON-encoded MtwMessage |
| `0x02` | Binary | Raw binary data (3D, audio) |
| `0x03` | Ping | Keep-alive ping |
| `0x04` | Pong | Keep-alive response |

### Encoding/Decoding

```rust
use mtw_protocol::frame::Frame;

// Encode a message
let bytes = Frame::encode_message(&msg)?;    // MtwMessage -> Bytes

// Encode raw binary
let bytes = Frame::encode_binary(&data)?;    // &[u8] -> Bytes

// Ping/pong
let ping = Frame::encode_ping();
let pong = Frame::encode_pong();

// Decode
let (frame_type, payload) = Frame::decode(bytes)?;
let msg = Frame::decode_message(bytes)?;     // Bytes -> MtwMessage
```

### Limits

- Maximum frame payload: 10 MB (`MAX_FRAME_SIZE = 10 * 1024 * 1024`)
- Protocol version: 1 (`PROTOCOL_VERSION = 1`)
- Magic bytes: `[0x4D, 0x54, 0x57]` ("MTW")

The JavaScript client (`packages/client/src/connection.ts`) implements the same frame format, ensuring binary compatibility between Rust server and JS clients.

---

## Request/Response Correlation

Request/response pairs are correlated using the `ref_id` field:

```
Client                          Server
  |                                |
  |--- Request (id: "abc123") --->|
  |                                |
  |<-- Response (ref_id: "abc123")|
  |                                |
```

JSON example:

```json
// Request
{
  "id": "01HX7Z3K0N8M6RVCD2F5GQWT4P",
  "type": "request",
  "payload": { "kind": "Json", "data": { "action": "get_users" } },
  "metadata": {},
  "timestamp": 1711843200000
}

// Response
{
  "id": "01HX7Z3K0P9N7SWDE3G6HRXU5Q",
  "type": "response",
  "ref_id": "01HX7Z3K0N8M6RVCD2F5GQWT4P",
  "payload": { "kind": "Json", "data": { "users": ["alice", "bob"] } },
  "metadata": {},
  "timestamp": 1711843200050
}
```

The client SDK uses `ref_id` to resolve pending `Promise`s for request/response pairs (see `packages/client/src/connection.ts`, `pendingRequests` map).

---

## Streaming Protocol

Streaming responses use a sequence of `Stream` chunks followed by a `StreamEnd` marker, all sharing the same `ref_id`:

```
Client                          Server
  |                                |
  |--- Request (id: "req1") ----->|
  |                                |
  |<-- Stream (ref_id: "req1") ---|  "Hello "
  |<-- Stream (ref_id: "req1") ---|  "world "
  |<-- Stream (ref_id: "req1") ---|  "from "
  |<-- Stream (ref_id: "req1") ---|  "mtwRequest!"
  |<-- StreamEnd (ref_id: "req1")-|
  |                                |
```

```json
// Stream chunk
{
  "id": "01HX...",
  "type": "stream",
  "ref_id": "req1",
  "payload": { "kind": "Text", "data": "Hello " },
  "timestamp": 1711843200100
}

// Stream end
{
  "id": "01HX...",
  "type": "stream_end",
  "ref_id": "req1",
  "payload": { "kind": "None" },
  "timestamp": 1711843200500
}
```

---

## AI Agent Protocol

Agent interactions use a dedicated message flow:

### Simple Request/Response

```
Client                               Server (Agent)
  |                                       |
  |--- AgentTask (id: "task1") --------->|
  |    agent: "assistant"                 |
  |    content: "explain this code"       |
  |                                       |  (LLM processes...)
  |<-- AgentComplete (ref_id: "task1") --|
  |    content: "This code does..."       |
```

### Streaming Response

```
Client                               Server (Agent)
  |                                       |
  |--- AgentTask (id: "task1") --------->|
  |                                       |
  |<-- AgentChunk (ref_id: "task1") -----|  "This "
  |<-- AgentChunk (ref_id: "task1") -----|  "code "
  |<-- AgentChunk (ref_id: "task1") -----|  "does..."
  |<-- AgentComplete (ref_id: "task1") --|
```

### Tool Calling

```
Client                               Server (Agent)
  |                                       |
  |--- AgentTask (id: "task1") --------->|
  |                                       |  (LLM decides to use a tool)
  |<-- AgentToolCall (ref_id: "task1") --|
  |    tool: "search"                     |
  |    args: {"query": "mtwRequest"}      |
  |                                       |
  |--- AgentToolResult ----------------->|  (Client executes the tool)
  |    tool_call_id: "tc1"                |
  |    result: "mtwRequest is a..."       |
  |                                       |  (LLM continues with tool result)
  |<-- AgentChunk (ref_id: "task1") -----|
  |<-- AgentComplete (ref_id: "task1") --|
```

### AgentTask JSON

```json
{
  "id": "01HX...",
  "type": "agent_task",
  "payload": { "kind": "Text", "data": "explain this code" },
  "metadata": {
    "agent": "assistant"
  },
  "timestamp": 1711843200000
}
```

### AgentToolCall JSON

```json
{
  "id": "01HX...",
  "type": "agent_tool_call",
  "ref_id": "01HX...",
  "payload": {
    "kind": "Json",
    "data": {
      "id": "tc-001",
      "name": "search",
      "arguments": { "query": "mtwRequest docs" }
    }
  },
  "timestamp": 1711843200200
}
```

### AgentToolResult JSON

```json
{
  "id": "01HX...",
  "type": "agent_tool_result",
  "ref_id": "01HX...",
  "payload": {
    "kind": "Json",
    "data": {
      "tool_call_id": "tc-001",
      "name": "search",
      "result": { "text": "mtwRequest is a real-time framework..." },
      "is_error": false
    }
  },
  "timestamp": 1711843200300
}
```

---

## Channel Operations

### Subscribe

```json
{
  "id": "01HX...",
  "type": "subscribe",
  "channel": "chat.general",
  "payload": { "kind": "None" },
  "metadata": {},
  "timestamp": 1711843200000
}
```

Server responds:

```json
{
  "id": "01HX...",
  "type": "response",
  "ref_id": "01HX...",
  "payload": {
    "kind": "Json",
    "data": { "subscribed": "chat.general", "members": 5 }
  },
  "timestamp": 1711843200010
}
```

### Publish

```json
{
  "id": "01HX...",
  "type": "publish",
  "channel": "chat.general",
  "payload": {
    "kind": "Json",
    "data": { "user": "alice", "text": "Hello!" }
  },
  "metadata": {},
  "timestamp": 1711843200100
}
```

---

## Connection Metadata

When a client connects, the server assigns metadata:

```rust
pub struct ConnMetadata {
    pub conn_id: ConnId,              // ULID
    pub remote_addr: Option<String>,  // "127.0.0.1:54321"
    pub user_agent: Option<String>,
    pub auth: Option<AuthInfo>,       // If authenticated
    pub connected_at: u64,            // Unix timestamp (ms)
}
```

### Disconnect Reasons

```rust
pub enum DisconnectReason {
    Normal,                // Clean close
    Timeout,               // Ping/pong timeout
    Error(String),         // Connection error
    Kicked(String),        // Removed by server/admin
    ServerShutdown,        // Server is shutting down
}
```

---

## TypeScript Types

The `@mtw/client` package (`packages/client/src/types.ts`) mirrors all Rust types exactly:

```typescript
type MsgType =
  | 'connect' | 'disconnect' | 'ping' | 'pong'
  | 'request' | 'response' | 'event' | 'stream' | 'stream_end'
  | 'subscribe' | 'unsubscribe' | 'publish'
  | 'agent_task' | 'agent_chunk' | 'agent_tool_call'
  | 'agent_tool_result' | 'agent_complete'
  | 'error' | 'ack';

type Payload =
  | { kind: 'None' }
  | { kind: 'Text'; data: string }
  | { kind: 'Json'; data: unknown }
  | { kind: 'Binary'; data: string };  // base64

interface MtwMessage {
  id: string;
  type: MsgType;
  channel?: string;
  payload: Payload;
  metadata: Record<string, unknown>;
  timestamp: number;
  ref_id?: string;
}
```

---

## Next Steps

- [Channels Guide](./channels-guide.md) -- channel pub/sub in depth
- [AI Agents Guide](./ai-agents-guide.md) -- agent protocol details
- [Frontend Guide](./frontend-guide.md) -- using the protocol from JavaScript
