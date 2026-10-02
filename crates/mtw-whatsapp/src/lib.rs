//! # mtw-whatsapp
//!
//! Driver crate that links mtwRequest to a real WhatsApp account. It talks
//! to the [`whatsapp-bridge`](../../services/whatsapp-bridge) Go sidecar
//! over a local socket or named pipe — this crate is pure Rust and imports
//! no WhatsApp protocol libraries.
//!
//! ## Architecture
//!
//! ```text
//! ┌───────────────────┐ local socket / ┌──────────────────┐
//! │  mtw-whatsapp     │  named pipe    │ whatsapp-bridge  │
//! │  (this crate,     │ ─────────────▶ │  (Go sidecar,    │
//! │   Rust async)     │ ◀───────────── │   whatsmeow)     │
//! └───────────────────┘  newline-JSON  └──────────────────┘
//!                                              │
//!                                              ▼ WhatsApp MD
//!                                      (mg.whatsapp.net)
//! ```
//!
//! ## Usage
//!
//! ```no_run
//! use mtw_whatsapp::{WhatsAppClient, WhatsAppConfig, Command, Event};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let cfg = WhatsAppConfig {
//!     socket_path: "/var/run/mtw-whatsapp/whatsapp.sock".into(),
//!     ..Default::default()
//! };
//! let client = WhatsAppClient::connect(cfg).await?;
//!
//! // Fire-and-forget outbound:
//! client.send(Command::SendText {
//!     id: "msg-1".into(),
//!     to: "5491123456789@s.whatsapp.net".into(),
//!     text: "hola".into(),
//! }).await?;
//!
//! // Consume events:
//! let mut events = client.subscribe();
//! while let Ok(evt) = events.recv().await {
//!     match evt {
//!         Event::Qr { code } => println!("scan this: {code}"),
//!         Event::Message { text, from, .. } => println!("{from}: {text}"),
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod protocol;

pub use protocol::{ChatItem, Command, Event, InboundAttachment, MediaKind};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mtw_ipc::IpcStream;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::sync::{broadcast, watch, Mutex};
use tokio::time::sleep;
use tracing::{debug, info, warn};

// ── errors ─────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum WhatsAppError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Json(#[from] serde_json::Error),

    #[error("base64 decode: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("bridge socket {0} unreachable after {1:?}: giving up")]
    ConnectTimeout(PathBuf, Duration),

    #[error("bridge reported an error: {code:?}: {message}")]
    Bridge {
        code: Option<String>,
        message: String,
    },

    /// A correlated request (e.g. `list_chats`) got no matching reply
    /// before its deadline.
    #[error("timed out waiting for the bridge's reply")]
    Timeout,

    /// The connection to the bridge closed while waiting for a correlated
    /// reply.
    #[error("bridge connection closed while waiting for a reply")]
    Closed,
}

// ── config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    /// Absolute path to the sidecar's local socket, or a Windows named
    /// pipe endpoint (`\\.\pipe\...`).
    pub socket_path: PathBuf,
    /// Total time to keep retrying the initial connect.
    pub connect_timeout: Duration,
    /// Delay between retry attempts.
    pub reconnect_backoff: Duration,
    /// Size of the broadcast channel that feeds subscribers.
    pub event_buffer: usize,
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from(default_socket()),
            connect_timeout: Duration::from_secs(30),
            reconnect_backoff: Duration::from_millis(500),
            event_buffer: 256,
        }
    }
}

/// Where the bridge listens when nothing is configured.
pub fn default_socket() -> &'static str {
    #[cfg(windows)]
    {
        r"\\.\pipe\mtw-whatsapp"
    }
    #[cfg(not(windows))]
    {
        "/var/run/mtw-whatsapp/whatsapp.sock"
    }
}

// ── client ─────────────────────────────────────────────────────────

/// Typed handle to the sidecar. Cheap to clone; commands share the same
/// writer, events share the same broadcast channel.
#[derive(Clone)]
pub struct WhatsAppClient {
    writer: Arc<Mutex<WriteHalf<IpcStream>>>,
    events_tx: broadcast::Sender<Event>,
    /// Flips to `true` when the read loop exits (bridge hung up or errored).
    closed_rx: watch::Receiver<bool>,
}

