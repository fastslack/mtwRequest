// =============================================================================
// mtwRequest — NAPI-RS Binding for Node.js
// =============================================================================
//
// This module provides the Node.js native addon surface for mtwRequest.
// It uses NAPI-RS to expose Rust structs and async functions directly to
// JavaScript/TypeScript without serialization overhead.
//
// Build: napi build --platform --release
//
// When napi and napi-derive are available as dependencies, uncomment the
// attribute macros below. The code compiles as a design reference in the
// meantime.
// =============================================================================

// #[macro_use]
// extern crate napi_derive;

pub mod ws;

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Re-export protocol types (these map 1:1 to the JS-facing API)
// ---------------------------------------------------------------------------
// use mtw_protocol::{MtwMessage, MsgType, Payload};

// ---------------------------------------------------------------------------
// MtwClient — top-level connection handle exposed to JS
// ---------------------------------------------------------------------------
//
// JS usage:
//   const client = await connect("ws://localhost:8080/ws", { token: "..." });
//   client.send("chat.general", { text: "hello" });
//   client.close();

// #[napi]
/// MtwClient wraps a persistent WebSocket connection to an mtwRequest server.
/// All channel subscriptions, agent interactions, and RPC calls go through this
/// handle. The client manages automatic reconnection, ping/pong keep-alive, and
/// message dispatching to registered listeners.
pub struct MtwClient {
    /// WebSocket endpoint URL (e.g. "ws://localhost:8080/ws")
    url: String,
    /// Authentication token or API key
    auth_token: Option<String>,
    /// Active channel subscriptions keyed by channel name
    subscriptions: HashMap<String, ChannelSubscription>,
    /// Whether the underlying socket is currently connected
    connected: bool,
    /// Unique connection ID assigned by the server
    conn_id: Option<String>,
}

/// Internal tracking for a channel subscription
struct ChannelSubscription {
    channel: String,
    active: bool,
}

// #[napi]
impl MtwClient {
    // ----- Connection lifecycle -----

    /// Create a new MtwClient and connect to the server.
    ///
    /// JS signature:
    ///   static async connect(url: string, options?: ConnectOptions): Promise<MtwClient>
    ///
    /// Options:
    ///   - token: string        — Bearer token for authentication
    ///   - apiKey: string       — API key for authentication
    ///   - reconnect: boolean   — Enable auto-reconnect (default: true)
    ///   - pingInterval: number — Ping interval in seconds (default: 30)
    ///
    // #[napi(factory)]
    pub async fn connect(url: String, token: Option<String>) -> Self {
        // In the real implementation this would:
        // 1. Open a tokio-tungstenite WebSocket connection
        // 2. Send a Connect message with auth credentials
        // 3. Wait for the Ack response containing the connection ID
        // 4. Spawn a background task for ping/pong and message dispatch
        MtwClient {
            url,
            auth_token: token,
            subscriptions: HashMap::new(),
            connected: false,
            conn_id: None,
        }
    }

    /// Close the connection gracefully.
    ///
    /// JS signature:
    ///   async close(): Promise<void>
    ///
    // #[napi]
    pub async fn close(&mut self) {
        // Send Disconnect message, await Ack, drop the socket
        self.connected = false;
    }

    /// Check whether the client is currently connected.
    ///
    /// JS signature:
    ///   get connected(): boolean
    ///
    // #[napi(getter)]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Get the server-assigned connection ID, if connected.
    ///
    /// JS signature:
    ///   get connectionId(): string | null
    ///
    // #[napi(getter)]
    pub fn connection_id(&self) -> Option<String> {
        self.conn_id.clone()
    }

    // ----- Messaging -----

    /// Send a message on a channel.
    ///
    /// JS signature:
    ///   async send(channel: string, payload: string | object | Buffer): Promise<void>
    ///
    /// The payload is automatically wrapped in the appropriate Payload variant:
    ///   - string  -> Payload::Text
    ///   - object  -> Payload::Json (serialized)
    ///   - Buffer  -> Payload::Binary (base64-encoded on the wire)
    ///
    // #[napi]
    pub async fn send(&self, _channel: String, _payload: String) {
        // Build an MtwMessage { msg_type: Publish, channel, payload: Text(payload) }
        // Encode as Frame::encode_message and write to WebSocket
    }

