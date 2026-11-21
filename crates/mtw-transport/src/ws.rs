use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use mtw_codec::json::JsonCodec;
use mtw_codec::MtwCodec;
use mtw_core::MtwError;
use mtw_protocol::frame::{Frame, FrameType};
use mtw_protocol::{
    ConnId, ConnMetadata, ConnTarget, DisconnectReason, EnvelopeSink, MsgType, MtwMessage, Payload,
    SharedEnvelope, TransportEvent,
};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::Utf8Bytes;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::auth::{extract_token, AuthDecision, MtwHandshakeAuth};
use crate::MtwTransport;

/// Role given to the owner's connections.
pub const ROLE_SUPER_ADMIN: &str = "super_admin";
/// Role given to a user's connections.
pub const ROLE_USER: &str = "user";

/// What the server is told about an admitted connection.
fn auth_info_for(decision: &AuthDecision) -> mtw_protocol::AuthInfo {
    let (user_id, role) = match decision {
        AuthDecision::AllowUser(id) => (Some(id.clone()), ROLE_USER),
        _ => (None, ROLE_SUPER_ADMIN),
    };
    mtw_protocol::AuthInfo {
        user_id,
        token: None,
        roles: vec![role.to_string()],
        claims: Default::default(),
    }
}

/// Largest HTTP upgrade request head accepted before the handshake.
const MAX_HANDSHAKE_HEAD: usize = 16 * 1024;
/// Time a client has to send its upgrade request and get authorized.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// WebSocket subprotocol advertised by MsgPack-capable clients.
const SUBPROTOCOL_MSGPACK: &str = "mtw.msgpack.v1";

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
    /// Connections using the MTW binary frame protocol (vs JSON text).
    /// Mutually exclusive with `msgpack_connections`.
    binary_connections: Arc<DashMap<ConnId, ()>>,
    /// Connections that negotiated the `mtw.msgpack.v1` subprotocol.
    /// Sent/received as WebSocket binary frames whose payload is a
    /// MsgPack-encoded `MtwMessage`.
    msgpack_connections: Arc<DashMap<ConnId, ()>>,
    /// Event channel
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    event_rx: Option<mpsc::UnboundedReceiver<TransportEvent>>,
    /// Codec for message serialization
    codec: Arc<dyn MtwCodec>,
    /// Shutdown signal
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// Handshake authorizer. `None` accepts every connection (library
    /// default); the server binary always installs one.
    auth: Option<Arc<dyn MtwHandshakeAuth>>,
}