impl WhatsAppClient {
    /// Connect, retrying the socket until `connect_timeout` elapses. A
    /// background task is spawned to pump inbound events onto the
    /// broadcast channel until the socket closes.
    pub async fn connect(cfg: WhatsAppConfig) -> Result<Self, WhatsAppError> {
        let stream = Self::dial_with_backoff(&cfg).await?;
        let (read_half, write_half): (ReadHalf<IpcStream>, WriteHalf<IpcStream>) = tokio::io::split(stream);

        let (events_tx, _) = broadcast::channel(cfg.event_buffer);
        let events_bg = events_tx.clone();
        let (closed_tx, closed_rx) = watch::channel(false);

        tokio::spawn(async move {
            info!(target: "mtw_whatsapp", "read loop started");
            let mut lines = BufReader::new(read_half).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) if line.trim().is_empty() => continue,
                    Ok(Some(line)) => {
                        // Never log the raw line: it can carry a QR string,
                        // a phone pairing code, or message text. Only the
                        // `type` tag is safe to surface, and only at debug.
                        debug!(
                            target: "mtw_whatsapp",
                            "← bridge event: type={}", log_summary(&line)
                        );
                        match serde_json::from_str::<Event>(&line) {
                            Ok(evt) => {
                                let subs = events_bg.receiver_count();
                                match events_bg.send(evt) {
                                    Ok(n) => info!(
                                        target: "mtw_whatsapp",
                                        "broadcast ok (subs={subs}, delivered={n})"
                                    ),
                                    Err(_) => warn!(
                                        target: "mtw_whatsapp",
                                        "broadcast send failed (no subscribers or closed)"
                                    ),
                                }
                            }
                            Err(err) => {
                                // Log the line's length, never its content —
                                // a malformed line can still carry a code.
                                warn!(
                                    target: "mtw_whatsapp",
                                    "dropping malformed event: {err} (len={})", line.len()
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        warn!(target: "mtw_whatsapp", "bridge closed socket");
                        break;
                    }
                    Err(err) => {
                        warn!(target: "mtw_whatsapp", "read error: {err}");
                        break;
                    }
                }
            }
            warn!(target: "mtw_whatsapp", "read loop exited");
            let _ = closed_tx.send(true);
        });

        Ok(Self {
            writer: Arc::new(Mutex::new(write_half)),
            events_tx,
            closed_rx,
        })
    }

    /// True once the connection to the bridge is gone (the read loop
    /// exited). A closed client never reconnects; dial a new one.
    pub fn is_closed(&self) -> bool {
        *self.closed_rx.borrow()
    }

    /// Resolves when the connection to the bridge is gone.
    pub async fn closed(&self) {
        let mut rx = self.closed_rx.clone();
        // An error means the read loop's sender was dropped: also closed.
        let _ = rx.wait_for(|closed| *closed).await;
    }