    /// Send a request and wait for a correlated response.
    ///
    /// JS signature:
    ///   async request(channel: string, payload: any, timeoutMs?: number): Promise<MtwMessage>
    ///
    // #[napi]
    pub async fn request(&self, _channel: String, _payload: String) -> String {
        // Build MtwMessage { msg_type: Request }, send, await Response with matching ref_id
        String::new()
    }

    // ----- Channel operations -----

    /// Subscribe to a channel. Returns an MtwChannel handle.
    ///
    /// JS signature:
    ///   async subscribe(channel: string): Promise<MtwChannel>
    ///
    // #[napi]
    pub async fn subscribe(&mut self, channel: String) -> MtwChannel {
        // Send Subscribe message, await Ack
        self.subscriptions.insert(
            channel.clone(),
            ChannelSubscription {
                channel: channel.clone(),
                active: true,
            },
        );
        MtwChannel {
            name: channel,
            active: true,
        }
    }

    /// Unsubscribe from a channel.
    ///
    /// JS signature:
    ///   async unsubscribe(channel: string): Promise<void>
    ///
    // #[napi]
    pub async fn unsubscribe(&mut self, channel: String) {
        // Send Unsubscribe message, await Ack
        self.subscriptions.remove(&channel);
    }

    // ----- Agent operations -----

    /// Create an agent interaction handle.
    ///
    /// JS signature:
    ///   createAgent(name: string): MtwAgent
    ///
    // #[napi]
    pub fn create_agent(&self, name: String) -> MtwAgent {
        MtwAgent {
            name,
            streaming: false,
        }
    }
}

// ---------------------------------------------------------------------------
// MtwChannel — channel subscription handle
// ---------------------------------------------------------------------------
//
// JS usage:
//   const channel = await client.subscribe("chat.general");
//   channel.onMessage((msg) => console.log(msg));
//   channel.publish({ text: "hello" });
//   channel.unsubscribe();

// #[napi]
/// MtwChannel represents an active subscription to a named channel.
/// Messages published to this channel by any client (including this one) will
/// be delivered to registered message handlers.
pub struct MtwChannel {
    /// Channel name (e.g. "chat.general", "3d-sync")
    name: String,
    /// Whether this subscription is still active
    active: bool,
}

// #[napi]
impl MtwChannel {
    /// Get the channel name.
    ///
    /// JS signature:
    ///   get name(): string
    ///
    // #[napi(getter)]
    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    /// Register a callback for incoming messages on this channel.
    ///
    /// JS signature:
    ///   onMessage(callback: (msg: MtwMessage) => void): void
    ///
    /// The callback receives deserialized MtwMessage objects. Multiple
    /// callbacks can be registered; they are invoked in registration order.
    ///
    // #[napi(ts_args_type = "callback: (msg: MtwMessage) => void")]
    pub fn on_message(&self) {
        // In real implementation: register a JS callback (napi::JsFunction)
        // that gets called from the message dispatch loop
    }

    /// Publish a message to this channel.
    ///
    /// JS signature:
    ///   async publish(payload: string | object | Buffer): Promise<void>
    ///
    // #[napi]
    pub async fn publish(&self, _payload: String) {
        // Build MtwMessage { msg_type: Publish, channel: self.name, payload }
        // and send through the client's WebSocket connection
    }

    /// Unsubscribe from this channel.
    ///
    /// JS signature:
    ///   async unsubscribe(): Promise<void>
    ///
    // #[napi]
    pub async fn unsubscribe(&mut self) {
        self.active = false;
    }

    /// Check if the subscription is active.
    ///
    /// JS signature:
    ///   get active(): boolean
    ///
    // #[napi(getter)]
    pub fn is_active(&self) -> bool {
        self.active
    }
}

