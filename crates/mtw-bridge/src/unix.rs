//! Unix domain socket bridge implementation
//!
//! Connects to a Unix socket served by an external process.
//! Uses MessagePack framing for minimal overhead (~0.05ms round-trip).

use async_trait::async_trait;
use mtw_core::MtwError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::protocol::{read_frame_length, BridgeRequest, BridgeResponse};
use crate::MtwBridge;

/// Unix domain socket bridge
pub struct UnixBridge {
    stream: Mutex<UnixStream>,
    timeout: std::time::Duration,
}

impl UnixBridge {
    /// Connect to a Unix socket
    pub async fn connect(path: &str, timeout_ms: u64) -> Result<Self, MtwError> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| MtwError::Transport(format!("bridge connect '{}': {}", path, e)))?;

        tracing::info!(path = %path, "bridge connected");

        Ok(Self {
            stream: Mutex::new(stream),
            timeout: std::time::Duration::from_millis(timeout_ms),
        })
    }

    /// Send a request and read the response
    async fn request(&self, req: BridgeRequest) -> Result<BridgeResponse, MtwError> {
        let frame = req
            .encode()
            .map_err(|e| MtwError::Internal(format!("encode: {}", e)))?;

        let mut stream = self.stream.lock().await;

        // Send frame
        stream
            .write_all(&frame)
            .await
            .map_err(|e| MtwError::Transport(format!("bridge write: {}", e)))?;

        // Read response length (4 bytes)
        let mut len_buf = [0u8; 4];
        tokio::time::timeout(self.timeout, stream.read_exact(&mut len_buf))
            .await
            .map_err(|_| MtwError::Transport("bridge timeout".into()))?
            .map_err(|e| MtwError::Transport(format!("bridge read len: {}", e)))?;

        let payload_len = read_frame_length(&len_buf);
        if payload_len > 10 * 1024 * 1024 {
            return Err(MtwError::Transport(format!(
                "bridge response too large: {} bytes",
                payload_len
            )));
        }

        // Read response payload
        let mut payload = vec![0u8; payload_len];
        tokio::time::timeout(self.timeout, stream.read_exact(&mut payload))
            .await
            .map_err(|_| MtwError::Transport("bridge timeout reading payload".into()))?
            .map_err(|e| MtwError::Transport(format!("bridge read payload: {}", e)))?;

        BridgeResponse::decode(&payload)
            .map_err(|e| MtwError::Internal(format!("bridge decode: {}", e)))
    }
}

#[async_trait]
impl MtwBridge for UnixBridge {
    async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, MtwError> {
        let req = BridgeRequest::new(name, args);
        let req_id = req.id.clone();

        let resp = self.request(req).await?;

        if resp.id != req_id {
            return Err(MtwError::Internal(format!(
                "bridge response ID mismatch: expected {}, got {}",
                req_id, resp.id
            )));
        }

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