    /// Subscribe to bridge events. Every active subscriber sees every
    /// event published after the subscription was created; slow
    /// subscribers will lose messages (that's standard `broadcast`
    /// behaviour). Use `resubscribe` to catch up.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events_tx.subscribe()
    }

    /// Send a command to the bridge. The future resolves once the
    /// bytes have been written to the socket — not when the bridge has
    /// acknowledged. Correlate with the incoming `Event::Ack` using the
    /// `id` you passed in the command.
    pub async fn send(&self, cmd: Command) -> Result<(), WhatsAppError> {
        let mut bytes = serde_json::to_vec(&cmd)?;
        bytes.push(b'\n');
        let mut guard = self.writer.lock().await;
        guard.write_all(&bytes).await?;
        Ok(())
    }

    /// Convenience: request a plain-text send. Returns the `id` that
    /// the bridge will echo back in the matching `Event::Ack`.
    pub async fn send_text(
        &self,
        to: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<String, WhatsAppError> {
        let id = ulid_like_id();
        self.send(Command::SendText {
            id: id.clone(),
            to: to.into(),
            text: text.into(),
        })
        .await?;
        Ok(id)
    }

    /// Convenience: re-request a QR code (forces re-pairing).
    pub async fn request_qr(&self) -> Result<(), WhatsAppError> {
        self.send(Command::RequestQr).await
    }

    /// Convenience: log out and wipe the stored session. The next
    /// connection will require a fresh QR scan.
    pub async fn logout(&self) -> Result<(), WhatsAppError> {
        self.send(Command::Logout).await
    }

    /// Start (or restart) the login flow using a scanned QR code.
    pub async fn link_qr(&self) -> Result<(), WhatsAppError> {
        self.send(Command::LinkQr).await
    }

    /// Start (or restart) the login flow using whatsmeow's phone-pairing
    /// code instead of a QR scan.
    pub async fn link_phone(&self, phone: impl Into<String>) -> Result<(), WhatsAppError> {
        self.send(Command::LinkPhone {
            phone: phone.into(),
        })
        .await
    }

    /// Abort the linking attempt in progress (QR or phone) and go back to
    /// `idle`. A no-op if nothing is linking.
    pub async fn link_cancel(&self) -> Result<(), WhatsAppError> {
        self.send(Command::LinkCancel).await
    }

    /// List contacts and joined groups. Waits for the correlated `chats`
    /// reply (matched by a generated `id`), or the matching `error` event,
    /// up to `timeout`.
    pub async fn list_chats(
        &self,
        limit: u32,
        timeout: Duration,
    ) -> Result<Vec<protocol::ChatItem>, WhatsAppError> {
        let id = ulid_like_id();
        // Subscribe BEFORE sending so the reply can't be missed.
        let mut rx = self.subscribe();
        self.send(Command::ListChats {
            id: id.clone(),
            limit,
        })
        .await?;
        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(Event::Chats { id: rid, items }) if rid == id => return Ok(items),
                    Ok(Event::Error {
                        id: Some(rid),
                        code,
                        message,
                    }) if rid == id => return Err(WhatsAppError::Bridge { code, message }),
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Err(WhatsAppError::Closed),
                }
            }
        };
        tokio::time::timeout(timeout, wait)
            .await
            .map_err(|_| WhatsAppError::Timeout)?
    }

    // ── internal ────────────────────────────────────────────────────

    async fn dial_with_backoff(cfg: &WhatsAppConfig) -> Result<IpcStream, WhatsAppError> {
        let deadline = tokio::time::Instant::now() + cfg.connect_timeout;
        loop {
            match mtw_ipc::connect(&cfg.socket_path.to_string_lossy()).await {
                Ok(s) => return Ok(s),
                Err(err) if err.kind() == std::io::ErrorKind::InvalidInput => {
                    return Err(WhatsAppError::Io(err));
                }
                Err(err) if tokio::time::Instant::now() < deadline => {
                    debug!(
                        target: "mtw_whatsapp",
                        "bridge socket not ready ({err}); retrying in {:?}",
                        cfg.reconnect_backoff
                    );
                    sleep(cfg.reconnect_backoff).await;
                }
                Err(_) => {
                    return Err(WhatsAppError::ConnectTimeout(
                        cfg.socket_path.clone(),
                        cfg.connect_timeout,
                    ));
                }
            }
        }
    }
}

// ── helpers ────────────────────────────────────────────────────────

/// Safe-to-log summary of a bridge line: just its `"type"` tag. Never
/// returns the payload — a bridge line can carry a QR string, a phone
/// pairing code, a phone number or message text, none of which may be
/// logged. Falls back to `"unknown"` when the line isn't a JSON object
/// with a string `"type"` field.
fn log_summary(line: &str) -> String {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Small monotonic-ish ID helper. Not strictly ULID to avoid pulling in
/// a dependency for something only used as an echo token.
fn ulid_like_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // suffixed with a short random tail
    let tail: u32 = fastrand::u32(..);
    format!("wa-{millis:x}-{tail:08x}")
}

// Prefer a tiny local PRNG rather than adding another workspace dep.
mod fastrand {
    use std::cell::Cell;
    use std::ops::RangeBounds;
    use std::time::{SystemTime, UNIX_EPOCH};

