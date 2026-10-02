//! WhatsApp integration wiring for the main server.
//!
//! Boots a [`WhatsAppClient`](mtw_whatsapp::WhatsAppClient) against the
//! `whatsapp-bridge` Go sidecar, then:
//!
//! * Publishes every event the sidecar emits onto pub/sub channels:
//!   - `whatsapp:qr` — QR code strings the user must scan to pair a device
//!   - `whatsapp:status` — session state only (`idle`|`linking`|`connected`|
//!     `disconnected`|`logged_out`|`unknown`); always carries
//!     `bridge_connected`. Safe to retain (`history = 1`): replaying the
//!     last message to a late subscriber is always a valid session state.
//!   - `whatsapp:pairing` — phone-pairing codes (`{"code","expires_at"}`)
//!   - `whatsapp:inbound` — incoming WhatsApp messages (text + attachments)
//!   - `whatsapp:events` — one-shot events (`ready`, `paired`, `ack`,
//!     `error`) that are not session states. `history = 0`: replaying an
//!     `ack`/`error` to a late subscriber would misrepresent it as current.
//! * Handles outbound Request messages whose `action` starts with
//!   `whatsapp.` (`whatsapp.send_text`, `.send_media`, `.react`, `.delete`,
//!   `.typing`, `.request_qr`, `.logout`, `.link_qr`, `.link_phone`,
//!   `.link_cancel`, `.list_chats`, `.status`).
//! * Caches the last session state, QR and pairing code so `whatsapp.status`
//!   can answer a page reloaded mid-linking (works even while the bridge is
//!   down).
//!
//! Both directions share the same `WhatsAppClient` handle (it's cheap to
//! clone), so the server stays single-instance while the sidecar handles
//! the Multi-Device protocol.
//!
//! Startup never waits for the sidecar: [`WhatsAppIntegration::start`]
//! returns a handle right away and a background task dials the bridge with
//! exponential backoff (2 s doubling to 30 s, forever), redialing the same
//! way whenever the connection drops. While disconnected, `whatsapp.*`
//! actions (except `whatsapp.status`) fail fast with `not_connected`.

use std::sync::{Arc, RwLock};

use mtw_core::{MtwError, WhatsAppSection};
use mtw_protocol::{MsgType, MtwMessage, Payload};
use mtw_router::MtwRouter;
use mtw_whatsapp::{Command, Event, MediaKind, WhatsAppClient, WhatsAppConfig, WhatsAppError};
use std::time::Duration;

/// Channel names the integration uses. Exported so callers can subscribe
/// from Rust code without repeating string literals.
pub mod channels {
    pub const INBOUND: &str = "whatsapp:inbound";
    pub const QR: &str = "whatsapp:qr";
    /// Session states only: `idle`|`linking`|`connected`|`disconnected`|
    /// `logged_out`|`unknown`. Always carries `bridge_connected`.
    pub const STATUS: &str = "whatsapp:status";
    pub const PAIRING: &str = "whatsapp:pairing";
    /// One-shot events that are not session states: `ready`, `paired`
    /// (`PairingSuccess`), `ack`, `error`. `history = 0` — never retained.
    pub const EVENTS: &str = "whatsapp:events";
}

/// Timeout for the correlated `list_chats` round-trip.
const LIST_CHATS_TIMEOUT: Duration = Duration::from_secs(10);
/// Default / bounds for `whatsapp.list_chats`' `limit`.
const LIST_CHATS_DEFAULT_LIMIT: u64 = 50;
const LIST_CHATS_MAX_LIMIT: u64 = 200;

/// Last known session state, as reported by the bridge. Lets
/// `whatsapp.status` answer without waiting for the next event.
#[derive(Debug, Clone)]
struct WaState {
    state: String,
    mode: Option<String>,
    jid: Option<String>,
    /// The user's own LID, present once `state == "connected"`. See
    /// [`mtw_whatsapp::Event::Status`].
    lid: Option<String>,
    reason: Option<String>,
    qr: Option<String>,
    code: Option<String>,
    expires_at: Option<i64>,
}

impl Default for WaState {
    fn default() -> Self {
        Self {
            state: "unknown".to_string(),
            mode: None,
            jid: None,
            lid: None,
            reason: None,
            qr: None,
            code: None,
            expires_at: None,
        }
    }
}

impl WaState {
    /// Drop the linking artifacts (QR / pairing code) once the session
    /// leaves `linking`.
    fn clear_linking(&mut self) {
        self.qr = None;
        self.code = None;
        self.expires_at = None;
    }
}

type SharedState = Arc<RwLock<WaState>>;

