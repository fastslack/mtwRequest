// =============================================================================
// mtwRequest — WASM Binding for Browser
// =============================================================================
//
// This module provides a WebAssembly binding for mtwRequest, designed to run
// in the browser. It uses wasm-bindgen to expose async Rust functions as
// JavaScript promises and leverages web-sys for WebSocket access.
//
// Build: wasm-pack build --target web
//
// JS usage (after wasm-pack):
//   import init, { MtwClient, MtwChannel, MtwAgent } from '@mtw/wasm';
//   await init();
//   const client = await MtwClient.connect("ws://localhost:7741/ws", "token");
//
// When wasm-bindgen and web-sys are available as dependencies, uncomment
// the attribute macros. The code compiles as a design reference in the
// meantime.
// =============================================================================

// use wasm_bindgen::prelude::*;
// use wasm_bindgen_futures::JsFuture;
// use web_sys::{WebSocket, MessageEvent};
// use js_sys::{Function, Promise, Uint8Array};

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// MtwClient — browser WebSocket client
// ---------------------------------------------------------------------------
//
// JS usage:
//   const client = await MtwClient.connect("ws://localhost:7741/ws");
//   await client.send("chat.general", "hello");
//   const channel = await client.subscribe("chat.general");
//   const agent = client.createAgent("assistant");
//   await client.close();

// #[wasm_bindgen]
/// MtwClient manages a WebSocket connection to an mtwRequest server from
/// the browser. It handles connection lifecycle, message framing (using the
/// MTW binary protocol), and dispatches incoming messages to registered
/// handlers.
///
/// Unlike the Node.js binding which uses tokio, this binding uses the
/// browser's native WebSocket API via web-sys, with wasm-bindgen-futures
/// bridging Rust futures to JS promises.
pub struct MtwClient {
    /// WebSocket endpoint URL
    url: String,
    /// Authentication token
    auth_token: Option<String>,
    /// Whether the WebSocket is currently open
    connected: bool,
    /// Server-assigned connection ID
    conn_id: Option<String>,
    /// Active subscriptions
    subscriptions: HashMap<String, bool>,
    // In real implementation:
    // ws: Option<WebSocket>,
    // on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
    // on_error: Option<Closure<dyn FnMut(ErrorEvent)>>,
    // on_close: Option<Closure<dyn FnMut(CloseEvent)>>,
    // pending_requests: HashMap<String, oneshot::Sender<MtwMessage>>,
}

// #[wasm_bindgen]
impl MtwClient {
    /// Connect to an mtwRequest server.
    ///
    /// JS signature:
    ///   static async connect(url: string, token?: string): Promise<MtwClient>
    ///
    /// This creates a browser WebSocket connection, performs the MTW handshake
    /// (sending a Connect message and waiting for Ack), and returns a ready
    /// client.
    ///
    /// Example:
    ///   const client = await MtwClient.connect("ws://localhost:7741/ws", "my-token");
    // #[wasm_bindgen(constructor)]
    pub fn connect(url: String, token: Option<String>) -> Self {
        // In real implementation:
        // 1. Create WebSocket via web_sys::WebSocket::new(&url)
        // 2. Set up onopen, onmessage, onerror, onclose handlers using Closure
        // 3. Wait for the WebSocket to open (via a Promise/Future bridge)
        // 4. Send a Connect message with auth credentials
        // 5. Wait for Ack with connection ID
        // 6. Start a setInterval-based ping timer using js_sys::setInterval
        MtwClient {
            url,
            auth_token: token,
            connected: false,
            conn_id: None,
            subscriptions: HashMap::new(),
        }
    }

    /// Close the WebSocket connection.
    ///
    /// JS signature:
    ///   async close(): Promise<void>
    // #[wasm_bindgen]
    pub fn close(&mut self) {
        // In real implementation:
        // 1. Send Disconnect message
        // 2. Call ws.close() on the WebSocket
        // 3. Clean up closures and handlers
        self.connected = false;
    }