impl WebSocketTransport {
    pub fn new(path: impl Into<String>, ping_interval: u64) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            path: path.into(),
            ping_interval,
            connections: Arc::new(DashMap::new()),
            binary_connections: Arc::new(DashMap::new()),
            msgpack_connections: Arc::new(DashMap::new()),
            event_tx,
            event_rx: Some(event_rx),
            codec: Arc::new(JsonCodec),
            shutdown_tx: None,
            auth: None,
        }
    }

    /// Require every connection to pass `auth` before the WebSocket
    /// upgrade completes. Rejected handshakes get a plain HTTP 401/403/503
    /// and the socket is closed.
    pub fn with_auth(mut self, auth: Arc<dyn MtwHandshakeAuth>) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Run the pre-upgrade auth check. Returns the stream (with the bytes
    /// already read replayed in front of it) when the connection may
    /// proceed, plus the request's `User-Agent`.
    async fn authorize_handshake(
        mut stream: TcpStream,
        addr: SocketAddr,
        auth: &dyn MtwHandshakeAuth,
    ) -> Option<(PrefixedStream<TcpStream>, Option<String>, AuthDecision)> {
        let head =
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_request_head(&mut stream)).await {
                Ok(Ok(head)) => head,
                Ok(Err(e)) => {
                    tracing::debug!(addr = %addr, error = %e, "handshake: bad request head");
                    reject(&mut stream, "400 Bad Request").await;
                    return None;
                }
                Err(_) => {
                    tracing::debug!(addr = %addr, "handshake: timed out reading request");
                    return None;
                }
            };
        let Some((target, headers)) = parse_request_head(&head) else {
            reject(&mut stream, "400 Bad Request").await;
            return None;
        };
        let token = extract_token(&target, &headers);
        let decision =
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, auth.authorize(token.as_deref())).await {
                Ok(d) => d,
                Err(_) => AuthDecision::Unavailable,
            };
        if !decision.is_allowed() {
            tracing::warn!(
                addr = %addr,
                decision = ?decision,
                had_token = token.is_some(),
                "websocket handshake rejected"
            );
            reject(&mut stream, decision.status_line()).await;
            return None;
        }
        let user_agent = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
            .map(|(_, v)| v.clone());
        Some((PrefixedStream::new(head, stream), user_agent, decision))
    }

    /// Handle a single WebSocket connection
    #[allow(clippy::too_many_arguments)]
    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        connections: Arc<DashMap<ConnId, WsSender>>,
        binary_connections: Arc<DashMap<ConnId, ()>>,
        msgpack_connections: Arc<DashMap<ConnId, ()>>,
        event_tx: mpsc::UnboundedSender<TransportEvent>,
        codec: Arc<dyn MtwCodec>,
        ping_interval: u64,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
        auth: Option<Arc<dyn MtwHandshakeAuth>>,
    ) {
        // ── Authorization, before any `101 Switching Protocols` ──
        let (stream, user_agent, auth_info) = match auth {
            Some(auth) => match Self::authorize_handshake(stream, addr, auth.as_ref()).await {
                Some((s, ua, decision)) => (s, ua, Some(auth_info_for(&decision))),
                None => return,
            },
            None => (PrefixedStream::new(Vec::new(), stream), None, None),
        };

        // Shared flag the subprotocol callback toggles on if the client
        // advertised `Sec-WebSocket-Protocol: mtw.msgpack.v1`. After the
        // handshake we read it back to route this conn through the
        // MsgPack code path.
        let msgpack_flag = Arc::new(AtomicBool::new(false));
        let cb_flag = msgpack_flag.clone();

        // The handshake callback's signature is fixed by tungstenite.
        #[allow(clippy::result_large_err)]
        let callback =
            move |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
                // The header is a comma‑separated list of preferences; pick
                // `mtw.msgpack.v1` if offered, fall back to default (JSON text)
                // otherwise. Clients that don't send the header stay on JSON.
                let wants_msgpack = req
                    .headers()
                    .get_all("Sec-WebSocket-Protocol")
                    .iter()
                    .filter_map(|h| h.to_str().ok())
                    .flat_map(|s| s.split(','))
                    .any(|p| p.trim() == SUBPROTOCOL_MSGPACK);
                if wants_msgpack {
                    cb_flag.store(true, Ordering::Relaxed);
                    resp.headers_mut().insert(
                        "Sec-WebSocket-Protocol",
                        SUBPROTOCOL_MSGPACK.parse().unwrap(),
                    );
                }
                Ok(resp)
            };

        let ws_stream = match tokio_tungstenite::accept_hdr_async(stream, callback).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!(addr = %addr, error = %e, "websocket handshake failed");
                return;
            }
        };

        let negotiated_msgpack = msgpack_flag.load(Ordering::Relaxed);

        let conn_id = ulid::Ulid::generate().to_string();
        let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();

        // Create a channel to send messages to this connection
        let (conn_tx, mut conn_rx) = mpsc::unbounded_channel::<WsMessage>();
        connections.insert(conn_id.clone(), conn_tx);
        if negotiated_msgpack {
            msgpack_connections.insert(conn_id.clone(), ());
        }

        // Notify: connected
        let meta = ConnMetadata {
            conn_id: conn_id.clone(),
            remote_addr: Some(addr.to_string()),
            user_agent,
            auth: auth_info,
            connected_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        let _ = event_tx.send(TransportEvent::Connected(conn_id.clone(), meta));

        // Send connect acknowledgment in the negotiated format.
        let ack = MtwMessage::new(
            MsgType::Ack,
            Payload::Json(serde_json::json!({ "conn_id": conn_id })),
        );
        if negotiated_msgpack {
            if let Ok(encoded) = rmp_serde::to_vec_named(&ack) {
                let _ = ws_sink.send(WsMessage::Binary(encoded.into())).await;
            }
        } else if let Ok(encoded) = codec.encode(&ack) {
            let _ = ws_sink
                .send(WsMessage::Text(
                    String::from_utf8(encoded.to_vec())
                        .unwrap_or_default()
                        .into(),
                ))
                .await;
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
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(ping_interval));
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
                            // MsgPack-negotiated connections carry the entire
                            // MtwMessage as a bare MsgPack object in the WS
                            // binary frame (no MTW Frame wrapper).
                            if negotiated_msgpack {
                                match rmp_serde::from_slice::<MtwMessage>(&data) {
                                    Ok(mtw_msg) => {
                                        let _ = event_tx.send(TransportEvent::Message(
                                            conn_id.clone(),
                                            mtw_msg,
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(TransportEvent::Error(
                                            conn_id.clone(),
                                            format!("msgpack decode error: {}", e),
                                        ));
                                    }
                                }
                                continue;
                            }
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
        msgpack_connections.remove(&conn_id);

        let _ = event_tx.send(TransportEvent::Disconnected(
            conn_id.clone(),
            disconnect_reason,
        ));

        tracing::debug!(conn_id = %conn_id, "connection closed");
    }
}

impl WebSocketTransport {
    /// Accept connections from an already-bound listener. `listen` binds
    /// and calls this; tests use it to listen on an ephemeral port.
    pub async fn serve(&mut self, listener: TcpListener) -> Result<(), MtwError> {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx.clone());

        let connections = self.connections.clone();
        let binary_connections = self.binary_connections.clone();
        let msgpack_connections = self.msgpack_connections.clone();
        let event_tx = self.event_tx.clone();
        let codec = self.codec.clone();
        let ping_interval = self.ping_interval;

        let auth = self.auth.clone();
        let local = listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();

        tracing::info!(
            addr = %local,
            path = %self.path,
            auth = auth.is_some(),
            "WebSocket transport listening"
        );

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
                            msgpack_connections.clone(),
                            event_tx.clone(),
                            codec.clone(),
                            ping_interval,
                            shutdown_rx,
                            auth.clone(),
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

        self.serve(listener).await
    }

    async fn send(&self, conn_id: &ConnId, msg: MtwMessage) -> Result<(), MtwError> {
        let ws_msg = if self.msgpack_connections.contains_key(conn_id) {
            let encoded = rmp_serde::to_vec_named(&msg)
                .map_err(|e| MtwError::Transport(format!("msgpack encode error: {}", e)))?;
            WsMessage::Binary(encoded.into())
        } else if self.binary_connections.contains_key(conn_id) {
            let frame = Frame::encode_message(&msg)
                .map_err(|e| MtwError::Transport(format!("frame encode error: {}", e)))?;
            WsMessage::Binary(frame)
        } else {
            let encoded = self.codec.encode(&msg)?;
            WsMessage::Text(
                String::from_utf8(encoded.to_vec())
                    .unwrap_or_default()
                    .into(),
            )
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
        // Hot-path fanout: zero encoding per subscriber — just a `Bytes`
        // refcount bump (binary/msgpack) or a zero-copy Utf8Bytes wrap (text).
        let ws_msg = if self.msgpack_connections.contains_key(conn_id) {
            WsMessage::Binary(envelope.msgpack())
        } else if self.binary_connections.contains_key(conn_id) {
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

/// Read the HTTP upgrade request head (through the blank line). WebSocket
/// clients send nothing else until they get `101`, but any extra bytes
/// read are kept and replayed by [`PrefixedStream`].
async fn read_request_head(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let scan_from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if buf[scan_from..].windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(buf);
        }
        if buf.len() > MAX_HANDSHAKE_HEAD {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head too large",
            ));
        }
    }
}