fn write_state(state: &SharedState) -> std::sync::RwLockWriteGuard<'_, WaState> {
    state.write().unwrap_or_else(|e| e.into_inner())
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
    state: SharedState,
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
            state: Arc::new(RwLock::new(WaState::default())),
        };
        let socket_path = std::path::PathBuf::from(&cfg.socket);
        tokio::spawn(connection_loop(
            socket_path,
            integration.client.clone(),
            integration.state.clone(),
            router,
        ));

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
        let payload_json = msg
            .payload
            .as_json()
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        const ACTIONS: [&str; 12] = [
            "whatsapp.request_qr",
            "whatsapp.logout",
            "whatsapp.send_text",
            "whatsapp.send_media",
            "whatsapp.react",
            "whatsapp.delete",
            "whatsapp.typing",
            "whatsapp.link_qr",
            "whatsapp.link_phone",
            "whatsapp.link_cancel",
            "whatsapp.list_chats",
            "whatsapp.status",
        ];
        if !ACTIONS.contains(&action) {
            return MtwMessage::error(400, format!("unknown whatsapp action: {action}"))
                .with_ref(&msg.id);
        }
        // Answered from the cache: works while the bridge is down.
        if action == "whatsapp.status" {
            return MtwMessage::response(&msg.id, Payload::Json(self.status_json()));
        }
        // Validate before the connection check so a bad number is a 400
        // regardless of the bridge, and nothing is written for it.
        let phone = if action == "whatsapp.link_phone" {
            match payload_json.get("phone").and_then(|v| v.as_str()) {
                Some(p) if is_valid_phone(p) => Some(p.to_string()),
                _ => {
                    return MtwMessage::error(
                        400,
                        "invalid_phone: expected 8-15 digits, international format without + or leading 0",
                    )
                    .with_ref(&msg.id);
                }
            }
        } else {
            None
        };
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
            "whatsapp.link_qr" => client.link_qr().await.map(|_| None),
            "whatsapp.link_phone" => client
                .link_phone(phone.unwrap_or_default())
                .await
                .map(|_| None),
            "whatsapp.link_cancel" => client.link_cancel().await.map(|_| None),
            "whatsapp.list_chats" => {
                return list_chats_response(&client, &payload_json, &msg.id).await
            }
            _ => unreachable!("action checked against ACTIONS above"),
        };

        match result {
            Ok(Some(id)) => MtwMessage::response(
                &msg.id,
                Payload::Json(serde_json::json!({ "id": id, "queued": true })),
            ),
            Ok(None) => {
                MtwMessage::response(&msg.id, Payload::Json(serde_json::json!({"ok": true})))
            }
            Err(err) => MtwMessage::error(500, err.to_string()).with_ref(&msg.id),
        }
    }

    /// `whatsapp.status` payload: bridge link + cached session state.
    fn status_json(&self) -> serde_json::Value {
        let bridge_connected = self.current_client().is_some();
        let st = self.state.read().unwrap_or_else(|e| e.into_inner()).clone();
        serde_json::json!({
            "bridge_connected": bridge_connected,
            "state": st.state,
            "mode": st.mode,
            "jid": st.jid,
            "lid": st.lid,
            "reason": st.reason,
            "qr": st.qr,
            "pairing_code": st.code,
            "pairing_expires_at": st.expires_at,
        })
    }
}

/// Phone for `link_phone`: digits only, 8-15 of them, no leading `0`
/// (international format without `+`).
fn is_valid_phone(p: &str) -> bool {
    (8..=15).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()) && !p.starts_with('0')
}

async fn list_chats_response(
    client: &WhatsAppClient,
    payload: &serde_json::Value,
    ref_id: &str,
) -> MtwMessage {
    let limit = payload
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(LIST_CHATS_DEFAULT_LIMIT)
        .clamp(1, LIST_CHATS_MAX_LIMIT) as u32;
    match client.list_chats(limit, LIST_CHATS_TIMEOUT).await {
        Ok(items) => {
            MtwMessage::response(ref_id, Payload::Json(serde_json::json!({ "items": items })))
        }
        Err(WhatsAppError::Bridge { code, message }) => MtwMessage::error(
            502,
            format!("{}: {message}", code.as_deref().unwrap_or("bridge_error")),
        )
        .with_ref(ref_id),
        Err(WhatsAppError::Timeout) => {
            MtwMessage::error(504, "timeout: bridge did not answer list_chats in time")
                .with_ref(ref_id)
        }
        Err(WhatsAppError::Closed) => {
            MtwMessage::error(503, "not_connected: whatsapp bridge is not connected")
                .with_ref(ref_id)
        }
        Err(err) => MtwMessage::error(500, err.to_string()).with_ref(ref_id),
    }
}