    thread_local! {
        static STATE: Cell<u64> = Cell::new({
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xdead_beef);
            nanos ^ 0x9E37_79B9_7F4A_7C15
        });
    }

    pub fn u32<R: RangeBounds<u32>>(_range: R) -> u32 {
        STATE.with(|s| {
            // xorshift64
            let mut x = s.get();
            if x == 0 { x = 0xdead_beef; }
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            s.set(x);
            x as u32
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Event, MediaKind};

    #[test]
    fn command_roundtrip_send_text() {
        let cmd = Command::SendText {
            id: "x".into(),
            to: "5491@s.whatsapp.net".into(),
            text: "hi".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"send_text\""));
        assert!(json.contains("\"to\":\"5491@s.whatsapp.net\""));
    }

    #[test]
    fn command_roundtrip_request_qr() {
        let cmd = Command::RequestQr;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "{\"type\":\"request_qr\"}");
    }

    #[test]
    fn event_parse_qr() {
        let json = r#"{"type":"qr","code":"2@abc"}"#;
        let evt: Event = serde_json::from_str(json).unwrap();
        assert!(matches!(evt, Event::Qr { ref code } if code == "2@abc"));
    }

    #[test]
    fn event_parse_message_minimal() {
        let json = r#"{
            "type":"message",
            "id":"1","from":"a","chat":"a","is_group":false,
            "author":"a","timestamp":1,"text":"hola"
        }"#;
        let evt: Event = serde_json::from_str(json).unwrap();
        match evt {
            Event::Message { text, attachments, .. } => {
                assert_eq!(text, "hola");
                assert!(attachments.is_empty());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn event_parse_message_with_attachment() {
        let json = r#"{
            "type":"message","id":"1","from":"a","chat":"a","is_group":false,
            "author":"a","timestamp":1,"text":"look",
            "attachments":[{"kind":"image","mime":"image/png","data_b64":"AA=="}]
        }"#;
        let evt: Event = serde_json::from_str(json).unwrap();
        match evt {
            Event::Message { attachments, .. } => {
                assert_eq!(attachments.len(), 1);
                assert_eq!(attachments[0].kind, MediaKind::Image);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn closed_resolves_when_bridge_drops_socket() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = mtw_ipc::unique_test_endpoint(dir.path(), "wa");
        let mut listener = mtw_ipc::IpcListener::bind(&endpoint).unwrap();
        let cfg = WhatsAppConfig {
            socket_path: std::path::PathBuf::from(&endpoint),
            connect_timeout: Duration::ZERO,
            ..Default::default()
        };
        let client = WhatsAppClient::connect(cfg).await.unwrap();
        let server_side = listener.accept().await.unwrap();
        assert!(!client.is_closed());

        drop(server_side);
        tokio::time::timeout(Duration::from_secs(5), client.closed())
            .await
            .expect("closed() should resolve after the bridge hangs up");
        assert!(client.is_closed());
    }

    #[tokio::test]
    async fn zero_timeout_connect_fails_fast_without_socket() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = mtw_ipc::unique_test_endpoint(dir.path(), "missing");
        let cfg = WhatsAppConfig {
            socket_path: std::path::PathBuf::from(&endpoint),
            connect_timeout: Duration::ZERO,
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let err = WhatsAppClient::connect(cfg).await.err().unwrap();
        assert!(matches!(err, WhatsAppError::ConnectTimeout(..)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// A Windows named-pipe endpoint is the wrong kind on Unix: `connect`
    /// must fail immediately with `InvalidInput`, never retrying until the
    /// (here, generous) `connect_timeout` deadline.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn wrong_kind_endpoint_fails_fast_instead_of_retrying() {
        let cfg = WhatsAppConfig {
            socket_path: std::path::PathBuf::from(r"\\.\pipe\x"),
            connect_timeout: Duration::from_secs(30),
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let err = WhatsAppClient::connect(cfg).await.err().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2), "should fail fast, not retry for 30s");
        match err {
            WhatsAppError::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidInput);
            }
            other => panic!("expected WhatsAppError::Io(InvalidInput), got {other:?}"),
        }
    }

    #[test]
    fn log_summary_omits_pairing_code_payload() {
        let line = r#"{"type":"pairing_code","code":"K3M9QX2P","expires_at":1790000000}"#;
        let summary = log_summary(line);
        assert_eq!(summary, "pairing_code");
        assert!(!summary.contains("K3M9QX2P"));
    }

    #[test]
    fn log_summary_omits_qr_and_message_payload() {
        assert_eq!(log_summary(r#"{"type":"qr","code":"2@abc123secret"}"#), "qr");
        assert_eq!(
            log_summary(r#"{"type":"message","text":"private text","id":"1","from":"a","chat":"a","is_group":false,"author":"a","timestamp":1}"#),
            "message"
        );
        assert_eq!(log_summary("not json at all"), "unknown");
    }

    #[tokio::test]
    async fn list_chats_returns_items_for_matching_id() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = mtw_ipc::unique_test_endpoint(dir.path(), "wa");
        let mut listener = mtw_ipc::IpcListener::bind(&endpoint).unwrap();
        let cfg = WhatsAppConfig {
            socket_path: std::path::PathBuf::from(&endpoint),
            connect_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let client = WhatsAppClient::connect(cfg).await.unwrap();
        let stream = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut lines = BufReader::new(read_half).lines();

        tokio::spawn(async move {
            let line = lines.next_line().await.unwrap().unwrap();
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = req["id"].as_str().unwrap().to_string();

            // Wrong id first: must be ignored by the waiting caller.
            let wrong = serde_json::json!({
                "type": "chats",
                "id": "different-id",
                "items": [],
            });
            write_half
                .write_all(format!("{wrong}\n").as_bytes())
                .await
                .unwrap();

            let matching = serde_json::json!({
                "type": "chats",
                "id": id,
                "items": [
                    {"jid": "g@g.us", "name": "Familia", "is_group": true, "last_ts": 0}
                ],
            });
            write_half
                .write_all(format!("{matching}\n").as_bytes())
                .await
                .unwrap();
        });

        let items = client
            .list_chats(50, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].jid, "g@g.us");
        assert_eq!(items[0].name, "Familia");
        assert!(items[0].is_group);
    }

    #[tokio::test]
    async fn list_chats_surfaces_bridge_error() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = mtw_ipc::unique_test_endpoint(dir.path(), "wa");
        let mut listener = mtw_ipc::IpcListener::bind(&endpoint).unwrap();
        let cfg = WhatsAppConfig {
            socket_path: std::path::PathBuf::from(&endpoint),
            connect_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let client = WhatsAppClient::connect(cfg).await.unwrap();
        let stream = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut lines = BufReader::new(read_half).lines();

        tokio::spawn(async move {
            let line = lines.next_line().await.unwrap().unwrap();
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = req["id"].as_str().unwrap().to_string();
            let err = serde_json::json!({
                "type": "error",
                "id": id,
                "code": "not_connected",
                "message": "bridge is not connected",
            });
            write_half
                .write_all(format!("{err}\n").as_bytes())
                .await
                .unwrap();
        });

        let result = client.list_chats(50, Duration::from_secs(2)).await;
        match result {
            Err(WhatsAppError::Bridge { code, message }) => {
                assert_eq!(code.as_deref(), Some("not_connected"));
                assert_eq!(message, "bridge is not connected");
            }
            other => panic!("expected Bridge error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_chats_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = mtw_ipc::unique_test_endpoint(dir.path(), "wa");
        let mut listener = mtw_ipc::IpcListener::bind(&endpoint).unwrap();
        let cfg = WhatsAppConfig {
            socket_path: std::path::PathBuf::from(&endpoint),
            connect_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let client = WhatsAppClient::connect(cfg).await.unwrap();
        let _stream = listener.accept().await.unwrap();
        // The fake bridge never replies.

        let started = std::time::Instant::now();
        let result = client.list_chats(50, Duration::from_millis(200)).await;
        assert!(matches!(result, Err(WhatsAppError::Timeout)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn event_parse_error() {
        let json = r#"{"type":"error","code":"send_failed","message":"boom"}"#;
        let evt: Event = serde_json::from_str(json).unwrap();
        match evt {
            Event::Error { code, message, id } => {
                assert_eq!(code.as_deref(), Some("send_failed"));
                assert_eq!(message, "boom");
                assert!(id.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }
}
