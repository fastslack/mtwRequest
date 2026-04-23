//! mtwRequest client using `tokio-tungstenite` and the native `MtwMessage`
//! wire format. Sends Subscribe/Publish, filters server ACKs out of `recv`.

use crate::{BenchClient, BenchMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use mtw_protocol::{MsgType, MtwMessage, Payload};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct MtwClient {
    ws: Option<Ws>,
}

impl MtwClient {
    pub fn new() -> Self {
        Self { ws: None }
    }
}

impl Default for MtwClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BenchClient for MtwClient {
    async fn connect(&mut self, url: &str) -> Result<()> {
        let (ws, _) = connect_async(url).await.context("mtw ws connect")?;
        self.ws = Some(ws);
        Ok(())
    }

    async fn subscribe(&mut self, channel: &str) -> Result<()> {
        let ws = self.ws.as_mut().context("not connected")?;
        let msg = MtwMessage::new(MsgType::Subscribe, Payload::None).with_channel(channel);
        ws.send(WsMessage::Text(serde_json::to_string(&msg)?.into())).await?;
        Ok(())
    }

    async fn publish(&mut self, channel: &str, payload: Bytes) -> Result<()> {
        let ws = self.ws.as_mut().context("not connected")?;
        let msg = MtwMessage::new(MsgType::Publish, Payload::Binary(payload.to_vec()))
            .with_channel(channel);
        ws.send(WsMessage::Text(serde_json::to_string(&msg)?.into())).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<BenchMessage> {
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
                        // Skip Response/Ack/Error/etc.
                        continue;
                    }
                    let payload = match m.payload {
                        Payload::Binary(b) => Bytes::from(b),
                        Payload::Json(v) => Bytes::from(serde_json::to_vec(&v)?),
                        Payload::Text(t) => Bytes::from(t.into_bytes()),
                        Payload::None => Bytes::new(),
                    };
                    return Ok(BenchMessage {
                        channel: m.channel.unwrap_or_default(),
                        payload,
                    });
                }
                WsMessage::Binary(b) => {
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