async fn dispatch_send_text(
    client: &WhatsAppClient,
    payload: &serde_json::Value,
) -> Result<String, WhatsAppError> {
    let to = payload.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
    client.send_text(to, text).await
}

async fn dispatch_send_media(
    client: &WhatsAppClient,
    payload: &serde_json::Value,
) -> Result<String, WhatsAppError> {
    let id = generate_id();
    let to = payload
        .get("to")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let kind = parse_media_kind(
        payload
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("document"),
    );
    let mime = payload
        .get("mime")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream")
        .to_string();
    let data_b64 = payload
        .get("data_b64")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let caption = payload
        .get("caption")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let filename = payload
        .get("filename")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

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

async fn dispatch_react(
    client: &WhatsAppClient,
    payload: &serde_json::Value,
) -> Result<(), WhatsAppError> {
    let id = generate_id();
    client
        .send(Command::React {
            id,
            to: payload
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message_id: payload
                .get("message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            emoji: payload
                .get("emoji")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .await
}

async fn dispatch_delete(
    client: &WhatsAppClient,
    payload: &serde_json::Value,
) -> Result<(), WhatsAppError> {
    let id = generate_id();
    client
        .send(Command::Delete {
            id,
            to: payload
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message_id: payload
                .get("message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            for_everyone: payload
                .get("for_everyone")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        })
        .await
}

async fn dispatch_typing(
    client: &WhatsAppClient,
    payload: &serde_json::Value,
) -> Result<(), WhatsAppError> {
    client
        .send(Command::Typing {
            to: payload
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
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
    for name in [
        channels::INBOUND,
        channels::QR,
        channels::STATUS,
        channels::PAIRING,
    ] {
        if router.channels().get(name).is_none() {
            router.channels().create_channel(name, false, None, 1);
        }
    }
    // whatsapp:events is never retained: an ack/error replayed to a late
    // subscriber would misrepresent it as current (C2.3).
    if router.channels().get(channels::EVENTS).is_none() {
        router
            .channels()
            .create_channel(channels::EVENTS, false, None, 0);
    }
}

/// Dial the bridge forever: on success publish the client and pump its
/// events until the connection drops, then clear it and redial. Failed
/// dials back off from [`BACKOFF_START`] doubling up to [`BACKOFF_MAX`].
async fn connection_loop(
    socket_path: std::path::PathBuf,
    slot: Arc<RwLock<Option<WhatsAppClient>>>,
    state: SharedState,
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
                    _ = pump_events(&mut events, &router, &state) => {}
                    _ = client.closed() => {}
                }
                // The read loop has exited, so nothing new can arrive: publish
                // whatever was still queued (e.g. a final status) before
                // clearing the slot.
                drain_events(&mut events, &router, &state).await;

                *slot.write().unwrap_or_else(|e| e.into_inner()) = None;
                // Without the bridge the session state is unknown again; a
                // stale QR / pairing code must not outlive the connection.
                *write_state(&state) = WaState::default();
                // Push it too (M2): without this, a card already open keeps
                // showing the last state it saw (e.g. "Vinculado") until it
                // reloads and re-reads the cache.
                publish_status_unknown(&router).await;
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
async fn drain_events(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    router: &MtwRouter,
    state: &SharedState,
) {
    use tokio::sync::broadcast::error::TryRecvError;
    loop {
        match events.try_recv() {
            Ok(evt) => publish_event(router, state, evt).await,
            Err(TryRecvError::Lagged(n)) => {
                tracing::warn!(dropped = n, "whatsapp: event pump lagged while draining");
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
        }
    }
}

async fn pump_events(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    router: &MtwRouter,
    state: &SharedState,
) {
    tracing::info!("whatsapp: event pump started");
    loop {
        match events.recv().await {
            Ok(evt) => {
                tracing::info!(event = ?std::mem::discriminant(&evt), "whatsapp: pump got event");
                publish_event(router, state, evt).await;
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

/// Update the state cache from `evt` and publish it on its channel. The
/// cache lock is taken and released synchronously, never across an await.
async fn publish_event(router: &MtwRouter, state: &SharedState, evt: Event) {
    let (channel, payload) = match evt {
        // Not a session state: moved off whatsapp:status so the card never
        // mistakes a one-shot boot notice for the current session (C2.3).
        Event::Ready => (channels::EVENTS, serde_json::json!({ "state": "ready" })),
        Event::Status {
            state: st,
            mode,
            jid,
            lid,
            reason,
        } => {
            {
                let mut cache = write_state(state);
                cache.state = st.clone();
                cache.mode = mode.clone();
                cache.jid = jid.clone();
                cache.lid = lid.clone();
                cache.reason = reason.clone();
                if st != "linking" {
                    cache.clear_linking();
                }
            }
            // bridge_connected is always true here: this event only reaches
            // the pump while the socket to the bridge is up (M2).
            let mut payload = serde_json::json!({ "state": st, "bridge_connected": true });
            for (key, value) in [
                ("mode", mode),
                ("jid", jid),
                ("lid", lid),
                ("reason", reason),
            ] {
                if let Some(v) = value {
                    payload[key] = serde_json::Value::String(v);
                }
            }
            (channels::STATUS, payload)
        }
        Event::Qr { code } => {
            write_state(state).qr = Some(code.clone());
            (channels::QR, serde_json::json!({ "code": code }))
        }
        Event::PairingCode { code, expires_at } => {
            {
                let mut cache = write_state(state);
                cache.code = Some(code.clone());
                cache.expires_at = Some(expires_at);
            }
            (
                channels::PAIRING,
                serde_json::json!({ "code": code, "expires_at": expires_at }),
            )
        }
        // Consumed by `WhatsAppClient::list_chats`; nothing to publish.
        Event::Chats { .. } => return,
        // Not a session state: moved off whatsapp:status (C2.3). Legacy
        // wire type; the current bridge reports pairing via `Event::Status`
        // instead (see protocol.rs docs).
        Event::PairingSuccess { jid } => {
            if jid.is_some() {
                write_state(state).jid = jid.clone();
            }
            (
                channels::EVENTS,
                serde_json::json!({ "state": "paired", "jid": jid }),
            )
        }
        // Legacy wire type; the current bridge reports this via
        // `Event::Status { state: "connected", .. }` instead.
        Event::Connected { jid, lid } => {
            {
                let mut cache = write_state(state);
                cache.state = "connected".to_string();
                cache.jid = jid.clone();
                cache.lid = lid.clone();
                cache.reason = None;
                cache.clear_linking();
            }
            let mut payload =
                serde_json::json!({ "state": "connected", "jid": jid, "bridge_connected": true });
            if let Some(l) = lid {
                payload["lid"] = serde_json::Value::String(l);
            }
            (channels::STATUS, payload)
        }
        // Legacy wire type; the current bridge reports this via
        // `Event::Status { state: "disconnected", .. }` instead.
        Event::Disconnected { reason } => {
            {
                let mut cache = write_state(state);
                cache.state = "disconnected".to_string();
                cache.reason = Some(reason.clone());
                cache.clear_linking();
            }
            (
                channels::STATUS,
                serde_json::json!({ "state": "disconnected", "reason": reason, "bridge_connected": true }),
            )
        }
        Event::Message {
            id,
            from,
            chat,
            is_group,
            group_name,
            author,
            push_name,
            timestamp,
            text,
            reply_to,
            attachments,
            from_me,
            sender_alt,
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
                "from_me": from_me,
                "sender_alt": sender_alt,
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
        // Not a session state: moved off whatsapp:status (C2.3).
        Event::Ack { id, message_id } => (
            channels::EVENTS,
            serde_json::json!({ "state": "ack", "id": id, "message_id": message_id }),
        ),
        // Not a session state: moved off whatsapp:status (C2.3).
        Event::Error { id, code, message } => (
            channels::EVENTS,
            serde_json::json!({ "state": "error", "id": id, "code": code, "message": message }),
        ),
    };

    publish_to_channel(router, channel, payload).await;
}

/// Publish `{"state":"unknown","bridge_connected":false}` on whatsapp:status
/// when the bridge connection drops (M2), so a card already open updates
/// immediately instead of showing a stale state until it reloads.
async fn publish_status_unknown(router: &MtwRouter) {
    publish_to_channel(
        router,
        channels::STATUS,
        serde_json::json!({ "state": "unknown", "bridge_connected": false }),
    )
    .await;
}

async fn publish_to_channel(router: &MtwRouter, channel: &'static str, payload: serde_json::Value) {
    match router.channels().get(channel) {
        Some(ch) => {
            let msg = MtwMessage::new(MsgType::Event, Payload::Json(payload)).with_channel(channel);
            let subs = ch.subscriber_count();
            match ch.publish(msg, None).await {
                Ok(n) => tracing::info!(
                    channel = channel,
                    subscribers = subs,
                    delivered = n,
                    "whatsapp: published event",
                ),
                Err(err) => {
                    tracing::warn!(channel = channel, error = %err, "whatsapp: publish failed")
                }
            }
        }
        None => {
            tracing::warn!(
                channel = channel,
                "whatsapp: channel not found, dropping event"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_router::{ChannelManager, MiddlewareChain};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};

    fn temp_socket() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = mtw_ipc::unique_test_endpoint(dir.path(), "wa");
        (dir, endpoint)
    }

    fn section(socket: &str) -> WhatsAppSection {
        WhatsAppSection {
            enabled: true,
            socket: socket.to_string(),
            connect_timeout_secs: 60,
        }
    }

    fn request(action: &str) -> MtwMessage {
        MtwMessage::new(MsgType::Request, Payload::Json(serde_json::json!({})))
            .with_metadata("action", serde_json::json!(action))
    }

    fn request_with(action: &str, payload: serde_json::Value) -> MtwMessage {
        MtwMessage::new(MsgType::Request, Payload::Json(payload))
            .with_metadata("action", serde_json::json!(action))
    }

    fn json_of(resp: &MtwMessage) -> serde_json::Value {
        resp.payload
            .as_json()
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    /// Start the integration against a fake bridge endpoint and return the
    /// accepted stream split into a line reader and a writer.
    async fn connected_fake() -> (
        tempfile::TempDir,
        Arc<MtwRouter>,
        WhatsAppIntegration,
        tokio::io::Lines<BufReader<ReadHalf<mtw_ipc::IpcStream>>>,
        WriteHalf<mtw_ipc::IpcStream>,
    ) {
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(
            ChannelManager::new(),
            MiddlewareChain::new(),
        ));
        let mut listener = mtw_ipc::IpcListener::bind(&sock).unwrap();
        let wa = WhatsAppIntegration::start(&section(&sock), router.clone())
            .await
            .unwrap()
            .unwrap();
        let stream = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .unwrap()
            .unwrap();
        wait_connected(&wa).await;
        let (read_half, write_half) = tokio::io::split(stream);
        (
            dir,
            router,
            wa,
            BufReader::new(read_half).lines(),
            write_half,
        )
    }

    async fn next_line(
        lines: &mut tokio::io::Lines<BufReader<ReadHalf<mtw_ipc::IpcStream>>>,
    ) -> String {
        tokio::time::timeout(Duration::from_secs(2), lines.next_line())
            .await
            .expect("bridge should receive a line")
            .unwrap()
            .unwrap()
    }

    /// Poll `whatsapp.status` until `pred` holds on its payload.
    async fn wait_status(
        wa: &WhatsAppIntegration,
        pred: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let req = request("whatsapp.status");
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let resp = wa.handle_action("whatsapp.status", &req).await;
                assert_eq!(
                    resp.msg_type,
                    MsgType::Response,
                    "status must not error: {resp:?}"
                );
                let body = json_of(&resp);
                if pred(&body) {
                    return body;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("status should reach the expected state")
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
        let router = Arc::new(MtwRouter::new(
            ChannelManager::new(),
            MiddlewareChain::new(),
        ));

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
        let mut listener = mtw_ipc::IpcListener::bind(&sock).unwrap();
        let stream = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .expect("integration should dial the bridge")
            .unwrap();
        wait_connected(&wa).await;

        let resp = wa.handle_action("whatsapp.request_qr", &req).await;
        assert_eq!(resp.msg_type, MsgType::Response);

        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut lines = BufReader::new(read_half).lines();
        let line = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(line, r#"{"type":"request_qr"}"#);

        // Events from the bridge are forwarded to the whatsapp:* channels.
        write_half
            .write_all(b"{\"type\":\"qr\",\"code\":\"2@abc\"}\n")
            .await
            .unwrap();
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

        drop(dir);
    }

    #[tokio::test]
    async fn status_sent_right_before_hangup_reaches_history() {
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(
            ChannelManager::new(),
            MiddlewareChain::new(),
        ));
        let mut listener = mtw_ipc::IpcListener::bind(&sock).unwrap();
        let wa = WhatsAppIntegration::start(&section(&sock), router.clone())
            .await
            .unwrap()
            .unwrap();
        let mut stream = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .unwrap()
            .unwrap();
        wait_connected(&wa).await;

        // A burst of events followed by a disconnected status and then a
        // final QR line, then an immediate hang-up: none of the burst may
        // be lost when the drop is seen — including the very last line,
        // which is the one most exposed to an off-by-one in drain_events.
        let mut burst = Vec::new();
        for i in 0..100 {
            burst.extend_from_slice(format!("{{\"type\":\"qr\",\"code\":\"c{i}\"}}\n").as_bytes());
        }
        burst.extend_from_slice(b"{\"type\":\"disconnected\",\"reason\":\"final\"}\n");
        burst.extend_from_slice(b"{\"type\":\"qr\",\"code\":\"final\"}\n");
        stream.write_all(&burst).await.unwrap();
        stream.flush().await.unwrap();
        drop(stream);

        // The burst's very last line must have been drained, not dropped,
        // by the race between the pump noticing EOF and the events still
        // queued ahead of it.
        let qr = router.channels().get(channels::QR).unwrap();
        let got = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let hist = qr.get_history(None).await;
                if hist.last().and_then(|m| m.payload.as_json().cloned())
                    == Some(serde_json::json!({"code": "final"}))
                {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(got.is_ok(), "the burst's last queued event was dropped");

        // Once the drop itself is noticed, whatsapp:status must read
        // "unknown"/bridge_connected:false (M2) — overriding whatever
        // session state (here "final") was last cached, since the bridge
        // really is gone.
        let status = router.channels().get(channels::STATUS).unwrap();
        let got = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let hist = status.get_history(None).await;
                if let Some(last) = hist.last() {
                    let p = last.payload.as_json().unwrap();
                    if p["state"] == "unknown" && p["bridge_connected"] == false {
                        return true;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            got.is_ok(),
            "bridge drop was never pushed as the final status"
        );

        drop(dir);
    }

    #[tokio::test]
    async fn reconnects_after_bridge_drops() {
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(
            ChannelManager::new(),
            MiddlewareChain::new(),
        ));
        let mut listener = mtw_ipc::IpcListener::bind(&sock).unwrap();
        let wa = WhatsAppIntegration::start(&section(&sock), router)
            .await
            .unwrap()
            .unwrap();

        let first = tokio::time::timeout(Duration::from_secs(8), listener.accept())
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
        assert!(is_not_connected(
            &wa.handle_action("whatsapp.logout", &req).await
        ));

        let second = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .expect("integration should redial the bridge")
            .unwrap();
        wait_connected(&wa).await;
        let resp = wa.handle_action("whatsapp.logout", &req).await;
        assert_eq!(resp.msg_type, MsgType::Response);
        let mut lines = BufReader::new(second).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        assert_eq!(line, r#"{"type":"logout"}"#);

        drop(dir);
    }

    #[tokio::test]
    async fn status_reports_unknown_without_bridge() {
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(
            ChannelManager::new(),
            MiddlewareChain::new(),
        ));
        let wa = WhatsAppIntegration::start(&section(&sock), router)
            .await
            .unwrap()
            .unwrap();
        let resp = wa
            .handle_action("whatsapp.status", &request("whatsapp.status"))
            .await;
        assert_eq!(resp.msg_type, MsgType::Response, "unexpected: {resp:?}");
        let body = json_of(&resp);
        assert_eq!(body["bridge_connected"], false);
        assert_eq!(body["state"], "unknown");
        drop(dir);
    }

    #[tokio::test]
    async fn status_carries_last_qr_and_pairing_code() {
        let (dir, router, wa, _lines, mut w) = connected_fake().await;

        w.write_all(b"{\"type\":\"status\",\"state\":\"linking\",\"mode\":\"phone\"}\n")
            .await
            .unwrap();
        w.write_all(
            b"{\"type\":\"pairing_code\",\"code\":\"K3M9QX2P\",\"expires_at\":1790000000}\n",
        )
        .await
        .unwrap();

        let body = wait_status(&wa, |b| b["pairing_code"] == "K3M9QX2P").await;
        assert_eq!(body["bridge_connected"], true);
        assert_eq!(body["state"], "linking");
        assert_eq!(body["mode"], "phone");
        assert_eq!(body["pairing_expires_at"], 1790000000);

        let pairing = router
            .channels()
            .get(channels::PAIRING)
            .expect("pairing channel exists");
        let hist = pairing.get_history(None).await;
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].payload.as_json().unwrap()["code"], "K3M9QX2P");
        assert_eq!(hist[0].payload.as_json().unwrap()["expires_at"], 1790000000);

        // A QR rotation while linking is cached too.
        w.write_all(b"{\"type\":\"qr\",\"code\":\"2@xyz\"}\n")
            .await
            .unwrap();
        wait_status(&wa, |b| b["qr"] == "2@xyz").await;

        w.write_all(b"{\"type\":\"status\",\"state\":\"connected\",\"jid\":\"5491100000000@s.whatsapp.net\",\"lid\":\"123456@lid\"}\n")
            .await
            .unwrap();
        let body = wait_status(&wa, |b| b["state"] == "connected").await;
        assert_eq!(body["jid"], "5491100000000@s.whatsapp.net");
        assert_eq!(body["lid"], "123456@lid");
        assert!(
            body.get("qr").is_none_or(|v| v.is_null()),
            "qr not cleared: {body}"
        );
        assert!(
            body.get("pairing_code").is_none_or(|v| v.is_null()),
            "code not cleared: {body}"
        );
        assert!(body.get("pairing_expires_at").is_none_or(|v| v.is_null()));

        // whatsapp:status carries the new payload shape, nulls omitted.
        let status = router.channels().get(channels::STATUS).unwrap();
        let hist = status.get_history(None).await;
        let last = hist.last().unwrap().payload.as_json().unwrap().clone();
        assert_eq!(
            last,
            serde_json::json!({
                "state": "connected",
                "jid": "5491100000000@s.whatsapp.net",
                "lid": "123456@lid",
                "bridge_connected": true,
            })
        );

        drop(dir);
    }

    #[tokio::test]
    async fn status_channel_never_carries_ack_ready_or_error() {
        // C2.3: ack/ready/error must land on whatsapp:events, never on
        // whatsapp:status — otherwise pressing "Mandar prueba" on a linked
        // card would flip it to "Sin vincular" the moment the bridge acks
        // the send (final-review.md C2, failure (a)).
        let (dir, router, wa, mut lines, mut w) = connected_fake().await;

        w.write_all(
            b"{\"type\":\"status\",\"state\":\"connected\",\"jid\":\"549@s.whatsapp.net\"}\n",
        )
        .await
        .unwrap();
        wait_status(&wa, |b| b["state"] == "connected").await;

        w.write_all(b"{\"type\":\"ready\"}\n").await.unwrap();
        w.write_all(b"{\"type\":\"error\",\"message\":\"boom\"}\n")
            .await
            .unwrap();

        let fake = tokio::spawn(async move {
            let line = next_line(&mut lines).await;
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["type"], "send_text");
            let reply =
                serde_json::json!({"type": "ack", "id": req["id"], "message_id": "wamid-1"});
            w.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
            w
        });
        let send = request_with(
            "whatsapp.send_text",
            serde_json::json!({"to": "5491123456789", "text": "hola"}),
        );
        let resp = wa.handle_action("whatsapp.send_text", &send).await;
        assert_eq!(resp.msg_type, MsgType::Response, "unexpected: {resp:?}");
        let _w = fake.await.unwrap();

        // Give the pump a moment to process the three events above, then
        // check nothing clobbered the session state: whatsapp.status (the
        // cache) and whatsapp:status's retained history must still read
        // "connected", never "ack"/"ready"/"error".
        tokio::time::sleep(Duration::from_millis(100)).await;
        let body = wa
            .handle_action("whatsapp.status", &request("whatsapp.status"))
            .await;
        assert_eq!(
            json_of(&body)["state"],
            "connected",
            "ack/ready/error leaked into the cache"
        );

        let status = router.channels().get(channels::STATUS).unwrap();
        let hist = status.get_history(None).await;
        let last = hist.last().unwrap().payload.as_json().unwrap().clone();
        assert_eq!(
            last["state"], "connected",
            "whatsapp:status was clobbered by a non-session event"
        );

        // history=0 on whatsapp:events: the channel exists but never retains.
        let events = router
            .channels()
            .get(channels::EVENTS)
            .expect("events channel exists");
        assert!(events.get_history(None).await.is_empty());

        drop(dir);
    }

    #[tokio::test]
    async fn bridge_drop_pushes_unknown_status() {
        // M2: an open card must not be left showing a stale state (e.g.
        // "Vinculado") until it reloads — the drop itself must be pushed.
        let (dir, sock) = temp_socket();
        let router = Arc::new(MtwRouter::new(
            ChannelManager::new(),
            MiddlewareChain::new(),
        ));
        let mut listener = mtw_ipc::IpcListener::bind(&sock).unwrap();
        let wa = WhatsAppIntegration::start(&section(&sock), router.clone())
            .await
            .unwrap()
            .unwrap();

        let stream = tokio::time::timeout(Duration::from_secs(8), listener.accept())
            .await
            .unwrap()
            .unwrap();
        wait_connected(&wa).await;

        // Reach "connected" first so there is a stale state to correct.
        let mut write_half = stream;
        write_half
            .write_all(
                b"{\"type\":\"status\",\"state\":\"connected\",\"jid\":\"549@s.whatsapp.net\"}\n",
            )
            .await
            .unwrap();
        wait_status(&wa, |b| b["state"] == "connected").await;

        drop(write_half);

        let status = router.channels().get(channels::STATUS).unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let hist = status.get_history(None).await;
                if let Some(last) = hist.last() {
                    let p = last.payload.as_json().unwrap();
                    if p["state"] == "unknown" && p["bridge_connected"] == false {
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            got.is_ok(),
            "bridge drop was never pushed on whatsapp:status"
        );

        // The cached whatsapp.status answer agrees.
        let body = wait_status(&wa, |b| b["state"] == "unknown").await;
        assert_eq!(body["bridge_connected"], false);

        drop(dir);
    }

    #[tokio::test]
    async fn link_phone_forwards_and_validates() {
        let (dir, _router, wa, mut lines, _w) = connected_fake().await;

        let bad = request_with("whatsapp.link_phone", serde_json::json!({"phone": "+54 9"}));
        let resp = wa.handle_action("whatsapp.link_phone", &bad).await;
        assert_eq!(resp.msg_type, MsgType::Error);
        assert_eq!(json_of(&resp)["code"], 400);
        assert!(json_of(&resp)["message"]
            .as_str()
            .unwrap()
            .starts_with("invalid_phone"));

        let missing = request("whatsapp.link_phone");
        let resp = wa.handle_action("whatsapp.link_phone", &missing).await;
        assert_eq!(json_of(&resp)["code"], 400);

        let leading_zero = request_with(
            "whatsapp.link_phone",
            serde_json::json!({"phone": "01123456789"}),
        );
        let resp = wa.handle_action("whatsapp.link_phone", &leading_zero).await;
        assert_eq!(json_of(&resp)["code"], 400);

        let good = request_with(
            "whatsapp.link_phone",
            serde_json::json!({"phone": "5491123456789"}),
        );
        let resp = wa.handle_action("whatsapp.link_phone", &good).await;
        assert_eq!(resp.msg_type, MsgType::Response, "unexpected: {resp:?}");
        assert_eq!(json_of(&resp), serde_json::json!({"ok": true}));

        // The invalid calls wrote nothing: the first line is the valid one.
        assert_eq!(
            next_line(&mut lines).await,
            r#"{"type":"link_phone","phone":"5491123456789"}"#
        );

        drop(dir);
    }

    #[tokio::test]
    async fn list_chats_returns_items() {
        let (dir, _router, wa, mut lines, mut w) = connected_fake().await;

        let fake = tokio::spawn(async move {
            let line = next_line(&mut lines).await;
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["type"], "list_chats");
            assert_eq!(req["limit"], 200, "limit is clamped to 200");
            let reply = serde_json::json!({
                "type": "chats",
                "id": req["id"],
                "items": [{"jid": "g@g.us", "name": "Familia", "is_group": true, "last_ts": 5}],
            });
            w.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
            w
        });

        let req = request_with("whatsapp.list_chats", serde_json::json!({"limit": 5000}));
        let resp = wa.handle_action("whatsapp.list_chats", &req).await;
        assert_eq!(resp.msg_type, MsgType::Response, "unexpected: {resp:?}");
        assert_eq!(
            json_of(&resp),
            serde_json::json!({"items": [{"jid": "g@g.us", "name": "Familia", "is_group": true, "last_ts": 5}]})
        );
        let _w = fake.await.unwrap();

        drop(dir);
    }

    #[tokio::test]
    async fn list_chats_maps_bridge_error_to_502() {
        let (dir, _router, wa, mut lines, mut w) = connected_fake().await;

        let fake = tokio::spawn(async move {
            let line = next_line(&mut lines).await;
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["limit"], 50, "default limit is 50");
            let reply = serde_json::json!({
                "type": "error", "id": req["id"],
                "code": "not_connected", "message": "session is not connected",
            });
            w.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
            w
        });

        let resp = wa
            .handle_action("whatsapp.list_chats", &request("whatsapp.list_chats"))
            .await;
        assert_eq!(resp.msg_type, MsgType::Error);
        assert_eq!(json_of(&resp)["code"], 502);
        assert_eq!(
            json_of(&resp)["message"],
            "not_connected: session is not connected"
        );
        let _w = fake.await.unwrap();

        drop(dir);
    }

    #[tokio::test]
    async fn link_qr_and_cancel_forward() {
        let (dir, _router, wa, mut lines, _w) = connected_fake().await;

        let resp = wa
            .handle_action("whatsapp.link_qr", &request("whatsapp.link_qr"))
            .await;
        assert_eq!(json_of(&resp), serde_json::json!({"ok": true}));
        assert_eq!(next_line(&mut lines).await, r#"{"type":"link_qr"}"#);

        let resp = wa
            .handle_action("whatsapp.link_cancel", &request("whatsapp.link_cancel"))
            .await;
        assert_eq!(json_of(&resp), serde_json::json!({"ok": true}));
        assert_eq!(next_line(&mut lines).await, r#"{"type":"link_cancel"}"#);

        drop(dir);
    }
}
