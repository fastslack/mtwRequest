//! WhatsApp integration wiring for the main server.
//!
//! Boots a [`WhatsAppClient`](mtw_whatsapp::WhatsAppClient) against the
//! `whatsapp-bridge` Go sidecar, then:
//!
//! * Publishes every event the sidecar emits onto pub/sub channels:
//!   - `whatsapp:qr` — QR code strings the user must scan to pair a device
//!   - `whatsapp:status` — connection / auth lifecycle
//!   - `whatsapp:inbound` — incoming WhatsApp messages (text + attachments)
//! * Handles outbound Request messages whose `action` starts with
//!   `whatsapp.` (`whatsapp.send_text`, `.send_media`, `.react`, `.delete`,
//!   `.typing`, `.request_qr`, `.logout`).
//!
//! Both directions share the same `WhatsAppClient` handle (it's cheap to
//! clone), so the server stays single-instance while the sidecar handles
//! the Multi-Device protocol.
//!
//! Startup never waits for the sidecar: [`WhatsAppIntegration::start`]
//! returns a handle right away and a background task dials the bridge with
//! exponential backoff (2 s doubling to 30 s, forever), redialing the same
//! way whenever the connection drops. While disconnected, `whatsapp.*`
//! actions fail fast with `not_connected`.

use std::sync::{Arc, RwLock};

use mtw_core::{MtwError, WhatsAppSection};
use mtw_protocol::{MsgType, MtwMessage, Payload};
use mtw_router::MtwRouter;
use mtw_whatsapp::{
    Command, Event, MediaKind, WhatsAppClient, WhatsAppConfig, WhatsAppError,
};
use std::time::Duration;

/// Channel names the integration uses. Exported so callers can subscribe
/// from Rust code without repeating string literals.
pub mod channels {
    pub const INBOUND: &str = "whatsapp:inbound";
    pub const QR: &str = "whatsapp:qr";
    pub const STATUS: &str = "whatsapp:status";
}

/// First retry delay after a failed dial or a dropped connection.
const BACKOFF_START: Duration = Duration::from_secs(2);
/// Upper bound for the retry delay.
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Live handle to the WhatsApp bridge kept alive for the lifetime of the
/// server process. Holds the current client while connected; the
/// background connection task swaps it in and out.
#[derive(Clone)]
pub struct WhatsAppIntegration {
    client: Arc<RwLock<Option<WhatsAppClient>>>,
}

impl WhatsAppIntegration {
    /// Return a handle immediately and connect to the sidecar in the
    /// background (see module docs). Returns `Ok(None)` if the section is
    /// disabled. `connect_timeout_secs` no longer delays boot.
    pub async fn start(
        cfg: &WhatsAppSection,
        router: Arc<MtwRouter>,
    ) -> Result<Option<Self>, MtwError> {
        if !cfg.enabled {
            tracing::info!("whatsapp: disabled in config, skipping");
            return Ok(None);
        }

        ensure_channels(&router);

        let integration = Self {
            client: Arc::new(RwLock::new(None)),
        };
        let socket_path = std::path::PathBuf::from(&cfg.socket);
        tokio::spawn(connection_loop(socket_path, integration.client.clone(), router));

        tracing::info!(socket = %cfg.socket, "whatsapp: integration started, connecting in background");
        Ok(Some(integration))
    }

    /// True while a live connection to the bridge is established.
    #[cfg(test)]
    pub fn is_connected(&self) -> bool {
        self.current_client().is_some()
    }

    fn current_client(&self) -> Option<WhatsAppClient> {
        let guard = self.client.read().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().filter(|c| !c.is_closed()).cloned()
    }

    /// Dispatch a `whatsapp.*` action that arrived as a Request. Returns
    /// the response message to forward back to the caller.
    pub async fn handle_action(&self, action: &str, msg: &MtwMessage) -> MtwMessage {
        let payload_json = msg.payload.as_json().cloned().unwrap_or(serde_json::Value::Null);

        const ACTIONS: [&str; 7] = [
            "whatsapp.request_qr",
            "whatsapp.logout",
            "whatsapp.send_text",
            "whatsapp.send_media",
            "whatsapp.react",
            "whatsapp.delete",
            "whatsapp.typing",
        ];
        if !ACTIONS.contains(&action) {
            return MtwMessage::error(400, format!("unknown whatsapp action: {action}"))
                .with_ref(&msg.id);
        }
        let Some(client) = self.current_client() else {
            return MtwMessage::error(503, "not_connected: whatsapp bridge is not connected")
                .with_ref(&msg.id);
        };

        let result: Result<Option<String>, WhatsAppError> = match action {
            "whatsapp.request_qr" => client.request_qr().await.map(|_| None),
            "whatsapp.logout" => client.logout().await.map(|_| None),
            "whatsapp.send_text" => dispatch_send_text(&client, &payload_json).await.map(Some),
            "whatsapp.send_media" => dispatch_send_media(&client, &payload_json).await.map(Some),
            "whatsapp.react" => dispatch_react(&client, &payload_json).await.map(|_| None),
            "whatsapp.delete" => dispatch_delete(&client, &payload_json).await.map(|_| None),
            "whatsapp.typing" => dispatch_typing(&client, &payload_json).await.map(|_| None),
            _ => unreachable!("action checked against ACTIONS above"),
        };

        match result {
            Ok(Some(id)) => MtwMessage::response(
                &msg.id,
                Payload::Json(serde_json::json!({ "id": id, "queued": true })),
            ),
            Ok(None) => MtwMessage::response(&msg.id, Payload::Json(serde_json::json!({"ok": true}))),
            Err(err) => MtwMessage::error(500, err.to_string()).with_ref(&msg.id),
        }
    }
}

