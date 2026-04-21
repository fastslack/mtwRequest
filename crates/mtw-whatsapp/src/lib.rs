//! # mtw-whatsapp
//!
//! Driver crate that links mtwRequest to a real WhatsApp account. It talks
//! to the [`whatsapp-bridge`](../../services/whatsapp-bridge) Go sidecar
//! over a Unix domain socket — this crate is pure Rust and imports no
//! WhatsApp protocol libraries.
//!
//! ## Architecture
//!
//! ```text
//! ┌───────────────────┐  Unix socket   ┌──────────────────┐
//! │  mtw-whatsapp     │ ─────────────▶ │ whatsapp-bridge  │
//! │  (this crate,     │ ◀───────────── │  (Go sidecar,    │
//! │   Rust async)     │  newline-JSON  │   whatsmeow)     │
//! └───────────────────┘                └──────────────────┘
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

pub use protocol::{Command, Event, InboundAttachment, MediaKind};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, Mutex};
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

    #[error("bridge reported an error: {code}: {message}")]
    BridgeError { code: String, message: String },
}

// ── config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    /// Absolute path to the sidecar's Unix socket.
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
            socket_path: PathBuf::from("/var/run/mtw-whatsapp/whatsapp.sock"),
            connect_timeout: Duration::from_secs(30),
            reconnect_backoff: Duration::from_millis(500),
            event_buffer: 256,
        }
    }
}

// ── client ─────────────────────────────────────────────────────────

/// Typed handle to the sidecar. Cheap to clone; commands share the same
/// writer, events share the same broadcast channel.
#[derive(Clone)]
pub struct WhatsAppClient {
    writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    events_tx: broadcast::Sender<Event>,
}

impl WhatsAppClient {
    /// Connect, retrying the socket until `connect_timeout` elapses. A
    /// background task is spawned to pump inbound events onto the
    /// broadcast channel until the socket closes.
    pub async fn connect(cfg: WhatsAppConfig) -> Result<Self, WhatsAppError> {
        let stream = Self::dial_with_backoff(&cfg).await?;
        let (read_half, write_half) = stream.into_split();

        let (events_tx, _) = broadcast::channel(cfg.event_buffer);
        let events_bg = events_tx.clone();

        tokio::spawn(async move {
            info!(target: "mtw_whatsapp", "read loop started");
            let mut lines = BufReader::new(read_half).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) if line.trim().is_empty() => continue,
                    Ok(Some(line)) => {
                        info!(target: "mtw_whatsapp", "← bridge: {line}");
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
                                warn!(
                                    target: "mtw_whatsapp",
                                    "dropping malformed event: {err} ({line})"
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
        });

        Ok(Self {
            writer: Arc::new(Mutex::new(write_half)),
            events_tx,
        })
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

    // ── internal ────────────────────────────────────────────────────

    async fn dial_with_backoff(cfg: &WhatsAppConfig) -> Result<UnixStream, WhatsAppError> {
        let deadline = tokio::time::Instant::now() + cfg.connect_timeout;
        loop {
            match UnixStream::connect(&cfg.socket_path).await {
                Ok(s) => return Ok(s),
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
