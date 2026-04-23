//! Centrifugo client over WebSocket using the JSON protocol (v5).
//!
//! Wire format (simplified):
//!   client → server  : `{"id":N,"connect":{"name":"bench"}}`
//!                      `{"id":N,"subscribe":{"channel":"X"}}`
//!                      `{"id":N,"publish":{"channel":"X","data":"<b64>"}}`
//!   server → client  : `{"push":{"channel":"X","pub":{"data":"<b64>"}}}`
//!                      `{"id":N, ...}`  (command replies — ignored in recv)
//!
//! Payloads are base64-encoded inside a JSON string so binary data survives
//! the JSON envelope. This matches what a typical centrifuge-js client would
//! do when shipping bytes.

use crate::{BenchClient, BenchMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct CentrifugoClient {
    ws: Option<Ws>,
    next_id: u64,
    pending: std::collections::VecDeque<BenchMessage>,
}

impl CentrifugoClient {
    pub fn new() -> Self {
        Self {
            ws: None,
            next_id: 1,
            pending: std::collections::VecDeque::new(),
        }
    }

    fn bump_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl Default for CentrifugoClient {
    fn default() -> Self {
        Self::new()
    }
}

// Add at crate root? base64 lives as a transitive from mtw-protocol already.
// We re-import explicitly here.

#[async_trait]
impl BenchClient for CentrifugoClient {
    async fn connect(&mut self, url: &str) -> Result<()> {
        let (mut ws, _) = connect_async(url)
            .await
            .context("centrifugo ws connect")?;

        // Fire the connect command. We deliberately do NOT block waiting for
        // the reply — v5 may batch multiple JSON replies in a single frame
        // and the server queues subsequent commands until the connect is
        // processed, so there's no correctness benefit to waiting here.
        let id = self.next_id;
        self.next_id += 1;
        let cmd = json!({ "id": id, "connect": { "name": "bench" } });
        ws.send(WsMessage::Text(cmd.to_string().into())).await?;

        self.ws = Some(ws);
        Ok(())
    }

    async fn subscribe(&mut self, channel: &str) -> Result<()> {
        let id = self.bump_id();
        let cmd = json!({ "id": id, "subscribe": { "channel": channel } });
        let ws = self.ws.as_mut().context("not connected")?;
        ws.send(WsMessage::Text(cmd.to_string().into())).await?;
        // We don't await the reply here; the bench-runner waits for the
        // first real push before considering the sub live.
        Ok(())
    }

    async fn publish(&mut self, channel: &str, payload: Bytes) -> Result<()> {
        let id = self.bump_id();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&payload);
        let cmd = json!({
            "id": id,
            "publish": { "channel": channel, "data": b64 }
        });
        let ws = self.ws.as_mut().context("not connected")?;
        ws.send(WsMessage::Text(cmd.to_string().into())).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<BenchMessage> {
        // Return any buffered push left over from a batched frame first.
        if let Some(m) = self.pending.pop_front() {
            return Ok(m);
        }
        let ws = self.ws.as_mut().context("not connected")?;
        loop {
            let frame = ws
                .next()
                .await
                .context("centrifugo stream closed")?
                .context("centrifugo frame error")?;
            let WsMessage::Text(s) = frame else {
                continue;
            };
            // Centrifugo v5 batches multiple JSON messages per frame, separated
            // by LF. Parse each line independently and collect pushes.
            for line in s.split('\n') {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let v: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let Some(push) = v.get("push") else { continue };
                let channel = push
                    .get("channel")
                    .and_then(|c| c.as_str())
                    .unwrap_or_default()
                    .to_string();
                let Some(pub_) = push.get("pub") else { continue };
                let data = pub_.get("data").cloned().unwrap_or(serde_json::Value::Null);
                let payload = match data {
                    serde_json::Value::String(s) => base64::engine::general_purpose::STANDARD
                        .decode(s.as_bytes())
                        .map(Bytes::from)
                        .unwrap_or_else(|_| Bytes::from(s.into_bytes())),
                    other => Bytes::from(serde_json::to_vec(&other)?),
                };
                self.pending.push_back(BenchMessage { channel, payload });
            }
            if let Some(m) = self.pending.pop_front() {
                return Ok(m);
            }
            // Frame had no pushes (probably a command reply); keep reading.
        }
    }

    async fn close(&mut self) -> Result<()> {
        if let Some(mut ws) = self.ws.take() {
            let _ = ws.close(None).await;
        }
        Ok(())
    }
}