async fn dispatch_send_text(client: &WhatsAppClient, payload: &serde_json::Value) -> Result<String, WhatsAppError> {
    let to = payload.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
    client.send_text(to, text).await
}

async fn dispatch_send_media(client: &WhatsAppClient, payload: &serde_json::Value) -> Result<String, WhatsAppError> {
    let id = generate_id();
    let to = payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let kind = parse_media_kind(payload.get("kind").and_then(|v| v.as_str()).unwrap_or("document"));
    let mime = payload.get("mime").and_then(|v| v.as_str()).unwrap_or("application/octet-stream").to_string();
    let data_b64 = payload.get("data_b64").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let caption = payload.get("caption").and_then(|v| v.as_str()).map(|s| s.to_string());
    let filename = payload.get("filename").and_then(|v| v.as_str()).map(|s| s.to_string());

    client
        .send(Command::SendMedia {
            id: id.clone(),
            to,
            kind,
            mime,
            caption,
            filename,
            data_b64,
        })
        .await?;
    Ok(id)
}

async fn dispatch_react(client: &WhatsAppClient, payload: &serde_json::Value) -> Result<(), WhatsAppError> {
    let id = generate_id();
    client
        .send(Command::React {
            id,
            to: payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            message_id: payload.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            emoji: payload.get("emoji").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        })
        .await
}

async fn dispatch_delete(client: &WhatsAppClient, payload: &serde_json::Value) -> Result<(), WhatsAppError> {
    let id = generate_id();
    client
        .send(Command::Delete {
            id,
            to: payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            message_id: payload.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            for_everyone: payload.get("for_everyone").and_then(|v| v.as_bool()).unwrap_or(true),
        })
        .await
}

async fn dispatch_typing(client: &WhatsAppClient, payload: &serde_json::Value) -> Result<(), WhatsAppError> {
    client
        .send(Command::Typing {
            to: payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        })
        .await
}

// ── internals ────────────────────────────────────────────────────────

fn parse_media_kind(s: &str) -> MediaKind {
    match s {
        "image" => MediaKind::Image,
        "video" => MediaKind::Video,
        "audio" => MediaKind::Audio,
        "voice" => MediaKind::Voice,
        _ => MediaKind::Document,
    }
}

fn generate_id() -> String {
    format!("wa-{}", ulid::Ulid::new())
}

fn ensure_channels(router: &MtwRouter) {
    // Only create channels that aren't already declared in mtw.toml.
    // `create_channel` replaces on collision (DashMap::insert), so an
    // unconditional call here would clobber the operator's history/auth
    // settings. Default history=1 so late subscribers still see the last
    // QR / status published before they connected.
    for name in [channels::INBOUND, channels::QR, channels::STATUS] {
        if router.channels().get(name).is_none() {
            router.channels().create_channel(name, false, None, 1);
        }
    }
}

