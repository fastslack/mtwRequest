//! Bridge server — listens on a Unix socket and handles incoming tool requests.
//!
//! This is the reverse direction of `UnixBridge`: instead of Rust calling out to
//! an external process, the external process (e.g., mtwKernel in TypeScript)
//! calls into Rust-hosted tool handlers.
//!
//! # Example
//!
//! ```rust,no_run
//! use mtw_bridge::server::{BridgeServer, BridgeToolHandler};
//! use std::sync::Arc;
//!
//! # async fn example() {
//! let server = BridgeServer::new("/tmp/mtw-bridge.sock");
//!
//! // Register a simple tool
//! server.register_tool("echo", Arc::new(|args| {
//!     Box::pin(async move { Ok(args) })
//! }));
//!
//! // Register a compute-heavy tool
//! server.register_tool("compute.fibonacci", Arc::new(|args| {
//!     Box::pin(async move {
//!         let n = args["n"].as_u64().unwrap_or(10);
//!         let result = fib(n);
//!         Ok(serde_json::json!({ "result": result }))
//!     })
//! }));
//!
//! let handle = server.start().await.expect("failed to start bridge server");
//! // ... server is now accepting connections
//! server.shutdown();
//! handle.await.ok();
//! # }
//! # fn fib(n: u64) -> u64 { if n <= 1 { n } else { fib(n-1) + fib(n-2) } }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use mtw_core::MtwError;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::events::BridgeEventBus;
use crate::protocol::{read_frame_length, BridgeEventFrame, BridgeRequest, BridgeResponse};

/// Handler function for a bridge tool.
///
/// Receives tool arguments as a JSON `Value` and returns a JSON `Value` result
/// or an `MtwError`.
pub type BridgeToolHandler = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, MtwError>> + Send>> + Send + Sync,
>;

/// Bridge server that listens on a Unix socket and dispatches incoming
/// tool requests to registered handlers.
///
/// Each accepted connection is handled in its own tokio task, supporting
/// persistent (keep-alive) connections with multiple sequential requests.
pub struct BridgeServer {
    socket_path: String,
    tools: Arc<DashMap<String, BridgeToolHandler>>,
    shutdown: Arc<AtomicBool>,
    events: BridgeEventBus,
}

impl BridgeServer {
    /// Create a new bridge server bound to the given Unix socket path.
    ///
    /// The socket file will be created (or replaced) when [`start`](Self::start) is called.
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            tools: Arc::new(DashMap::new()),
            shutdown: Arc::new(AtomicBool::new(false)),
            events: BridgeEventBus::default(),
        }
    }

    /// Register a tool handler under the given name.
    ///
    /// If a tool with the same name already exists, it is replaced.
    pub fn register_tool(&self, name: impl Into<String>, handler: BridgeToolHandler) {
        self.tools.insert(name.into(), handler);
    }

    /// Number of registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Returns the event bus. Tool handlers (or any module holding a
    /// reference to the server) clone this and call `emit(topic, data)`
    /// to push frames to every connected client.
    pub fn event_bus(&self) -> BridgeEventBus {
        self.events.clone()
    }

    /// Start listening for connections.
    ///
    /// Returns a `JoinHandle` for the accept loop. The server runs until
    /// [`shutdown`](Self::shutdown) is called or the handle is aborted.
    pub async fn start(&self) -> Result<tokio::task::JoinHandle<()>, MtwError> {
        // Remove stale socket file if it exists
        let _ = std::fs::remove_file(&self.socket_path);

        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|e| MtwError::Transport(format!("bridge server bind '{}': {}", self.socket_path, e)))?;

        // Make the socket reachable from non-root callers (e.g. the
        // mtwKernel container running as uid 1000). Override via
        // `MTW_BRIDGE_SOCKET_MODE=0600` for hardened deployments where
        // both ends share a uid. Failure here is logged but not fatal —
        // the bridge still works for the user that owns the socket.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::env::var("MTW_BRIDGE_SOCKET_MODE")
                .ok()
                .and_then(|v| u32::from_str_radix(v.trim_start_matches("0o").trim_start_matches('0'), 8).ok())
                .unwrap_or(0o666);
            if let Err(e) = std::fs::set_permissions(
                &self.socket_path,
                std::fs::Permissions::from_mode(mode),
            ) {
                tracing::warn!(
                    error = %e,
                    path = %self.socket_path,
                    "bridge server: chmod socket failed (continuing — non-root clients may fail to connect)"
                );
            } else {
                tracing::debug!(
                    path = %self.socket_path,
                    mode = format!("{:o}", mode),
                    "bridge server: socket mode set"
                );
            }
        }

        tracing::info!(path = %self.socket_path, tools = self.tools.len(), "bridge server listening");

        let tools = Arc::clone(&self.tools);
        let shutdown = Arc::clone(&self.shutdown);
        let socket_path = self.socket_path.clone();
        let events = self.events.clone();

        let handle = tokio::spawn(async move {
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                // Use a short timeout so we can check the shutdown flag periodically
                let accept_result = tokio::select! {
                    result = listener.accept() => Some(result),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        continue;
                    }
                };

                match accept_result {
                    Some(Ok((stream, _addr))) => {
                        tracing::debug!("bridge server: new connection");
                        let tools = Arc::clone(&tools);
                        let events = events.clone();
                        tokio::spawn(handle_connection(stream, tools, events));
                    }
                    Some(Err(e)) => {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        tracing::error!(error = %e, "bridge server accept error");
                    }
                    None => continue,
                }
            }

            // Clean up socket file
            let _ = std::fs::remove_file(&socket_path);
            tracing::info!("bridge server stopped");
        });

        Ok(handle)
    }

    /// Signal the server to stop accepting new connections.
    ///
    /// In-flight requests on existing connections will finish, but no new
    /// connections will be accepted.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        tracing::info!("bridge server shutdown signaled");
    }

    /// Returns the socket path this server is bound to.
    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }
}

