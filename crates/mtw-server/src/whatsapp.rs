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

use std::sync::Arc;

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

/// Live handle to the WhatsApp bridge kept alive for the lifetime of the
/// server process.
#[derive(Clone)]
pub struct WhatsAppIntegration {
    client: WhatsAppClient,
}

impl WhatsAppIntegration {
    /// Connect to the sidecar and start pumping events onto `router`'s
    /// channels. Returns `Ok(None)` if the section is disabled; errors
    /// only on catastrophic failures that should abort boot.
    pub async fn start(
        cfg: &WhatsAppSection,
        router: Arc<MtwRouter>,
    ) -> Result<Option<Self>, MtwError> {
        if !cfg.enabled {
            tracing::info!("whatsapp: disabled in config, skipping");
            return Ok(None);
        }

        let wa_cfg = WhatsAppConfig {
            socket_path: std::path::PathBuf::from(&cfg.socket),
            connect_timeout: Duration::from_secs(cfg.connect_timeout_secs),
            ..Default::default()
        };

        let client = match WhatsAppClient::connect(wa_cfg).await {
            Ok(c) => c,
            Err(WhatsAppError::ConnectTimeout(path, elapsed)) => {
                // Not fatal — the sidecar may come up later. Warn and move
                // on; the rest of the server keeps running.
                tracing::warn!(
                    path = %path.display(),
                    timeout = ?elapsed,
                    "whatsapp: bridge socket not reachable, integration disabled this boot"
                );
                return Ok(None);
            }
            Err(err) => {
                return Err(MtwError::module("whatsapp", format!("connect: {err}")));
            }
        };

        ensure_channels(&router);
        spawn_event_pump(client.clone(), router);

        tracing::info!(socket = %cfg.socket, "whatsapp: bridge connected");
        Ok(Some(Self { client }))
    }

    /// Dispatch a `whatsapp.*` action that arrived as a Request. Returns
    /// the response message to forward back to the caller.
    pub async fn handle_action(&self, action: &str, msg: &MtwMessage) -> MtwMessage {
        let payload_json = msg.payload.as_json().cloned().unwrap_or(serde_json::Value::Null);

        let result: Result<Option<String>, WhatsAppError> = match action {
            "whatsapp.request_qr" => self.client.request_qr().await.map(|_| None),
            "whatsapp.logout" => self.client.logout().await.map(|_| None),
            "whatsapp.send_text" => self.dispatch_send_text(&payload_json).await.map(Some),
            "whatsapp.send_media" => self.dispatch_send_media(&payload_json).await.map(Some),
            "whatsapp.react" => self.dispatch_react(&payload_json).await.map(|_| None),
            "whatsapp.delete" => self.dispatch_delete(&payload_json).await.map(|_| None),
            "whatsapp.typing" => self.dispatch_typing(&payload_json).await.map(|_| None),
            _ => {
                return MtwMessage::error(400, format!("unknown whatsapp action: {action}"))
                    .with_ref(&msg.id);
            }
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

    async fn dispatch_send_text(&self, payload: &serde_json::Value) -> Result<String, WhatsAppError> {
        let to = payload.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
        self.client.send_text(to, text).await
    }

    async fn dispatch_send_media(&self, payload: &serde_json::Value) -> Result<String, WhatsAppError> {
        let id = generate_id();
        let to = payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kind = parse_media_kind(payload.get("kind").and_then(|v| v.as_str()).unwrap_or("document"));
        let mime = payload.get("mime").and_then(|v| v.as_str()).unwrap_or("application/octet-stream").to_string();
        let data_b64 = payload.get("data_b64").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let caption = payload.get("caption").and_then(|v| v.as_str()).map(|s| s.to_string());
        let filename = payload.get("filename").and_then(|v| v.as_str()).map(|s| s.to_string());

        self.client
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

    async fn dispatch_react(&self, payload: &serde_json::Value) -> Result<(), WhatsAppError> {
        let id = generate_id();
        self.client
            .send(Command::React {
                id,
                to: payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                message_id: payload.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                emoji: payload.get("emoji").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
            .await
    }

    async fn dispatch_delete(&self, payload: &serde_json::Value) -> Result<(), WhatsAppError> {
        let id = generate_id();
        self.client
            .send(Command::Delete {
                id,
                to: payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                message_id: payload.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                for_everyone: payload.get("for_everyone").and_then(|v| v.as_bool()).unwrap_or(true),
            })
            .await
    }

    async fn dispatch_typing(&self, payload: &serde_json::Value) -> Result<(), WhatsAppError> {
        self.client
            .send(Command::Typing {
                to: payload.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
            .await
    }
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

fn spawn_event_pump(client: WhatsAppClient, router: Arc<MtwRouter>) {
    let mut events = client.subscribe();
    tokio::spawn(async move {
        tracing::info!("whatsapp: event pump started");
        loop {
            match events.recv().await {
                Ok(evt) => {
                    tracing::info!(event = ?std::mem::discriminant(&evt), "whatsapp: pump got event");
                    publish_event(&router, evt).await;
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
    });
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
