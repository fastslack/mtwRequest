//! Socket.IO client on top of `rust_socketio` (async).
//!
//! Our bench server (see `bench-suite/servers/socketio/server.js`) exposes
//! three events:
//!   - "join"      (client → server, data = channel name) → `socket.join(ch)`
//!   - "publish"   (client → server, data = {channel, data_b64})
//!                 → `io.to(ch).emit("broadcast", { channel, data })`
//!   - "broadcast" (server → client, data = {channel, data_b64})
//!
//! All bytes travel base64-encoded inside a JSON string to survive the
//! engine.io envelope uniformly.

use crate::{BenchClient, BenchMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use bytes::Bytes;
use futures_util::FutureExt;
use rust_socketio::asynchronous::{Client, ClientBuilder};
use rust_socketio::Payload as SioPayload;
use serde_json::json;
use tokio::sync::mpsc;

pub struct SocketIoClient {
    client: Option<Client>,
    rx: Option<mpsc::UnboundedReceiver<BenchMessage>>,
}

impl SocketIoClient {
    pub fn new() -> Self {
        Self {
            client: None,
            rx: None,
        }
    }
}

impl Default for SocketIoClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BenchClient for SocketIoClient {
    async fn connect(&mut self, url: &str) -> Result<()> {
        let (tx, rx) = mpsc::unbounded_channel::<BenchMessage>();

        let client = ClientBuilder::new(url)
            .on("broadcast", move |payload, _| {
                let tx = tx.clone();
                async move {
                    if let Some(msg) = payload_to_bench(&payload) {
                        let _ = tx.send(msg);
                    }
                }
                .boxed()
            })
            .connect()
            .await
            .context("socketio connect")?;

        // rust_socketio 0.6 finishes `.connect()` before the engine.io
        // transport is fully upgraded; emits issued right after can silently
        // drop. A short yield is enough for the upgrade to complete.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;

        self.client = Some(client);
        self.rx = Some(rx);
        Ok(())
    }

    async fn subscribe(&mut self, channel: &str) -> Result<()> {
        let client = self.client.as_ref().context("not connected")?;
        // Wrap in an object — rust_socketio 0.6 serializes a bare JSON string
        // payload in a way the Node server doesn't parse as expected.
        client
            .emit("join", json!({ "channel": channel }))
            .await
            .map_err(|e| anyhow::anyhow!("socketio emit join: {e}"))?;
        Ok(())
    }

    async fn publish(&mut self, channel: &str, payload: Bytes) -> Result<()> {
        let client = self.client.as_ref().context("not connected")?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&payload);
        client
            .emit("publish", json!({ "channel": channel, "data": b64 }))
            .await
            .map_err(|e| anyhow::anyhow!("socketio emit publish: {e}"))?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<BenchMessage> {
        let rx = self.rx.as_mut().context("not connected")?;
        rx.recv().await.context("socketio receiver closed")
    }

    async fn close(&mut self) -> Result<()> {
        if let Some(client) = self.client.take() {
            let _ = client.disconnect().await;
        }
        self.rx.take();
        Ok(())
    }
}

fn payload_to_bench(payload: &SioPayload) -> Option<BenchMessage> {
    match payload {
        SioPayload::Binary(b) => Some(BenchMessage {
            channel: String::new(),
            payload: Bytes::copy_from_slice(b),
        }),
        #[allow(deprecated)]
        SioPayload::String(s) => {
            // Legacy single-string payload.
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(s.as_bytes())
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(s.clone().into_bytes()));
            Some(BenchMessage {
                channel: String::new(),
                payload: bytes,
            })
        }
        SioPayload::Text(values) => {
            // Text(Vec<serde_json::Value>) — our server emits a single object.
            let first = values.first()?;
            let channel = first
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let data = first.get("data").and_then(|v| v.as_str())?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.as_bytes())
                .map(Bytes::from)
                .unwrap_or_else(|_| Bytes::from(data.as_bytes().to_vec()));
            Some(BenchMessage {
                channel,
                payload: bytes,
            })
        }
    }
}