/// Outbound frame multiplexed onto a single connection's writer task —
/// either a response to a request or a server-pushed event.
enum OutFrame {
    Response(BridgeResponse),
    Event(BridgeEventFrame),
}

impl OutFrame {
    fn encode(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        let payload = match self {
            OutFrame::Response(r) => rmp_serde::to_vec_named(r)?,
            OutFrame::Event(e) => rmp_serde::to_vec_named(e)?,
        };
        let len = (payload.len() as u32).to_be_bytes();
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&len);
        frame.extend_from_slice(&payload);
        Ok(frame)
    }
}

/// Handle a single persistent connection.
///
/// The connection is split in half:
/// - the **reader** task reads requests, dispatches handlers, and pushes
///   responses through an internal mpsc;
/// - the **writer** task owns the write half and serializes both
///   responses and broadcast events into the socket;
/// - the **event pump** subscribes to the shared `BridgeEventBus` and
///   forwards every event into the writer's mpsc.
///
/// Splitting writes through a single mpsc avoids interleaving response
/// bytes with event bytes mid-frame.
async fn handle_connection(
    stream: UnixStream,
    tools: Arc<DashMap<String, BridgeToolHandler>>,
    events: BridgeEventBus,
) {
    let (mut reader, mut writer) = stream.into_split();

    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<OutFrame>();

    // Writer task — single point of socket writes.
    let writer_task = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            let bytes = match frame.encode() {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = %e, "bridge server: encode out-frame");
                    continue;
                }
            };
            if let Err(e) = writer.write_all(&bytes).await {
                tracing::debug!(error = %e, "bridge server: write failed, closing writer");
                break;
            }
            if let Err(e) = writer.flush().await {
                tracing::debug!(error = %e, "bridge server: flush failed, closing writer");
                break;
            }
        }
    });

    // Event pump — broadcasts events into this connection's writer.
    let pump_tx = out_tx.clone();
    let mut event_rx = events.subscribe();
    let pump_task = tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(frame) => {
                    if pump_tx.send(OutFrame::Event(frame)).is_err() {
                        // Writer is gone; connection is closing.
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        skipped = n,
                        "bridge server: event subscriber lagged, skipping frames"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Reader loop — request → response.
    loop {
        let mut len_buf = [0u8; 4];
        match reader.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    tracing::debug!("bridge server: client disconnected");
                } else {
                    tracing::debug!(error = %e, "bridge server: read error, closing connection");
                }
                break;
            }
        }

        let payload_len = read_frame_length(&len_buf);
        if payload_len > 10 * 1024 * 1024 {
            tracing::error!(len = payload_len, "bridge server: request too large, closing connection");
            let _ = out_tx.send(OutFrame::Response(BridgeResponse {
                id: "unknown".into(),
                result: None,
                error: Some("request too large".into()),
            }));
            break;
        }

        let mut payload = vec![0u8; payload_len];
        if let Err(e) = reader.read_exact(&mut payload).await {
            tracing::error!(error = %e, "bridge server: failed to read payload");
            break;
        }

        let request = match rmp_serde::from_slice::<BridgeRequest>(&payload) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!(error = %e, "bridge server: failed to decode request");
                let _ = out_tx.send(OutFrame::Response(BridgeResponse {
                    id: "unknown".into(),
                    result: None,
                    error: Some(format!("decode error: {}", e)),
                }));
                continue;
            }
        };

        let req_id = request.id.clone();
        let tool_name = request.tool.clone();

        let response = if let Some(handler) = tools.get(&tool_name) {
            let handler = handler.value().clone();
            match handler(request.args).await {
                Ok(result) => BridgeResponse {
                    id: req_id,
                    result: Some(result),
                    error: None,
                },
                Err(e) => BridgeResponse {
                    id: req_id,
                    result: None,
                    error: Some(format!("{}", e)),
                },
            }
        } else {
            BridgeResponse {
                id: req_id,
                result: None,
                error: Some(format!("tool not found: {}", tool_name)),
            }
        };

        if out_tx.send(OutFrame::Response(response)).is_err() {
            tracing::debug!("bridge server: writer closed, ending reader");
            break;
        }
    }

    // Closing the channel triggers writer/pump tear-down.
    drop(out_tx);
    let _ = writer_task.await;
    pump_task.abort();
    let _ = pump_task.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixStream;

    /// Helper: send a BridgeRequest over a stream and read the BridgeResponse
    async fn send_request(
        stream: &mut UnixStream,
        req: &BridgeRequest,
    ) -> BridgeResponse {
        let frame = req.encode().unwrap();
        stream.write_all(&frame).await.unwrap();

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let payload_len = read_frame_length(&len_buf);

        let mut payload = vec![0u8; payload_len];
        stream.read_exact(&mut payload).await.unwrap();

        BridgeResponse::decode(&payload).unwrap()
    }

    #[tokio::test]
    async fn test_register_tool_and_process_request() {
        let socket_path = format!("/tmp/mtw-bridge-test-{}.sock", ulid::Ulid::new());
        let server = BridgeServer::new(&socket_path);

        server.register_tool(
            "echo",
            Arc::new(|args| Box::pin(async move { Ok(args) })),
        );
        assert_eq!(server.tool_count(), 1);

        let handle = server.start().await.unwrap();

        // Give the server a moment to bind
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Connect as a client
        let mut client = UnixStream::connect(&socket_path).await.unwrap();

        let req = BridgeRequest::new("echo", serde_json::json!({"hello": "world"}));
        let resp = send_request(&mut client, &req).await;

        assert_eq!(resp.id, req.id);
        assert!(!resp.is_error());
        assert_eq!(resp.result.unwrap()["hello"], "world");

        server.shutdown();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_unknown_tool_returns_error() {
        let socket_path = format!("/tmp/mtw-bridge-test-{}.sock", ulid::Ulid::new());
        let server = BridgeServer::new(&socket_path);

        let handle = server.start().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = UnixStream::connect(&socket_path).await.unwrap();

        let req = BridgeRequest::new("nonexistent.tool", serde_json::json!({}));
        let resp = send_request(&mut client, &req).await;

        assert_eq!(resp.id, req.id);
        assert!(resp.is_error());
        assert!(resp.error.unwrap().contains("tool not found"));

        server.shutdown();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_multiple_tools_registered() {
        let socket_path = format!("/tmp/mtw-bridge-test-{}.sock", ulid::Ulid::new());
        let server = BridgeServer::new(&socket_path);

        server.register_tool(
            "add",
            Arc::new(|args| {
                Box::pin(async move {
                    let a = args["a"].as_f64().unwrap_or(0.0);
                    let b = args["b"].as_f64().unwrap_or(0.0);
                    Ok(serde_json::json!({ "sum": a + b }))
                })
            }),
        );

        server.register_tool(
            "multiply",
            Arc::new(|args| {
                Box::pin(async move {
                    let a = args["a"].as_f64().unwrap_or(0.0);
                    let b = args["b"].as_f64().unwrap_or(0.0);
                    Ok(serde_json::json!({ "product": a * b }))
                })
            }),
        );

        server.register_tool(
            "greet",
            Arc::new(|args| {
                Box::pin(async move {
                    let name = args["name"].as_str().unwrap_or("world");
                    Ok(serde_json::json!({ "message": format!("Hello, {}!", name) }))
                })
            }),
        );

        assert_eq!(server.tool_count(), 3);

        let handle = server.start().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = UnixStream::connect(&socket_path).await.unwrap();

        // Test add
        let req = BridgeRequest::new("add", serde_json::json!({"a": 3, "b": 7}));
        let resp = send_request(&mut client, &req).await;
        assert!(!resp.is_error());
        assert_eq!(resp.result.unwrap()["sum"], 10.0);

        // Test multiply
        let req = BridgeRequest::new("multiply", serde_json::json!({"a": 4, "b": 5}));
        let resp = send_request(&mut client, &req).await;
        assert!(!resp.is_error());
        assert_eq!(resp.result.unwrap()["product"], 20.0);

        // Test greet
        let req = BridgeRequest::new("greet", serde_json::json!({"name": "Rust"}));
        let resp = send_request(&mut client, &req).await;
        assert!(!resp.is_error());
        assert_eq!(resp.result.unwrap()["message"], "Hello, Rust!");

        server.shutdown();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_tool_handler_error_returns_error_response() {
        let socket_path = format!("/tmp/mtw-bridge-test-{}.sock", ulid::Ulid::new());
        let server = BridgeServer::new(&socket_path);

        server.register_tool(
            "failing",
            Arc::new(|_args| {
                Box::pin(async move {
                    Err(MtwError::Internal("something went wrong".into()))
                })
            }),
        );

        let handle = server.start().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = UnixStream::connect(&socket_path).await.unwrap();

        let req = BridgeRequest::new("failing", serde_json::json!({}));
        let resp = send_request(&mut client, &req).await;

        assert!(resp.is_error());
        assert!(resp.error.unwrap().contains("something went wrong"));

        server.shutdown();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_event_bus_pushes_to_connected_client() {
        let socket_path = format!("/tmp/mtw-bridge-test-{}.sock", ulid::Ulid::new());
        let server = BridgeServer::new(&socket_path);

        // A handler that emits an event partway through its execution.
        let bus = server.event_bus();
        server.register_tool(
            "trigger",
            Arc::new(move |_args| {
                let bus = bus.clone();
                Box::pin(async move {
                    bus.emit(
                        "torrent.progress",
                        serde_json::json!({"infohash": "abc", "progress": 0.42}),
                    );
                    Ok(serde_json::json!({"ok": true}))
                })
            }),
        );

        let handle = server.start().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = UnixStream::connect(&socket_path).await.unwrap();

        // Send the request that triggers the event.
        let req = BridgeRequest::new("trigger", serde_json::json!({}));
        let frame = req.encode().unwrap();
        client.write_all(&frame).await.unwrap();

        // We expect to read TWO frames now: one event, one response.
        // Order is technically race-y (the writer task pulls from a single
        // mpsc that both response and event push into), but since the
        // handler emits the event before returning Ok, the event arrives
        // first in practice. Decode both and identify by `type` field.
        let mut got_event = false;
        let mut got_response = false;
        for _ in 0..2 {
            let mut len_buf = [0u8; 4];
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.read_exact(&mut len_buf),
            )
            .await
            .expect("timeout reading frame")
            .unwrap();
            let payload_len = read_frame_length(&len_buf);
            let mut payload = vec![0u8; payload_len];
            client.read_exact(&mut payload).await.unwrap();

            let v: serde_json::Value = rmp_serde::from_slice(&payload).unwrap();
            if v.get("type").and_then(|t| t.as_str()) == Some("event") {
                assert_eq!(v["topic"], "torrent.progress");
                assert_eq!(v["data"]["infohash"], "abc");
                got_event = true;
            } else if v.get("id").is_some() {
                assert_eq!(v["result"]["ok"], true);
                got_response = true;
            } else {
                panic!("unexpected frame: {}", v);
            }
        }
        assert!(got_event, "did not receive event frame");
        assert!(got_response, "did not receive response frame");

        server.shutdown();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_persistent_connection_multiple_requests() {
        let socket_path = format!("/tmp/mtw-bridge-test-{}.sock", ulid::Ulid::new());
        let server = BridgeServer::new(&socket_path);

        server.register_tool(
            "counter",
            Arc::new(|args| {
                Box::pin(async move {
                    let n = args["n"].as_u64().unwrap_or(0);
                    Ok(serde_json::json!({ "n_plus_one": n + 1 }))
                })
            }),
        );

        let handle = server.start().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = UnixStream::connect(&socket_path).await.unwrap();

        // Send multiple requests on the same connection
        for i in 0..5u64 {
            let req = BridgeRequest::new("counter", serde_json::json!({"n": i}));
            let resp = send_request(&mut client, &req).await;
            assert!(!resp.is_error());
            assert_eq!(resp.result.unwrap()["n_plus_one"], i + 1);
        }

        server.shutdown();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        let _ = std::fs::remove_file(&socket_path);
    }
}
