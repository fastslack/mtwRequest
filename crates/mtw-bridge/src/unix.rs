//! Local socket / named pipe bridge implementation
//!
//! Connects to a local socket (Unix domain socket, or a named pipe on
//! Windows) served by an external process.
//! Uses MessagePack framing for minimal overhead (~0.05ms round-trip).
//!
//! The connection is multiplexed: any number of calls can be in flight at
//! once. A writer task owns the write half and sends whole frames, so a
//! caller that gives up (timeout, dropped future) never leaves half a frame
//! on the socket. A reader task owns the read half and routes each response
//! to its caller by `id`; a response that arrives after its caller timed
//! out is dropped instead of being read as the answer to the next call.
//! Server-pushed event frames on the same socket are skipped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mtw_core::MtwError;
use mtw_ipc::IpcStream;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::protocol::{read_frame_length, BridgeRequest, BridgeResponse};
use crate::MtwBridge;

/// Largest response frame accepted from the server.
const MAX_FRAME_BYTES: usize = 10 * 1024 * 1024;

/// Frames queued for the writer task before callers wait for room.
const WRITE_QUEUE: usize = 1024;

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<BridgeResponse>>>>;

/// Local socket / named pipe bridge. Despite the name (kept for API
/// stability), it connects over a Windows named pipe when `path` is a pipe
/// endpoint.
pub struct UnixBridge {
    frames: mpsc::Sender<Vec<u8>>,
    pending: Pending,
    closed: Arc<AtomicBool>,
    timeout: std::time::Duration,
    writer: JoinHandle<()>,
    reader: JoinHandle<()>,
}

/// Removes a call's pending entry when the call ends, however it ends
/// (response, timeout, error or the caller dropping the future).
struct PendingGuard<'a> {
    pending: &'a Pending,
    id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut map) = self.pending.lock() {
            map.remove(&self.id);
        }
    }
}

impl UnixBridge {
    /// Connect to a local socket or named pipe endpoint.
    pub async fn connect(path: &str, timeout_ms: u64) -> Result<Self, MtwError> {
        let stream = mtw_ipc::connect(path)
            .await
            .map_err(|e| MtwError::Transport(format!("bridge connect '{}': {}", path, e)))?;

        tracing::info!(path = %path, "bridge connected");

        Ok(Self::from_stream(
            stream,
            std::time::Duration::from_millis(timeout_ms),
        ))
    }

    fn from_stream(stream: IpcStream, timeout: std::time::Duration) -> Self {
        let (read_half, write_half) = tokio::io::split(stream);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let (frames, frames_rx) = mpsc::channel(WRITE_QUEUE);

        let writer = tokio::spawn(write_loop(write_half, frames_rx, closed.clone()));
        let reader = tokio::spawn(read_loop(read_half, pending.clone(), closed.clone()));

        Self {
            frames,
            pending,
            closed,
            timeout,
            writer,
            reader,
        }
    }

    /// Send a request and wait for the response with the same `id`.
    async fn request(&self, req: BridgeRequest) -> Result<BridgeResponse, MtwError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(MtwError::Transport("bridge connection closed".into()));
        }

        let frame = req
            .encode()
            .map_err(|e| MtwError::Internal(format!("encode: {}", e)))?;

        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| MtwError::Internal("bridge pending map poisoned".into()))?
            .insert(req.id.clone(), tx);
        let _guard = PendingGuard {
            pending: &self.pending,
            id: req.id,
        };

        let exchange = async {
            self.frames
                .send(frame)
                .await
                .map_err(|_| MtwError::Transport("bridge connection closed".into()))?;
            rx.await
                .map_err(|_| MtwError::Transport("bridge connection closed".into()))
        };

        tokio::time::timeout(self.timeout, exchange)
            .await
            .map_err(|_| MtwError::Transport("bridge timeout".into()))?
    }
}

impl Drop for UnixBridge {
    fn drop(&mut self) {
        self.writer.abort();
        self.reader.abort();
    }
}

/// Writes queued frames one at a time, each in full.
async fn write_loop<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut frames: mpsc::Receiver<Vec<u8>>,
    closed: Arc<AtomicBool>,
) {
    while let Some(frame) = frames.recv().await {
        if let Err(e) = writer.write_all(&frame).await {
            tracing::warn!(error = %e, "bridge write failed; closing bridge");
            break;
        }
    }
    closed.store(true, Ordering::Release);
}

/// Reads response frames and hands each one to the caller waiting on its id.
async fn read_loop<R: AsyncRead + Unpin>(mut reader: R, pending: Pending, closed: Arc<AtomicBool>) {
    loop {
        let mut len_buf = [0u8; 4];
        if let Err(e) = reader.read_exact(&mut len_buf).await {
            tracing::warn!(error = %e, "bridge read failed; closing bridge");
            break;
        }

        let payload_len = read_frame_length(&len_buf);
        if payload_len > MAX_FRAME_BYTES {
            tracing::error!(
                bytes = payload_len,
                "bridge response too large; closing bridge"
            );
            break;
        }

        let mut payload = vec![0u8; payload_len];
        if let Err(e) = reader.read_exact(&mut payload).await {
            tracing::warn!(error = %e, "bridge read failed; closing bridge");
            break;
        }

        match BridgeResponse::decode(&payload) {
            Ok(resp) => {
                let waiter = pending.lock().ok().and_then(|mut map| map.remove(&resp.id));
                match waiter {
                    Some(tx) => {
                        let _ = tx.send(resp);
                    }
                    None => tracing::debug!(
                        id = %resp.id,
                        "bridge response with no waiting caller (late or unknown); dropped"
                    ),
                }
            }
            // Event frames (`type: "event"`) share the socket and do not
            // decode as a response (no `id`); nothing here consumes them.
            Err(_) => tracing::trace!(bytes = payload_len, "bridge non-response frame skipped"),
        }
    }

    closed.store(true, Ordering::Release);
    // Dropping the senders wakes every waiting caller with "connection closed".
    if let Ok(mut map) = pending.lock() {
        map.clear();
    }
}

