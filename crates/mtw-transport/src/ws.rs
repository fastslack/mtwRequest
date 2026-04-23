use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use mtw_codec::json::JsonCodec;
use mtw_codec::MtwCodec;
use mtw_core::MtwError;
use mtw_protocol::frame::{Frame, FrameType};
use mtw_protocol::{
    ConnId, ConnMetadata, ConnTarget, DisconnectReason, EnvelopeSink, MsgType, MtwMessage,
    Payload, SharedEnvelope, TransportEvent,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::frame::Utf8Bytes;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::MtwTransport;

/// Wrap a `SharedEnvelope`'s cached JSON bytes into `Utf8Bytes` without
/// re-validating UTF-8. Safe because `serde_json::to_vec` always emits
/// valid UTF-8 (invariant held by `SharedEnvelope::text_bytes`).
#[inline]
fn envelope_text_utf8(envelope: &SharedEnvelope) -> Utf8Bytes {
    // SAFETY: `SharedEnvelope::text_bytes` returns the output of
    // `serde_json::to_vec`, which always produces valid UTF-8.
    unsafe { Utf8Bytes::from_bytes_unchecked(envelope.text_bytes()) }
}

type WsSender = mpsc::UnboundedSender<WsMessage>;

/// WebSocket transport implementation
pub struct WebSocketTransport {
    /// Path to listen on (e.g., "/ws")
    path: String,
    /// Ping interval in seconds
    ping_interval: u64,
    /// Active connections: conn_id -> sender
    connections: Arc<DashMap<ConnId, WsSender>>,
    /// Connections using binary frame protocol (vs JSON text)
    binary_connections: Arc<DashMap<ConnId, ()>>,
    /// Event channel
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    event_rx: Option<mpsc::UnboundedReceiver<TransportEvent>>,
    /// Codec for message serialization
    codec: Arc<dyn MtwCodec>,
    /// Shutdown signal
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl WebSocketTransport {
    pub fn new(path: impl Into<String>, ping_interval: u64) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            path: path.into(),
            ping_interval,
            connections: Arc::new(DashMap::new()),
            binary_connections: Arc::new(DashMap::new()),
            event_tx,
            event_rx: Some(event_rx),
            codec: Arc::new(JsonCodec),
            shutdown_tx: None,
        }
    }

    /// Handle a single WebSocket connection
    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        connections: Arc<DashMap<ConnId, WsSender>>,
        binary_connections: Arc<DashMap<ConnId, ()>>,
        event_tx: mpsc::UnboundedSender<TransportEvent>,
        codec: Arc<dyn MtwCodec>,
        ping_interval: u64,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!(addr = %addr, error = %e, "websocket handshake failed");
                return;
            }
        };

        let conn_id = ulid::Ulid::new().to_string();
        let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();

        // Create a channel to send messages to this connection
        let (conn_tx, mut conn_rx) = mpsc::unbounded_channel::<WsMessage>();
        connections.insert(conn_id.clone(), conn_tx);

        // Notify: connected
        let meta = ConnMetadata {
            conn_id: conn_id.clone(),
            remote_addr: Some(addr.to_string()),
            user_agent: None,
            auth: None,
            connected_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        let _ = event_tx.send(TransportEvent::Connected(conn_id.clone(), meta));

        // Send connect acknowledgment
        let ack = MtwMessage::new(MsgType::Ack, Payload::Json(serde_json::json!({
            "conn_id": conn_id,
        })));
        if let Ok(encoded) = codec.encode(&ack) {
            let _ = ws_sink.send(WsMessage::Text(String::from_utf8(encoded.to_vec()).unwrap_or_default().into())).await;
        }

        // Spawn task to forward messages from channel to WebSocket sink.
        //
        // Batching: `SinkExt::send` flushes after each item, which for
        // tungstenite translates to ~3 syscalls per WS frame. Under
        // fanout bursts that kills the worker path. Instead, `feed` every
        // item into tungstenite's write buffer, then `flush` once per
        // burst — one syscall covers many frames.
        //
        // Pattern: block on `recv` for the first item, then drain anything
        // else already queued with non-blocking `try_recv` (capped by
        // `MAX_BATCH`), then flush once.
        const MAX_BATCH: usize = 256;
        let write_handle = tokio::spawn(async move {
            while let Some(first) = conn_rx.recv().await {
                if ws_sink.feed(first).await.is_err() {
                    break;
                }
                let mut batched = 1usize;
                while batched < MAX_BATCH {
                    match conn_rx.try_recv() {
                        Ok(msg) => {
                            if ws_sink.feed(msg).await.is_err() {
                                break;
                            }
                            batched += 1;
                        }
                        Err(_) => break,
                    }
                }
                if ws_sink.flush().await.is_err() {
                    break;
                }
            }
            let _ = ws_sink.close().await;
        });

        // Ping timer
        let connections_ping = connections.clone();
        let conn_id_ping = conn_id.clone();
        let ping_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(ping_interval));
            loop {
                interval.tick().await;
                if let Some(sender) = connections_ping.get(&conn_id_ping) {
                    if sender.send(WsMessage::Ping(vec![].into())).is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        // Read loop
        let disconnect_reason = loop {
            tokio::select! {
                msg = ws_stream_rx.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            match codec.decode(text.as_bytes()) {
                                Ok(mtw_msg) => {
                                    let _ = event_tx.send(TransportEvent::Message(
                                        conn_id.clone(),
                                        mtw_msg,
                                    ));
                                }
                                Err(e) => {
                                    let _ = event_tx.send(TransportEvent::Error(
                                        conn_id.clone(),
                                        format!("decode error: {}", e),
                                    ));
                                }
                            }
                        }
                        Some(Ok(WsMessage::Binary(data))) => {
                            let bytes = Bytes::from(data.to_vec());
                            match Frame::decode(bytes.clone()) {
                                Ok((FrameType::Json, payload)) => {
                                    binary_connections.insert(conn_id.clone(), ());
                                    match serde_json::from_slice::<MtwMessage>(&payload) {
                                        Ok(mtw_msg) => {
                                            let _ = event_tx.send(TransportEvent::Message(
                                                conn_id.clone(),
                                                mtw_msg,
                                            ));
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(TransportEvent::Error(
                                                conn_id.clone(),
                                                format!("frame JSON decode error: {}", e),
                                            ));
                                        }
                                    }
                                }
                                Ok((FrameType::Ping, _)) => {
                                    if let Some(sender) = connections.get(&conn_id) {
                                        let pong = Frame::encode_pong();
                                        let _ = sender.send(WsMessage::Binary(pong));
                                    }
                                }
                                Ok((FrameType::Pong, _)) => {}
                                Ok((FrameType::Binary, payload)) => {
                                    let _ = event_tx.send(TransportEvent::Binary(
                                        conn_id.clone(),
                                        payload.to_vec(),
                                    ));
                                }
                                Err(_) => {
                                    let _ = event_tx.send(TransportEvent::Binary(
                                        conn_id.clone(),
                                        bytes.to_vec(),
                                    ));
                                }
                            }
                        }
                        Some(Ok(WsMessage::Pong(_))) => {}
                        Some(Ok(WsMessage::Close(_))) => {
                            break DisconnectReason::Normal;
                        }
                        Some(Err(e)) => {
                            break DisconnectReason::Error(e.to_string());
                        }
                        None => {
                            break DisconnectReason::Normal;
                        }
                        _ => {}
                    }
                }
                _ = shutdown_rx.recv() => {
                    break DisconnectReason::ServerShutdown;
                }
            }
        };

        // Cleanup
        ping_handle.abort();
        write_handle.abort();
        connections.remove(&conn_id);
        binary_connections.remove(&conn_id);

        let _ = event_tx.send(TransportEvent::Disconnected(
            conn_id.clone(),
            disconnect_reason,
        ));

        tracing::debug!(conn_id = %conn_id, "connection closed");
    }
}