/// Dial the bridge forever: on success publish the client and pump its
/// events until the connection drops, then clear it and redial. Failed
/// dials back off from [`BACKOFF_START`] doubling up to [`BACKOFF_MAX`].
async fn connection_loop(
    socket_path: std::path::PathBuf,
    slot: Arc<RwLock<Option<WhatsAppClient>>>,
    router: Arc<MtwRouter>,
) {
    let mut delay = BACKOFF_START;
    let mut attempts: u64 = 0;
    loop {
        let wa_cfg = WhatsAppConfig {
            socket_path: socket_path.clone(),
            // Single attempt per dial; the backoff lives here.
            connect_timeout: Duration::ZERO,
            ..Default::default()
        };
        match WhatsAppClient::connect(wa_cfg).await {
            Ok(client) => {
                // Subscribe before publishing the client so no event is lost.
                let mut events = client.subscribe();
                *slot.write().unwrap_or_else(|e| e.into_inner()) = Some(client.clone());
                tracing::info!(socket = %socket_path.display(), attempts, "whatsapp: bridge connected");

                tokio::select! {
                    _ = pump_events(&mut events, &router) => {}
                    _ = client.closed() => {}
                }
                // The read loop has exited, so nothing new can arrive: publish
                // whatever was still queued (e.g. a final status) before
                // clearing the slot.
                drain_events(&mut events, &router).await;

                *slot.write().unwrap_or_else(|e| e.into_inner()) = None;
                tracing::info!(
                    socket = %socket_path.display(),
                    retry_in = ?BACKOFF_START,
                    "whatsapp: bridge disconnected, reconnecting",
                );
                // Same backoff as the initial dial: wait before redialing so
                // a bridge that accepts and drops cannot cause a tight loop.
                tokio::time::sleep(BACKOFF_START).await;
                delay = (BACKOFF_START * 2).min(BACKOFF_MAX);
                attempts = 0;
            }
            Err(err) => {
                attempts += 1;
                tracing::debug!(
                    socket = %socket_path.display(),
                    attempts,
                    retry_in = ?delay,
                    error = %err,
                    "whatsapp: bridge not reachable, retrying",
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(BACKOFF_MAX);
            }
        }
    }
}

/// Publish every event still buffered in `events` without waiting.
async fn drain_events(events: &mut tokio::sync::broadcast::Receiver<Event>, router: &MtwRouter) {
    use tokio::sync::broadcast::error::TryRecvError;
    loop {
        match events.try_recv() {
            Ok(evt) => publish_event(router, evt).await,
            Err(TryRecvError::Lagged(n)) => {
                tracing::warn!(dropped = n, "whatsapp: event pump lagged while draining");
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
        }
    }
}

async fn pump_events(events: &mut tokio::sync::broadcast::Receiver<Event>, router: &MtwRouter) {
    tracing::info!("whatsapp: event pump started");
    loop {
        match events.recv().await {
            Ok(evt) => {
                tracing::info!(event = ?std::mem::discriminant(&evt), "whatsapp: pump got event");
                publish_event(router, evt).await;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    dropped = n,
                    "whatsapp: event pump lagged — consider raising event_buffer",
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                tracing::warn!("whatsapp: event stream closed, pump exiting");
                break;
            }
        }
    }
}

async fn publish_event(router: &MtwRouter, evt: Event) {
    let (channel, payload) = match evt {
        Event::Ready => (
            channels::STATUS,
            serde_json::json!({ "state": "ready" }),
        ),
        Event::Qr { code } => (
            channels::QR,
            serde_json::json!({ "code": code }),
        ),
        Event::PairingSuccess { jid } => (
            channels::STATUS,
            serde_json::json!({ "state": "paired", "jid": jid }),
        ),
        Event::Connected { jid } => (
            channels::STATUS,
            serde_json::json!({ "state": "connected", "jid": jid }),
        ),
        Event::Disconnected { reason } => (
            channels::STATUS,
            serde_json::json!({ "state": "disconnected", "reason": reason }),
        ),
        Event::Message {
            id, from, chat, is_group, group_name, author, push_name,
            timestamp, text, reply_to, attachments,
        } => (
            channels::INBOUND,
            serde_json::json!({
                "id": id,
                "from": from,
                "chat": chat,
                "is_group": is_group,
                "group_name": group_name,
                "author": author,
                "push_name": push_name,
                "timestamp": timestamp,
                "text": text,
                "reply_to": reply_to,
                "attachments": attachments
                    .into_iter()
                    .map(|a| serde_json::json!({
                        "kind": a.kind,
                        "mime": a.mime,
                        "filename": a.filename,
                        "data_b64": a.data_b64,
                        "caption": a.caption,
                    }))
                    .collect::<Vec<_>>(),
            }),
        ),
        Event::Ack { id, message_id } => (
            channels::STATUS,
            serde_json::json!({ "state": "ack", "id": id, "message_id": message_id }),
        ),
        Event::Error { id, code, message } => (
            channels::STATUS,
            serde_json::json!({ "state": "error", "id": id, "code": code, "message": message }),
        ),
    };

    match router.channels().get(channel) {
        Some(ch) => {
            let msg = MtwMessage::new(MsgType::Event, Payload::Json(payload))
                .with_channel(channel);
            let subs = ch.subscriber_count();
            match ch.publish(msg, None).await {
                Ok(n) => tracing::info!(
                    channel = channel,
                    subscribers = subs,
                    delivered = n,
                    "whatsapp: published event",
                ),
                Err(err) => tracing::warn!(channel = channel, error = %err, "whatsapp: publish failed"),
            }
        }
        None => {
            tracing::warn!(channel = channel, "whatsapp: channel not found, dropping event");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_router::{ChannelManager, MiddlewareChain};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    fn temp_socket() -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("mtw-wa-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("wa.sock");
        (dir, sock)
    }

    fn section(socket: &std::path::Path) -> WhatsAppSection {
        WhatsAppSection {
            enabled: true,
            socket: socket.to_string_lossy().to_string(),
            connect_timeout_secs: 60,
        }
    }

    fn request(action: &str) -> MtwMessage {
        MtwMessage::new(MsgType::Request, Payload::Json(serde_json::json!({})))
            .with_metadata("action", serde_json::json!(action))
    }

    fn is_not_connected(resp: &MtwMessage) -> bool {
        resp.msg_type == MsgType::Error
            && resp
                .payload
                .as_json()
                .and_then(|p| p.get("message"))
                .and_then(|m| m.as_str())
                .is_some_and(|m| m.starts_with("not_connected"))
    }

    async fn wait_connected(wa: &WhatsAppIntegration) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while !wa.is_connected() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("integration should connect to the fake bridge");
    }

    #[tokio::test]
    async fn start_does_not_block_and_connects_in_background() {
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(ChannelManager::new(), MiddlewareChain::new()));

        // No bridge socket yet: start must return right away with a handle.
        let started = std::time::Instant::now();
        let wa = WhatsAppIntegration::start(&section(&sock), router.clone())
            .await
            .unwrap()
            .expect("enabled integration returns a handle immediately");
        assert!(started.elapsed() < Duration::from_secs(1));

        let req = request("whatsapp.request_qr");
        let resp = wa.handle_action("whatsapp.request_qr", &req).await;
        assert!(is_not_connected(&resp), "unexpected response: {:?}", resp);

        // Bridge comes up later: the background task connects.
        let listener = UnixListener::bind(&sock).unwrap();
        let (stream, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .expect("integration should dial the bridge")
            .unwrap();
        wait_connected(&wa).await;

        let resp = wa.handle_action("whatsapp.request_qr", &req).await;
        assert_eq!(resp.msg_type, MsgType::Response);

        let (read_half, mut write_half) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        let line = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(line, r#"{"type":"request_qr"}"#);

        // Events from the bridge are forwarded to the whatsapp:* channels.
        write_half.write_all(b"{\"type\":\"qr\",\"code\":\"2@abc\"}\n").await.unwrap();
        let qr = router.channels().get(channels::QR).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while qr.get_history(None).await.is_empty() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("qr event should reach whatsapp:qr");
        let hist = qr.get_history(None).await;
        assert_eq!(hist[0].payload.as_json().unwrap()["code"], "2@abc");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn status_sent_right_before_hangup_reaches_history() {
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(ChannelManager::new(), MiddlewareChain::new()));
        let listener = UnixListener::bind(&sock).unwrap();
        let wa = WhatsAppIntegration::start(&section(&sock), router.clone())
            .await
            .unwrap()
            .unwrap();
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .unwrap()
            .unwrap();
        wait_connected(&wa).await;

        // A burst of events followed by a final status, then an immediate
        // hang-up: the final status must not be lost when the drop is seen.
        let mut burst = Vec::new();
        for i in 0..100 {
            burst.extend_from_slice(format!("{{\"type\":\"qr\",\"code\":\"c{i}\"}}\n").as_bytes());
        }
        burst.extend_from_slice(b"{\"type\":\"disconnected\",\"reason\":\"final\"}\n");
        stream.write_all(&burst).await.unwrap();
        stream.flush().await.unwrap();
        drop(stream);

        let status = router.channels().get(channels::STATUS).unwrap();
        let got = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let hist = status.get_history(None).await;
                if let Some(last) = hist.last() {
                    if last.payload.as_json().and_then(|p| p.get("reason"))
                        == Some(&serde_json::json!("final"))
                    {
                        return true;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(got.is_ok(), "final status before hang-up was dropped");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn reconnects_after_bridge_drops() {
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(ChannelManager::new(), MiddlewareChain::new()));
        let listener = UnixListener::bind(&sock).unwrap();
        let wa = WhatsAppIntegration::start(&section(&sock), router)
            .await
            .unwrap()
            .unwrap();

        let (first, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .unwrap()
            .unwrap();
        wait_connected(&wa).await;

        // Bridge hangs up: actions fail fast until the next connection.
        drop(first);
        tokio::time::timeout(Duration::from_secs(5), async {
            while wa.is_connected() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("integration should notice the drop");
        let req = request("whatsapp.logout");
        assert!(is_not_connected(&wa.handle_action("whatsapp.logout", &req).await));

        let (second, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .expect("integration should redial the bridge")
            .unwrap();
        wait_connected(&wa).await;
        let resp = wa.handle_action("whatsapp.logout", &req).await;
        assert_eq!(resp.msg_type, MsgType::Response);
        let mut lines = BufReader::new(second).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        assert_eq!(line, r#"{"type":"logout"}"#);

        let _ = std::fs::remove_dir_all(dir);
    }
}