    /// Whether the client is currently connected.
    ///
    /// JS signature:
    ///   get connected(): boolean
    // #[wasm_bindgen(getter)]
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// The server-assigned connection ID.
    ///
    /// JS signature:
    ///   get connectionId(): string | undefined
    // #[wasm_bindgen(getter, js_name = "connectionId")]
    pub fn connection_id(&self) -> Option<String> {
        self.conn_id.clone()
    }

    /// Send a text message on a channel.
    ///
    /// JS signature:
    ///   async send(channel: string, payload: string): Promise<void>
    ///
    /// The message is encoded using the MTW binary frame protocol and sent
    /// as a WebSocket binary message.
    // #[wasm_bindgen]
    pub fn send(&self, _channel: String, _payload: String) {
        // 1. Build MtwMessage { msg_type: Publish, channel, payload: Text }
        // 2. Encode with Frame::encode_message
        // 3. Send as binary via ws.send_with_u8_array
    }

    /// Send a binary message on a channel.
    ///
    /// JS signature:
    ///   async sendBinary(channel: string, data: Uint8Array): Promise<void>
    ///
    /// Useful for 3D scene data, audio, or other binary payloads.
    // #[wasm_bindgen(js_name = "sendBinary")]
    pub fn send_binary(&self, _channel: String, _data: Vec<u8>) {
        // 1. Build MtwMessage { msg_type: Publish, channel, payload: Binary(data) }
        // 2. Encode with Frame::encode_binary
        // 3. Send via ws.send_with_u8_array
    }

    /// Send a request and wait for the response.
    ///
    /// JS signature:
    ///   async request(channel: string, payload: string): Promise<MtwMessage>
    ///
    /// Returns a Promise that resolves with the response message.
    // #[wasm_bindgen]
    pub fn request(&self, _channel: String, _payload: String) -> String {
        // Build Request, send, return a Promise that resolves when
        // a Response with matching ref_id arrives
        String::new()
    }

    /// Subscribe to a channel.
    ///
    /// JS signature:
    ///   async subscribe(channel: string): Promise<MtwChannel>
    // #[wasm_bindgen]
    pub fn subscribe(&mut self, channel: String) -> MtwChannel {
        // Send Subscribe message, await Ack
        self.subscriptions.insert(channel.clone(), true);
        MtwChannel {
            name: channel,
            active: true,
        }
    }

    /// Unsubscribe from a channel.
    ///
    /// JS signature:
    ///   async unsubscribe(channel: string): Promise<void>
    // #[wasm_bindgen]
    pub fn unsubscribe(&mut self, channel: String) {
        self.subscriptions.remove(&channel);
    }

    /// Create an agent interaction handle.
    ///
    /// JS signature:
    ///   createAgent(name: string): MtwAgent
    // #[wasm_bindgen(js_name = "createAgent")]
    pub fn create_agent(&self, name: String) -> MtwAgent {
        MtwAgent {
            name,
            streaming: false,
        }
    }

    /// Register a global message handler.
    ///
    /// JS signature:
    ///   onMessage(callback: (msg: any) => void): void
    ///
    /// The callback receives all messages not handled by a channel subscription.
    // #[wasm_bindgen(js_name = "onMessage")]
    pub fn on_message(&self) {
        // Accept a js_sys::Function and store it.
        // The WebSocket onmessage handler deserializes incoming frames
        // and dispatches to either channel handlers or this global handler.
    }
}

// ---------------------------------------------------------------------------
// MtwChannel — channel subscription handle (browser)
// ---------------------------------------------------------------------------

// #[wasm_bindgen]
/// MtwChannel represents a channel subscription in the browser.
/// It provides methods to publish messages and register handlers for
/// incoming messages on the channel.
pub struct MtwChannel {
    name: String,
    active: bool,
}

// #[wasm_bindgen]
impl MtwChannel {
    /// The channel name.
    // #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// Whether the subscription is active.
    // #[wasm_bindgen(getter)]
    pub fn active(&self) -> bool {
        self.active
    }

    /// Register a message handler for this channel.
    ///
    /// JS signature:
    ///   onMessage(callback: (msg: any) => void): void
    // #[wasm_bindgen(js_name = "onMessage")]
    pub fn on_message(&self) {
        // Accept js_sys::Function, store as handler for this channel
    }