#[async_trait]
impl MtwTransport for WebSocketTransport {
    fn name(&self) -> &str {
        "websocket"
    }

    async fn listen(&mut self, addr: SocketAddr) -> Result<(), MtwError> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| MtwError::Transport(format!("failed to bind {}: {}", addr, e)))?;

        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx.clone());

        let connections = self.connections.clone();
        let binary_connections = self.binary_connections.clone();
        let event_tx = self.event_tx.clone();
        let codec = self.codec.clone();
        let ping_interval = self.ping_interval;

        tracing::info!(addr = %addr, path = %self.path, "WebSocket transport listening");

        tokio::spawn(async move {
            loop {
                let shutdown_rx = shutdown_tx.subscribe();
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        tracing::debug!(addr = %addr, "new connection");
                        tokio::spawn(Self::handle_connection(
                            stream,
                            addr,
                            connections.clone(),
                            binary_connections.clone(),
                            event_tx.clone(),
                            codec.clone(),
                            ping_interval,
                            shutdown_rx,
                        ));
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to accept connection");
                    }
                }
            }
        });

        Ok(())
    }

    async fn send(&self, conn_id: &ConnId, msg: MtwMessage) -> Result<(), MtwError> {
        let ws_msg = if self.binary_connections.contains_key(conn_id) {
            let frame = Frame::encode_message(&msg)
                .map_err(|e| MtwError::Transport(format!("frame encode error: {}", e)))?;
            WsMessage::Binary(frame)
        } else {
            let encoded = self.codec.encode(&msg)?;
            WsMessage::Text(String::from_utf8(encoded.to_vec()).unwrap_or_default().into())
        };

        if let Some(sender) = self.connections.get(conn_id) {
            sender
                .send(ws_msg)
                .map_err(|_| MtwError::Transport("failed to send message".into()))?;
            Ok(())
        } else {
            Err(MtwError::ConnectionNotFound(conn_id.clone()))
        }
    }

    async fn send_envelope(
        &self,
        conn_id: &ConnId,
        envelope: std::sync::Arc<mtw_protocol::SharedEnvelope>,
    ) -> Result<(), MtwError> {
        // Hot-path fanout: zero JSON/frame encoding per subscriber — just a
        // `Bytes` refcount bump (binary) or a zero-copy Utf8Bytes wrap (text).
        let ws_msg = if self.binary_connections.contains_key(conn_id) {
            WsMessage::Binary(envelope.binary())
        } else {
            WsMessage::Text(envelope_text_utf8(&envelope))
        };

        if let Some(sender) = self.connections.get(conn_id) {
            sender
                .send(ws_msg)
                .map_err(|_| MtwError::Transport("failed to send envelope".into()))?;
            Ok(())
        } else {
            Err(MtwError::ConnectionNotFound(conn_id.clone()))
        }
    }

    async fn send_binary(&self, conn_id: &ConnId, data: &[u8]) -> Result<(), MtwError> {
        let ws_msg = WsMessage::Binary(Bytes::copy_from_slice(data));

        if let Some(sender) = self.connections.get(conn_id) {
            sender
                .send(ws_msg)
                .map_err(|_| MtwError::Transport("failed to send binary".into()))?;
            Ok(())
        } else {
            Err(MtwError::ConnectionNotFound(conn_id.clone()))
        }
    }

    async fn broadcast(&self, msg: MtwMessage) -> Result<(), MtwError> {
        let encoded = self.codec.encode(&msg)?;
        let text = String::from_utf8(encoded.to_vec()).unwrap_or_default();
        // Build WsMessage once — its inner Utf8Bytes is backed by refcounted
        // bytes::Bytes, so .clone() is a refcount bump, not a data copy.
        let ws_msg = WsMessage::Text(text.into());

        let mut errors = vec![];
        for entry in self.connections.iter() {
            if entry.send(ws_msg.clone()).is_err() {
                errors.push(entry.key().clone());
            }
        }

        for conn_id in errors {
            self.connections.remove(&conn_id);
        }

        Ok(())
    }

    async fn close(&self, conn_id: &ConnId) -> Result<(), MtwError> {
        if let Some((_, sender)) = self.connections.remove(conn_id) {
            let _ = sender.send(WsMessage::Close(None));
            Ok(())
        } else {
            Err(MtwError::ConnectionNotFound(conn_id.clone()))
        }
    }

    fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<TransportEvent>> {
        self.event_rx.take()
    }

    fn connection_count(&self) -> usize {
        self.connections.len()
    }

    fn has_connection(&self, conn_id: &ConnId) -> bool {
        self.connections.contains_key(conn_id)
    }

    async fn shutdown(&self) -> Result<(), MtwError> {
        if let Some(ref tx) = self.shutdown_tx {
            let _ = tx.send(());
        }

        let conn_ids: Vec<ConnId> = self.connections.iter().map(|e| e.key().clone()).collect();
        for conn_id in conn_ids {
            let _ = self.close(&conn_id).await;
        }

        Ok(())
    }
}

