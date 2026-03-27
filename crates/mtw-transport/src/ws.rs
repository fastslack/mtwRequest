use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use mtw_codec::json::JsonCodec;
use mtw_codec::MtwCodec;
use mtw_core::MtwError;
use mtw_protocol::frame::{Frame, FrameType};
use mtw_protocol::{
    ConnId, ConnMetadata, DisconnectReason, MsgType, MtwMessage, Payload, TransportEvent,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::MtwTransport;

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
            let _ = ws_sink.send(WsMessage::Text(String::from_utf8_lossy(&encoded).into())).await;
        }

        // Spawn task to forward messages from channel to WebSocket sink
        let write_handle = tokio::spawn(async move {
            while let Some(msg) = conn_rx.recv().await {
                if ws_sink.send(msg).await.is_err() {
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
                                    // MTW binary frame containing a JSON message
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
                                    // MTW Ping frame — respond with MTW Pong
                                    if let Some(sender) = connections.get(&conn_id) {
                                        let pong = Frame::encode_pong();
                                        let _ = sender.send(WsMessage::Binary(pong.to_vec().into()));
                                    }
                                }
                                Ok((FrameType::Pong, _)) => {
                                    // MTW Pong — connection is alive
                                }
                                Ok((FrameType::Binary, payload)) => {
                                    // Raw binary data (audio, 3D, etc.)
                                    let _ = event_tx.send(TransportEvent::Binary(
                                        conn_id.clone(),
                                        payload.to_vec(),
                                    ));
                                }
                                Err(_) => {
                                    // Not an MTW frame — treat as raw binary
                                    let _ = event_tx.send(TransportEvent::Binary(
                                        conn_id.clone(),
                                        bytes.to_vec(),
                                    ));
                                }
                            }
                        }
                        Some(Ok(WsMessage::Pong(_))) => {
                            // Connection is alive
                        }
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
            // Client speaks MTW binary protocol — send as binary frame
            let frame = Frame::encode_message(&msg)
                .map_err(|e| MtwError::Transport(format!("frame encode error: {}", e)))?;
            WsMessage::Binary(frame.to_vec().into())
        } else {
            // Client speaks JSON text — send as plain JSON
            let encoded = self.codec.encode(&msg)?;
            WsMessage::Text(String::from_utf8_lossy(&encoded).into())
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

    async fn send_binary(&self, conn_id: &ConnId, data: &[u8]) -> Result<(), MtwError> {
        let ws_msg = WsMessage::Binary(data.to_vec().into());

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
        let text = String::from_utf8_lossy(&encoded).to_string();

        let mut errors = vec![];
        for entry in self.connections.iter() {
            if entry.send(WsMessage::Text(text.clone().into())).is_err() {
                errors.push(entry.key().clone());
            }
        }

        // Clean up dead connections
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

        // Close all connections
        let conn_ids: Vec<ConnId> = self.connections.iter().map(|e| e.key().clone()).collect();
        for conn_id in conn_ids {
            let _ = self.close(&conn_id).await;
        }

        Ok(())
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