#[async_trait]
impl MtwBridge for UnixBridge {
    async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, MtwError> {
        let resp = self.request(BridgeRequest::new(name, args)).await?;

        if let Some(error) = resp.error {
            return Err(MtwError::Agent(error));
        }

        resp.result
            .ok_or_else(|| MtwError::Internal("bridge: empty result without error".into()))
    }

    async fn health(&self) -> Result<bool, MtwError> {
        let req = BridgeRequest::new("_health", serde_json::json!({}));
        match self.request(req).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::BridgeEventFrame;
    use std::time::Duration;

    fn encode_response(resp: &BridgeResponse) -> Vec<u8> {
        let payload = rmp_serde::to_vec_named(resp).unwrap();
        let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&payload);
        frame
    }

    async fn read_request(stream: &mut IpcStream) -> BridgeRequest {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let mut payload = vec![0u8; read_frame_length(&len_buf)];
        stream.read_exact(&mut payload).await.unwrap();
        rmp_serde::from_slice(&payload).unwrap()
    }

    fn ok(req: &BridgeRequest) -> Vec<u8> {
        encode_response(&BridgeResponse {
            id: req.id.clone(),
            result: Some(serde_json::json!({ "tool": req.tool })),
            error: None,
        })
    }

    /// A bridge connected to a fake server; returns the server-side stream.
    async fn pair(timeout: Duration) -> (tempfile::TempDir, UnixBridge, IpcStream) {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = mtw_ipc::unique_test_endpoint(dir.path(), "bridge");
        let mut listener = mtw_ipc::IpcListener::bind(&endpoint).unwrap();
        let (client, server) = tokio::join!(mtw_ipc::connect(&endpoint), listener.accept());
        let bridge = UnixBridge::from_stream(client.unwrap(), timeout);
        (dir, bridge, server.unwrap())
    }

    #[tokio::test]
    async fn concurrent_calls_get_their_own_responses_out_of_order() {
        let (_dir, bridge, mut server) = pair(Duration::from_secs(5)).await;

        let server_task = tokio::spawn(async move {
            let a = read_request(&mut server).await;
            let b = read_request(&mut server).await;
            // Answer in reverse order, with an event frame in between.
            server.write_all(&ok(&b)).await.unwrap();
            let event = BridgeEventFrame::new("torrent.progress", serde_json::json!({}));
            server.write_all(&event.encode().unwrap()).await.unwrap();
            server.write_all(&ok(&a)).await.unwrap();
            server
        });

        let (ra, rb) = tokio::join!(
            bridge.call_tool("tool_a", serde_json::json!({})),
            bridge.call_tool("tool_b", serde_json::json!({})),
        );
        assert_eq!(ra.unwrap(), serde_json::json!({ "tool": "tool_a" }));
        assert_eq!(rb.unwrap(), serde_json::json!({ "tool": "tool_b" }));
        drop(server_task.await.unwrap());
    }

    #[tokio::test]
    async fn late_response_after_timeout_is_not_given_to_the_next_call() {
        let (_dir, bridge, mut server) = pair(Duration::from_millis(100)).await;

        let slow = bridge.call_tool("slow", serde_json::json!({})).await;
        assert!(matches!(slow, Err(MtwError::Transport(ref m)) if m == "bridge timeout"));

        let server_task = tokio::spawn(async move {
            let slow_req = read_request(&mut server).await;
            let next_req = read_request(&mut server).await;
            // The slow call's answer arrives late, before the next one's.
            server.write_all(&ok(&slow_req)).await.unwrap();
            server.write_all(&ok(&next_req)).await.unwrap();
            server
        });

        let next = bridge.call_tool("next", serde_json::json!({})).await;
        assert_eq!(next.unwrap(), serde_json::json!({ "tool": "next" }));
        drop(server_task.await.unwrap());
    }

    #[tokio::test]
    async fn waiting_calls_fail_fast_when_the_server_closes() {
        let (_dir, bridge, mut server) = pair(Duration::from_secs(30)).await;

        let server_task = tokio::spawn(async move {
            let _ = read_request(&mut server).await;
            drop(server);
        });

        let started = std::time::Instant::now();
        let res = bridge.call_tool("never", serde_json::json!({})).await;
        assert!(matches!(res, Err(MtwError::Transport(ref m)) if m == "bridge connection closed"));
        assert!(started.elapsed() < Duration::from_secs(5));
        server_task.await.unwrap();

        // Later calls fail at once too.
        assert!(bridge
            .call_tool("again", serde_json::json!({}))
            .await
            .is_err());
        assert!(!bridge.health().await.unwrap());
    }
}