/// Pre-bound delivery handle resolved at subscribe time. The writer mpsc
/// and the wire-format flag are captured once, so every broadcast avoids
/// any conn-id lookup.
struct WsConnTarget {
    sender: mpsc::UnboundedSender<WsMessage>,
    is_binary: bool,
}

impl ConnTarget for WsConnTarget {
    fn deliver(&self, envelope: &Arc<SharedEnvelope>) {
        let ws_msg = if self.is_binary {
            WsMessage::Binary(envelope.binary())
        } else {
            WsMessage::Text(envelope_text_utf8(envelope))
        };
        let _ = self.sender.send(ws_msg);
    }
}

/// Direct delivery sink. `deliver` is the slow path (DashMap lookup per
/// call). `resolve` is the fast path: called once at subscribe time, its
/// handle is cached on the subscriber entry so broadcasts never pay for
/// a conn-id lookup after that.
impl EnvelopeSink for WebSocketTransport {
    fn deliver(&self, conn_id: &ConnId, envelope: &Arc<SharedEnvelope>) {
        let ws_msg = if self.binary_connections.contains_key(conn_id) {
            WsMessage::Binary(envelope.binary())
        } else {
            WsMessage::Text(envelope_text_utf8(envelope))
        };
        if let Some(sender) = self.connections.get(conn_id) {
            let _ = sender.send(ws_msg);
        }
    }

    fn resolve(&self, conn_id: &ConnId) -> Option<Arc<dyn ConnTarget>> {
        let sender = self.connections.get(conn_id)?.clone();
        let is_binary = self.binary_connections.contains_key(conn_id);
        Some(Arc::new(WsConnTarget { sender, is_binary }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_transport_creation() {
        let transport = WebSocketTransport::new("/ws", 30);
        assert_eq!(transport.name(), "websocket");
        assert_eq!(transport.connection_count(), 0);
    }
}
