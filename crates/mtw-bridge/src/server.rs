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

use crate::protocol::{read_frame_length, BridgeRequest, BridgeResponse};

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

    /// Start listening for connections.
    ///
    /// Returns a `JoinHandle` for the accept loop. The server runs until
    /// [`shutdown`](Self::shutdown) is called or the handle is aborted.
    pub async fn start(&self) -> Result<tokio::task::JoinHandle<()>, MtwError> {
        // Remove stale socket file if it exists
        let _ = std::fs::remove_file(&self.socket_path);

        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|e| MtwError::Transport(format!("bridge server bind '{}': {}", self.socket_path, e)))?;

        tracing::info!(path = %self.socket_path, tools = self.tools.len(), "bridge server listening");

        let tools = Arc::clone(&self.tools);
        let shutdown = Arc::clone(&self.shutdown);
        let socket_path = self.socket_path.clone();

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
                        tokio::spawn(handle_connection(stream, tools));
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

/// Handle a single persistent connection, reading requests in a loop.
async fn handle_connection(
    mut stream: UnixStream,
    tools: Arc<DashMap<String, BridgeToolHandler>>,
) {
    loop {
        // Read 4-byte length prefix
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    tracing::debug!("bridge server: client disconnected");
                } else {
                    tracing::debug!(error = %e, "bridge server: read error, closing connection");
                }
                return;
            }
        }

        let payload_len = read_frame_length(&len_buf);
        if payload_len > 10 * 1024 * 1024 {
            tracing::error!(len = payload_len, "bridge server: request too large, closing connection");
            let _ = send_error_response(&mut stream, "unknown", "request too large").await;
            return;
        }

        // Read payload
        let mut payload = vec![0u8; payload_len];
        if let Err(e) = stream.read_exact(&mut payload).await {
            tracing::error!(error = %e, "bridge server: failed to read payload");
            return;
        }

        // Decode request
        let request = match rmp_serde::from_slice::<BridgeRequest>(&payload) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!(error = %e, "bridge server: failed to decode request");
                let _ = send_error_response(&mut stream, "unknown", &format!("decode error: {}", e)).await;
                continue;
            }
        };

        let req_id = request.id.clone();
        let tool_name = request.tool.clone();

        // Look up and invoke tool handler
        let response = if let Some(handler) = tools.get(&tool_name) {
            let handler = handler.value().clone();
            // Catch panics from the handler
            match tokio::task::spawn(async move { handler(request.args).await }).await {
                Ok(Ok(result)) => BridgeResponse {
                    id: req_id,
                    result: Some(result),
                    error: None,
                },
                Ok(Err(e)) => BridgeResponse {
                    id: req_id,
                    result: None,
                    error: Some(format!("{}", e)),
                },
                Err(e) => {
                    // JoinError — likely a panic
                    tracing::error!(tool = %tool_name, error = %e, "bridge server: tool handler panicked");
                    BridgeResponse {
                        id: req_id,
                        result: None,
                        error: Some(format!("tool handler panicked: {}", e)),
                    }
                }
            }
        } else {
            BridgeResponse {
                id: req_id,
                result: None,
                error: Some(format!("tool not found: {}", tool_name)),
            }
        };

        // Encode and send response
        if let Err(e) = send_response(&mut stream, &response).await {
            tracing::error!(error = %e, "bridge server: failed to send response");
            return;
        }
    }
}

/// Encode a `BridgeResponse` as a length-prefixed MessagePack frame and write it.
async fn send_response(
    stream: &mut UnixStream,
    response: &BridgeResponse,
) -> Result<(), MtwError> {
    let payload = rmp_serde::to_vec_named(response)
        .map_err(|e| MtwError::Internal(format!("encode response: {}", e)))?;
    let len = (payload.len() as u32).to_be_bytes();

    stream
        .write_all(&len)
        .await
        .map_err(|e| MtwError::Transport(format!("write response len: {}", e)))?;
    stream
        .write_all(&payload)
        .await
        .map_err(|e| MtwError::Transport(format!("write response payload: {}", e)))?;
    stream
        .flush()
        .await
        .map_err(|e| MtwError::Transport(format!("flush response: {}", e)))?;

    Ok(())
}

/// Send a quick error response when we don't have a proper request ID.
async fn send_error_response(
    stream: &mut UnixStream,
    id: &str,
    error: &str,
) -> Result<(), MtwError> {
    let response = BridgeResponse {
        id: id.to_string(),
        result: None,
        error: Some(error.to_string()),
    };
    send_response(stream, &response).await
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