    /// Publish a message to this channel.
    ///
    /// JS signature:
    ///   async publish(payload: string): Promise<void>
    // #[wasm_bindgen]
    pub fn publish(&self, _payload: String) {
        // Build Publish message and send through the parent client's WebSocket
    }

    /// Publish binary data to this channel.
    ///
    /// JS signature:
    ///   async publishBinary(data: Uint8Array): Promise<void>
    // #[wasm_bindgen(js_name = "publishBinary")]
    pub fn publish_binary(&self, _data: Vec<u8>) {
        // Encode as binary frame and send
    }

    /// Unsubscribe from this channel.
    ///
    /// JS signature:
    ///   async unsubscribe(): Promise<void>
    // #[wasm_bindgen]
    pub fn unsubscribe(&mut self) {
        self.active = false;
    }
}

// ---------------------------------------------------------------------------
// MtwAgent — AI agent handle (browser)
// ---------------------------------------------------------------------------

// #[wasm_bindgen]
/// MtwAgent provides browser-side AI agent interaction.
///
/// Since WASM runs in the browser, streaming is exposed via a callback
/// pattern (rather than async iterators, which require more complex
/// wasm-bindgen glue).
///
/// Example:
///   const agent = client.createAgent("assistant");
///   const response = await agent.send("Hello!");
///
///   // Streaming with callback
///   agent.stream("Tell me a story", (chunk) => {
///     document.body.textContent += chunk.text;
///   });
pub struct MtwAgent {
    name: String,
    streaming: bool,
}

// #[wasm_bindgen]
impl MtwAgent {
    /// The agent name.
    // #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// Whether a streaming response is in progress.
    // #[wasm_bindgen(getter, js_name = "isStreaming")]
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Send a task and get the complete response.
    ///
    /// JS signature:
    ///   async send(content: string): Promise<string>
    ///
    /// Returns a Promise that resolves with the complete agent response text.
    // #[wasm_bindgen]
    pub fn send(&self, _content: String) -> String {
        // Build AgentTask, send via WebSocket, collect chunks until AgentComplete
        // Return a Promise via wasm_bindgen_futures::future_to_promise
        String::new()
    }

    /// Stream a response using a callback.
    ///
    /// JS signature:
    ///   stream(content: string, onChunk: (chunk: { text: string, done: boolean }) => void): void
    ///
    /// The callback is invoked for each chunk. When done is true, the stream
    /// is complete.
    // #[wasm_bindgen]
    pub fn stream(&mut self, _content: String) {
        // Accept a js_sys::Function callback.
        // Send AgentTask, then for each AgentChunk, invoke the callback
        // with a JS object { text, done }.
        self.streaming = true;
    }

    /// Register a tool handler.
    ///
    /// JS signature:
    ///   onToolCall(toolName: string, handler: (params: any) => Promise<any>): void
    // #[wasm_bindgen(js_name = "onToolCall")]
    pub fn on_tool_call(&self, _tool_name: String) {
        // Store a JS function handler. When AgentToolCall arrives:
        // 1. Parse params from payload
        // 2. Call the JS handler via wasm_bindgen_futures
        // 3. Send AgentToolResult back
    }
}

// ---------------------------------------------------------------------------
// Utility functions exported to JS
// ---------------------------------------------------------------------------

// #[wasm_bindgen]
/// Initialize the WASM module. Must be called before any other function.
///
/// JS signature:
///   async function init(): Promise<void>
///
/// This sets up the panic hook for better error messages in the browser
/// console and initializes any global state.
pub fn init() {
    // console_error_panic_hook::set_once();
    // console_log::init_with_level(log::Level::Debug).ok();
}

// #[wasm_bindgen(js_name = "protocolVersion")]
/// Get the MTW protocol version supported by this WASM module.
///
/// JS signature:
///   function protocolVersion(): number
pub fn protocol_version() -> u8 {
    // mtw_protocol::frame::PROTOCOL_VERSION
    1
}