// ---------------------------------------------------------------------------
// MtwAgent — AI agent interaction handle
// ---------------------------------------------------------------------------
//
// JS usage:
//   const agent = client.createAgent("assistant");
//   const response = await agent.send("Explain this code");
//
//   // Streaming
//   for await (const chunk of agent.stream("Explain this code")) {
//     process.stdout.write(chunk.text);
//   }
//
//   // With tool calls
//   agent.onToolCall("read_file", async (params) => {
//     return fs.readFileSync(params.path, "utf-8");
//   });

// #[napi]
/// MtwAgent provides a high-level interface for interacting with AI agents
/// running on the mtwRequest server. It handles task submission, streaming
/// responses, and tool call orchestration.
pub struct MtwAgent {
    /// Agent name as registered on the server (e.g. "assistant", "code-reviewer")
    name: String,
    /// Whether a streaming response is currently in progress
    streaming: bool,
}

// #[napi]
impl MtwAgent {
    /// Get the agent name.
    ///
    /// JS signature:
    ///   get name(): string
    ///
    // #[napi(getter)]
    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    /// Send a task to the agent and wait for the complete response.
    ///
    /// JS signature:
    ///   async send(content: string, options?: AgentOptions): Promise<AgentResponse>
    ///
    /// Options:
    ///   - context: Message[]   — conversation history
    ///   - metadata: object     — arbitrary metadata
    ///   - timeout: number      — timeout in ms
    ///
    // #[napi]
    pub async fn send(&self, _content: String) -> String {
        // Build MtwMessage { msg_type: AgentTask, payload: Text(content) }
        // with metadata.agent = self.name
        // Send and collect all AgentChunk messages until AgentComplete
        String::new()
    }

    /// Stream a response from the agent, yielding chunks as they arrive.
    ///
    /// JS signature:
    ///   stream(content: string, options?: AgentOptions): AsyncIterable<AgentChunk>
    ///
    /// Each chunk contains:
    ///   - text: string         — the text content of this chunk
    ///   - done: boolean        — whether this is the final chunk
    ///   - toolCall?: ToolCall  — if the agent wants to call a tool
    ///
    // #[napi]
    pub fn stream(&mut self, _content: String) {
        // Returns a NAPI AsyncIterator that yields AgentChunk objects.
        // Internally sends AgentTask and listens for AgentChunk messages
        // with matching ref_id until AgentComplete is received.
        self.streaming = true;
    }

    /// Register a tool handler. When the agent requests this tool,
    /// the callback is invoked and its return value is sent back.
    ///
    /// JS signature:
    ///   onToolCall(toolName: string, handler: (params: object) => Promise<any>): void
    ///
    // #[napi(ts_args_type = "toolName: string, handler: (params: object) => Promise<any>")]
    pub fn on_tool_call(&self, _tool_name: String) {
        // Register a JS callback that handles AgentToolCall messages.
        // When triggered:
        // 1. Deserialize the tool call parameters from the message payload
        // 2. Invoke the JS handler
        // 3. Send AgentToolResult back with the handler's return value
    }

    /// Check if a streaming response is in progress.
    ///
    /// JS signature:
    ///   get isStreaming(): boolean
    ///
    // #[napi(getter)]
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }
}

// ---------------------------------------------------------------------------
// Module initialization
// ---------------------------------------------------------------------------
//
// NAPI-RS generates the module init function automatically via the #[napi]
// macro on each exported struct/function. No explicit #[module_exports]
// block is needed with napi-derive v2+.
//
// The generated Node.js addon exposes:
//
//   const { MtwClient, MtwChannel, MtwAgent } = require('./mtw-request.node');
//
//   // Or with the JS wrapper package:
//   import { connect, MtwClient } from '@mtw/node';
//
//   const client = await MtwClient.connect("ws://localhost:8080/ws", "token");
//   const channel = await client.subscribe("chat.general");
//   channel.onMessage((msg) => { ... });
//   channel.publish("hello world");
//
//   const agent = client.createAgent("assistant");
//   const reply = await agent.send("Hello!");
//   console.log(reply);
