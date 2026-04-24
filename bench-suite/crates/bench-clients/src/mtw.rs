//! mtwRequest client using `tokio-tungstenite` and the native `MtwMessage`
//! wire format.
//!
//! Two wire formats are supported:
//! - **JSON text** (default) — each `MtwMessage` serialized via `serde_json`,
//!   sent as a WebSocket text frame. Compatible with any client out there.
//! - **MsgPack** (opt-in via `MTW_WIRE=msgpack`) — bodies encoded via
//!   `rmp-serde`, negotiated through the `mtw.msgpack.v1` subprotocol, sent
//!   as WebSocket binary frames. ~50 % smaller on the wire, ~3× faster to
//!   encode/decode.

use crate::{BenchClient, BenchMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use mtw_protocol::{MsgType, MtwMessage, Payload};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{client_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Wire format negotiated at connect time. Set via `MTW_WIRE` env var.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wire {
    Json,
    MsgPack,
}

impl Wire {
    fn from_env() -> Self {
        match std::env::var("MTW_WIRE")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("msgpack") | Some("mp") | Some("rmp") => Wire::MsgPack,
            _ => Wire::Json,
        }
    }
}

pub struct MtwClient {
    ws: Option<Ws>,
    wire: Wire,
}

impl MtwClient {
    pub fn new() -> Self {
        Self {
            ws: None,
            wire: Wire::from_env(),
        }
    }
}

impl Default for MtwClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode a message in the negotiated wire format.
fn encode(wire: Wire, msg: &MtwMessage) -> Result<WsMessage> {
    match wire {
        Wire::Json => Ok(WsMessage::Text(serde_json::to_string(msg)?.into())),
        Wire::MsgPack => {
            let bytes = rmp_serde::to_vec_named(msg).context("msgpack encode")?;
            Ok(WsMessage::Binary(bytes.into()))
        }
    }
}

#[async_trait]
impl BenchClient for MtwClient {
    async fn connect(&mut self, url: &str) -> Result<()> {
        // Build the upgrade request. For MsgPack we advertise the
        // subprotocol; the server echoes it back if it accepts.
        let mut req = url.into_client_request().context("build mtw request")?;
        if self.wire == Wire::MsgPack {
            req.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                "mtw.msgpack.v1".parse().context("header")?,
            );
        }

        // Dial TCP ourselves so `client_async` can handshake with our headers.
        let host = req
            .uri()
            .host()
            .context("url missing host")?
            .to_string();
        let port = req.uri().port_u16().unwrap_or(80);
        let tcp = TcpStream::connect((host.as_str(), port))
            .await
            .context("mtw tcp connect")?;
        let (ws, _) = client_async(req, MaybeTlsStream::Plain(tcp))
            .await
            .context("mtw ws handshake")?;
        self.ws = Some(ws);
        Ok(())
    }

    async fn subscribe(&mut self, channel: &str) -> Result<()> {
        let wire = self.wire;
        let ws = self.ws.as_mut().context("not connected")?;
        let msg = MtwMessage::new(MsgType::Subscribe, Payload::None).with_channel(channel);
        ws.send(encode(wire, &msg)?).await?;
        Ok(())
    }

    async fn publish(&mut self, channel: &str, payload: Bytes) -> Result<()> {
        let wire = self.wire;
        let ws = self.ws.as_mut().context("not connected")?;
        let msg = MtwMessage::new(MsgType::Publish, Payload::Binary(payload.to_vec()))
            .with_channel(channel);
        ws.send(encode(wire, &msg)?).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<BenchMessage> {
        let wire = self.wire;
        let ws = self.ws.as_mut().context("not connected")?;
        loop {
            let frame = ws
                .next()
                .await
                .context("mtw ws stream closed")?
                .context("mtw ws frame error")?;
            match frame {
                WsMessage::Text(s) => {
                    let m: MtwMessage = match serde_json::from_str(&s) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if !matches!(m.msg_type, MsgType::Publish | MsgType::Event) {
                        continue;
                    }
                    return Ok(to_bench_msg(m));
                }
                WsMessage::Binary(b) => {
                    if wire == Wire::MsgPack {
                        let m: MtwMessage = match rmp_serde::from_slice(&b) {
                            Ok(m) => m,
                            Err(_) => continue,
                        };
                        if !matches!(m.msg_type, MsgType::Publish | MsgType::Event) {
                            continue;
                        }
                        return Ok(to_bench_msg(m));
                    }
                    return Ok(BenchMessage {
                        channel: String::new(),
                        payload: Bytes::from(b),
                    });
                }
                WsMessage::Ping(p) => {
                    ws.send(WsMessage::Pong(p)).await?;
                }
                WsMessage::Close(_) => anyhow::bail!("mtw ws closed"),
                _ => {}
            }
        }
    }

    async fn close(&mut self) -> Result<()> {
        if let Some(mut ws) = self.ws.take() {
            let _ = ws.close(None).await;
        }
        Ok(())
    }
}

fn to_bench_msg(m: MtwMessage) -> BenchMessage {
    let payload = match m.payload {
        Payload::Binary(b) => Bytes::from(b),
        Payload::Json(v) => Bytes::from(serde_json::to_vec(&v).unwrap_or_default()),
        Payload::Text(t) => Bytes::from(t.into_bytes()),
        Payload::None => Bytes::new(),
    };
    BenchMessage {
        channel: m.channel.unwrap_or_default(),
        payload,
    }
}