/// Parse the request target and headers out of a complete request head.
fn parse_request_head(head: &[u8]) -> Option<(String, Vec<(String, String)>)> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(head) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return None,
    }
    let target = req.path?.to_string();
    let headers = req
        .headers
        .iter()
        .map(|h| {
            (
                h.name.to_string(),
                String::from_utf8_lossy(h.value).into_owned(),
            )
        })
        .collect();
    Some((target, headers))
}

/// Answer a refused handshake with a bare HTTP status and close.
async fn reject(stream: &mut TcpStream, status_line: &str) {
    let resp = format!(
        "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n"
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// A stream that first yields bytes already read from `inner` (the
/// request head consumed by the auth check), then reads from `inner`.
pub(crate) struct PrefixedStream<S> {
    prefix: Vec<u8>,
    pos: usize,
    inner: S,
}

impl<S> PrefixedStream<S> {
    fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self {
            prefix,
            pos: 0,
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if this.pos < this.prefix.len() {
            let n = buf.remaining().min(this.prefix.len() - this.pos);
            buf.put_slice(&this.prefix[this.pos..this.pos + n]);
            this.pos += n;
            if this.pos == this.prefix.len() {
                this.prefix = Vec::new();
                this.pos = 0;
            }
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Pre-bound delivery handle resolved at subscribe time. The writer mpsc
/// and the wire-format flag are captured once, so every broadcast avoids
/// any conn-id lookup.
/// Wire format resolved at subscribe time. Captured once so the broadcast
/// hot path picks the right cached bytes without a DashMap lookup.
#[derive(Clone, Copy)]
enum WireFormat {
    /// Plain JSON sent as a WebSocket text frame. Default.
    JsonText,
    /// MTW `Frame`‑wrapped JSON sent as a WebSocket binary frame. Used by
    /// clients that opted into the legacy binary framing.
    MtwBinary,
    /// MsgPack body sent as a WebSocket binary frame. Negotiated via the
    /// `mtw.msgpack.v1` subprotocol.
    MsgPack,
}

struct WsConnTarget {
    sender: mpsc::UnboundedSender<WsMessage>,
    format: WireFormat,
}

impl ConnTarget for WsConnTarget {
    fn deliver(&self, envelope: &Arc<SharedEnvelope>) {
        let ws_msg = match self.format {
            WireFormat::MsgPack => WsMessage::Binary(envelope.msgpack()),
            WireFormat::MtwBinary => WsMessage::Binary(envelope.binary()),
            WireFormat::JsonText => WsMessage::Text(envelope_text_utf8(envelope)),
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
        let ws_msg = if self.msgpack_connections.contains_key(conn_id) {
            WsMessage::Binary(envelope.msgpack())
        } else if self.binary_connections.contains_key(conn_id) {
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
        let format = if self.msgpack_connections.contains_key(conn_id) {
            WireFormat::MsgPack
        } else if self.binary_connections.contains_key(conn_id) {
            WireFormat::MtwBinary
        } else {
            WireFormat::JsonText
        };
        Some(Arc::new(WsConnTarget { sender, format }))
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

    // ── Handshake auth ──────────────────────────────────────

    use crate::auth::KernelBrokerAuth;
    use std::sync::atomic::AtomicUsize;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Error as WsError;

    const ADMIN: &str = "super-admin-token";
    const USER: &str = "kul_user_token";
    /// A user the kernel names (200 `{kind:"user", id:"u1"}`).
    const USER_OK: &str = "kul_named_user";
    /// A 200 with no body: not trusted.
    const USER_BARE: &str = "kul_bare_200";
    /// A 200 naming a user with a blank id: refused.
    const USER_NO_ID: &str = "kul_blank_id";

    /// Minimal stand-in for Kernl's `GET /api/auth/broker`: 204 for the
    /// super-admin bearer, 403 for a `kul_` user token, 401 otherwise.
    /// Returns its base URL and a hit counter.
    async fn fake_kernel() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    break;
                };
                let hits = hits_srv.clone();
                tokio::spawn(async move {
                    let Ok(head) = read_request_head(&mut s).await else {
                        return;
                    };
                    let (target, headers) = parse_request_head(&head).unwrap();
                    assert!(target.starts_with("/api/auth/broker"));
                    hits.fetch_add(1, Ordering::SeqCst);
                    let bearer = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                        .map(|(_, v)| v.trim_start_matches("Bearer ").to_string());
                    // The kernel names a user only to a broker that says it
                    // can hold one to their permissions.
                    let principals = headers.iter().any(|(k, v)| {
                        k.eq_ignore_ascii_case(crate::auth::BROKER_FEATURES_HEADER)
                            && v.split(',').any(|f| f.trim() == "principals")
                    });
                    let (status, body) = match bearer.as_deref() {
                        Some(ADMIN) => ("204 No Content", ""),
                        Some(USER_OK) if principals => ("200 OK", r#"{"kind":"user","id":"u1"}"#),
                        Some(USER_BARE) => ("200 OK", ""),
                        Some(USER_NO_ID) => ("200 OK", r#"{"kind":"user","id":" "}"#),
                        Some(t) if t.starts_with("kul_") => ("403 Forbidden", ""),
                        _ => ("401 Unauthorized", ""),
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = s.write_all(resp.as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        (format!("http://{addr}"), hits)
    }

    /// Start a broker guarded by `KernelBrokerAuth(kernel_url)`; returns
    /// the broker's `ws://` URL. The transport is leaked for the test.
    async fn broker(kernel_url: &str, ttl: Duration) -> String {
        let auth = KernelBrokerAuth::new(kernel_url, ttl).unwrap();
        let mut t = WebSocketTransport::new("/ws", 30).with_auth(Arc::new(auth));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        t.serve(listener).await.unwrap();
        // Keep the transport (and its connection map) alive.
        Box::leak(Box::new(t));
        format!("ws://{addr}/ws")
    }

    /// Like [`broker`], but hands back the transport's event stream.
    async fn broker_with_events(
        kernel_url: &str,
    ) -> (String, mpsc::UnboundedReceiver<TransportEvent>) {
        let auth = KernelBrokerAuth::new(kernel_url, Duration::ZERO).unwrap();
        let mut t = WebSocketTransport::new("/ws", 30).with_auth(Arc::new(auth));
        let events = t.take_event_receiver().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        t.serve(listener).await.unwrap();
        Box::leak(Box::new(t));
        (format!("ws://{addr}/ws"), events)
    }

    async fn connected_meta(events: &mut mpsc::UnboundedReceiver<TransportEvent>) -> ConnMetadata {
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .expect("no transport event")
                .expect("event stream closed");
            if let TransportEvent::Connected(_, meta) = ev {
                return meta;
            }
        }
    }

    fn http_status(err: WsError) -> u16 {
        match err {
            WsError::Http(resp) => resp.status().as_u16(),
            other => panic!("expected HTTP rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handshake_without_token_is_rejected() {
        let (kernel, hits) = fake_kernel().await;
        let url = broker(&kernel, Duration::ZERO).await;
        let err = tokio_tungstenite::connect_async(url).await.unwrap_err();
        assert_eq!(http_status(err), 401);
        // The kernel decides, even for a missing token.
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn handshake_with_user_token_is_rejected() {
        let (kernel, _) = fake_kernel().await;
        let url = broker(&kernel, Duration::ZERO).await;
        let err = tokio_tungstenite::connect_async(format!("{url}?token={USER}"))
            .await
            .unwrap_err();
        assert_eq!(http_status(err), 403);
    }

    #[tokio::test]
    async fn handshake_with_unknown_token_is_rejected() {
        let (kernel, _) = fake_kernel().await;
        let url = broker(&kernel, Duration::ZERO).await;
        let err = tokio_tungstenite::connect_async(format!("{url}?token=nope"))
            .await
            .unwrap_err();
        assert_eq!(http_status(err), 401);
    }

    #[tokio::test]
    async fn handshake_with_valid_query_token_passes() {
        let (kernel, _) = fake_kernel().await;
        let url = broker(&kernel, Duration::ZERO).await;
        let (mut ws, resp) = tokio_tungstenite::connect_async(format!("{url}?token={ADMIN}"))
            .await
            .expect("super admin must connect");
        assert_eq!(resp.status().as_u16(), 101);
        // The connection is live: the server sends its connect ack.
        let first = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(first.to_text().unwrap().contains("conn_id"));
    }

    #[tokio::test]
    async fn handshake_with_valid_bearer_header_passes() {
        let (kernel, _) = fake_kernel().await;
        let url = broker(&kernel, Duration::ZERO).await;
        let mut req = url.into_client_request().unwrap();
        req.headers_mut()
            .insert("Authorization", format!("Bearer {ADMIN}").parse().unwrap());
        let (_ws, resp) = tokio_tungstenite::connect_async(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 101);
    }

    #[tokio::test]
    async fn handshake_rejected_when_kernel_is_down() {
        // Bind then drop to get a port nothing listens on.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap()
        };
        let url = broker(&format!("http://{dead}"), Duration::from_secs(30)).await;
        let err = tokio_tungstenite::connect_async(format!("{url}?token={ADMIN}"))
            .await
            .unwrap_err();
        assert_eq!(http_status(err), 503);
    }

    #[tokio::test]
    async fn valid_token_is_cached() {
        let (kernel, hits) = fake_kernel().await;
        let url = broker(&kernel, Duration::from_secs(30)).await;
        for _ in 0..3 {
            tokio_tungstenite::connect_async(format!("{url}?token={ADMIN}"))
                .await
                .unwrap();
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // Rejections are never cached.
        for _ in 0..2 {
            let _ = tokio_tungstenite::connect_async(format!("{url}?token={USER}")).await;
        }
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    /// An expired cache entry used to deadlock DashMap (a `get` guard held
    /// across `remove` on the same shard): the owner's first reconnect after
    /// the TTL hung, and with it every later handshake on the broker.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expired_cache_entry_is_revalidated_not_deadlocked() {
        let (kernel, hits) = fake_kernel().await;
        let url = broker(&kernel, Duration::from_millis(100)).await;
        let connect = |q: String| {
            let url = url.clone();
            async move {
                tokio::time::timeout(
                    Duration::from_secs(3),
                    tokio_tungstenite::connect_async(format!("{url}{q}")),
                )
                .await
                .expect("broker hung on the handshake")
            }
        };
        connect(format!("?token={ADMIN}")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        connect(format!("?token={ADMIN}")).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        // The broker still answers everyone afterwards.
        let err = connect(String::new()).await.unwrap_err();
        assert_eq!(http_status(err), 401);
        connect(format!("?token={ADMIN}")).await.unwrap();
    }

    #[tokio::test]
    async fn a_user_the_kernel_names_connects_as_that_user() {
        let (kernel, _) = fake_kernel().await;
        let (url, mut events) = broker_with_events(&kernel).await;
        let (_ws, resp) = tokio_tungstenite::connect_async(format!("{url}?token={USER_OK}"))
            .await
            .expect("a named user must connect");
        assert_eq!(resp.status().as_u16(), 101);
        let auth = connected_meta(&mut events).await.auth.expect("auth info");
        assert_eq!(auth.roles, vec![ROLE_USER.to_string()]);
        assert_eq!(auth.user_id.as_deref(), Some("u1"));
    }

    #[tokio::test]
    async fn the_owner_connects_as_super_admin() {
        let (kernel, _) = fake_kernel().await;
        let (url, mut events) = broker_with_events(&kernel).await;
        tokio_tungstenite::connect_async(format!("{url}?token={ADMIN}"))
            .await
            .unwrap();
        let auth = connected_meta(&mut events).await.auth.expect("auth info");
        assert_eq!(auth.roles, vec![ROLE_SUPER_ADMIN.to_string()]);
    }

    #[tokio::test]
    async fn a_200_that_names_no_user_is_rejected() {
        let (kernel, _) = fake_kernel().await;
        let url = broker(&kernel, Duration::ZERO).await;
        let err = tokio_tungstenite::connect_async(format!("{url}?token={USER_BARE}"))
            .await
            .unwrap_err();
        assert_eq!(http_status(err), 503);
        let err = tokio_tungstenite::connect_async(format!("{url}?token={USER_NO_ID}"))
            .await
            .unwrap_err();
        assert_eq!(http_status(err), 403);
    }

    #[tokio::test]
    async fn without_auth_configured_transport_stays_open() {
        let mut t = WebSocketTransport::new("/ws", 30);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        t.serve(listener).await.unwrap();
        let (_ws, resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 101);
        drop(t);
    }
}
