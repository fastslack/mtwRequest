// =============================================================================
// mtwRequest — WebSocket Client for Node.js NAPI Binding
// =============================================================================
//
// Low-level WebSocket client that wraps tokio-tungstenite for direct
// WebSocket communication from Node.js. This is used for:
//   - Connecting to an mtwRequest server
//   - Connecting to arbitrary WebSocket endpoints
//   - Raw WebSocket communication (text, binary, ping/pong)
//
// When NAPI-RS is available as a dependency, uncomment the #[napi] attributes.
// The code compiles as a design reference in the meantime.
//
// JS usage:
//   const ws = await MtwWebSocket.connect("ws://localhost:8080/ws");
//   await ws.send("hello");
//   await ws.sendJson({ type: "subscribe", channel: "chat.general" });
//   const msg = await ws.receive();
//   console.log(msg.msgType, msg.text);
//   await ws.close();
//
//   // With options:
//   const ws = await MtwWebSocket.connectWithOptions(
//     "ws://localhost:8080/ws",
//     [{ key: "Authorization", value: "Bearer ..." }],
//     null,
//     ["mtw-v1"]
//   );

// #[macro_use]
// extern crate napi_derive;

// ---------------------------------------------------------------------------
// WebSocket message types (JS-friendly)
// ---------------------------------------------------------------------------

/// Represents a WebSocket message in a JS-friendly format.
// #[napi(object)]
pub struct JsWsMessage {
    /// Message type: "text", "binary", "ping", "pong", "close"
    pub msg_type: String,
    /// Text content (for text messages).
    pub text: Option<String>,
    /// Binary content (for binary/ping/pong messages).
    pub data: Option<Vec<u8>>,
}

/// A key-value header for WebSocket connection options.
// #[napi(object)]
pub struct JsWsHeader {
    pub key: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// MtwWebSocket — low-level WebSocket client
// ---------------------------------------------------------------------------

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message as WsMsg;

type WsSink = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    WsMsg,
>;

type WsStream = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
>;

/// Low-level WebSocket client wrapping tokio-tungstenite.
///
/// Uses a split sink/stream architecture for concurrent send/receive.
/// Thread-safe via Arc<RwLock>.
// #[napi]
pub struct MtwWebSocket {
    sink: Arc<RwLock<WsSink>>,
    stream: Arc<RwLock<WsStream>>,
}

fn tungstenite_to_js(msg: WsMsg) -> Option<JsWsMessage> {
    match msg {
        WsMsg::Text(text) => Some(JsWsMessage {
            msg_type: "text".to_string(),
            text: Some(text.to_string()),
            data: None,
        }),
        WsMsg::Binary(data) => Some(JsWsMessage {
            msg_type: "binary".to_string(),
            text: None,
            data: Some(data.to_vec()),
        }),
        WsMsg::Ping(data) => Some(JsWsMessage {
            msg_type: "ping".to_string(),
            text: None,
            data: Some(data.to_vec()),
        }),
        WsMsg::Pong(data) => Some(JsWsMessage {
            msg_type: "pong".to_string(),
            text: None,
            data: Some(data.to_vec()),
        }),
        WsMsg::Close(_) => Some(JsWsMessage {
            msg_type: "close".to_string(),
            text: None,
            data: None,
        }),
        WsMsg::Frame(_) => None,
    }
}

// #[napi]
impl MtwWebSocket {
    /// Connect to a WebSocket URL with default settings.
    ///
    /// JS signature:
    ///   static async connect(url: string): Promise<MtwWebSocket>
    // #[napi(factory)]
    pub async fn connect(url: String) -> Result<MtwWebSocket, String> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {}", e))?;

        let (sink, stream) = ws_stream.split();

        Ok(MtwWebSocket {
            sink: Arc::new(RwLock::new(sink)),
            stream: Arc::new(RwLock::new(stream)),
        })
    }

    /// Connect with custom headers, bearer token, and/or subprotocols.
    ///
    /// JS signature:
    ///   static async connectWithOptions(
    ///     url: string,
    ///     headers?: JsWsHeader[],
    ///     bearerToken?: string,
    ///     protocols?: string[]
    ///   ): Promise<MtwWebSocket>
    // #[napi(factory)]
    pub async fn connect_with_options(
        url: String,
        headers: Option<Vec<JsWsHeader>>,
        bearer_token: Option<String>,
        _protocols: Option<Vec<String>>,
    ) -> Result<MtwWebSocket, String> {
        use tokio_tungstenite::tungstenite::http::Request;

        let mut request = Request::builder().uri(&url);

        if let Some(headers) = headers {
            for h in headers {
                request = request.header(&h.key, &h.value);
            }
        }

        if let Some(token) = bearer_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let request = request
            .body(())
            .map_err(|e| format!("Failed to build request: {}", e))?;

        let (ws_stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| format!("WebSocket connect failed: {}", e))?;

        let (sink, stream) = ws_stream.split();

        Ok(MtwWebSocket {
            sink: Arc::new(RwLock::new(sink)),
            stream: Arc::new(RwLock::new(stream)),
        })
    }

    /// Send a text message.
    ///
    /// JS signature:
    ///   async send(text: string): Promise<void>
    // #[napi]
    pub async fn send(&self, text: String) -> Result<(), String> {
        let mut sink = self.sink.write().await;
        sink.send(WsMsg::Text(text.into()))
            .await
            .map_err(|e| format!("Send failed: {}", e))
    }

    /// Send a JSON value as text.
    ///
    /// JS signature:
    ///   async sendJson(value: any): Promise<void>
    // #[napi]
    pub async fn send_json(&self, value: serde_json::Value) -> Result<(), String> {
        let text = serde_json::to_string(&value)
            .map_err(|e| format!("JSON serialize failed: {}", e))?;
        self.send(text).await
    }

    /// Send binary data.
    ///
    /// JS signature:
    ///   async sendBinary(data: Buffer): Promise<void>
    // #[napi]
    pub async fn send_binary(&self, data: Vec<u8>) -> Result<(), String> {
        let mut sink = self.sink.write().await;
        sink.send(WsMsg::Binary(data.into()))
            .await
            .map_err(|e| format!("Send binary failed: {}", e))
    }

    /// Send a ping frame.
    ///
    /// JS signature:
    ///   async ping(): Promise<void>
    // #[napi]
    pub async fn ping(&self) -> Result<(), String> {
        let mut sink = self.sink.write().await;
        sink.send(WsMsg::Ping(vec![].into()))
            .await
            .map_err(|e| format!("Ping failed: {}", e))
    }

    /// Receive the next message. Returns None if the connection is closed.
    ///
    /// JS signature:
    ///   async receive(): Promise<JsWsMessage | null>
    // #[napi]
    pub async fn receive(&self) -> Result<Option<JsWsMessage>, String> {
        let mut stream = self.stream.write().await;
        match stream.next().await {
            Some(Ok(msg)) => Ok(tungstenite_to_js(msg)),
            Some(Err(e)) => Err(format!("Receive error: {}", e)),
            None => Ok(None),
        }
    }

    /// Close the WebSocket connection gracefully.
    ///
    /// JS signature:
    ///   async close(): Promise<void>
    // #[napi]
    pub async fn close(&self) -> Result<(), String> {
        let mut sink = self.sink.write().await;
        sink.send(WsMsg::Close(None))
            .await
            .map_err(|e| format!("Close failed: {}", e))
    }
}
