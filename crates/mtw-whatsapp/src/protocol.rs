//! Wire types for the whatsapp-bridge socket protocol. Mirrors
//! `services/whatsapp-bridge/protocol.md`. Both directions live here so
//! users of the crate never need to remember the discriminator strings.

use serde::{Deserialize, Deserializer, Serialize};

/// Helper that treats a JSON `null` as an empty vec during deserialization.
/// Needed because the Go whatsapp-bridge emits `"attachments": null` when a
/// message has no media — plain `#[serde(default)]` only covers the field
/// being absent, not present-but-null, and would otherwise reject the event.
fn null_as_empty_vec<'de, D, T>(de: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(de)?.unwrap_or_default())
}

// ── Driver → Bridge ───────────────────────────────────────────────────

/// The kinds of media WhatsApp accepts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Voice,
    Document,
}

/// Everything the driver can ask the bridge to do.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Restart the login flow; the next `qr` event follows.
    RequestQr,

    /// Send a plain-text message.
    SendText {
        id: String,
        to: String,
        text: String,
    },

    /// Send media with optional caption. `data_b64` is base64-encoded bytes.
    SendMedia {
        id: String,
        to: String,
        kind: MediaKind,
        mime: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        data_b64: String,
    },

    /// React to a message with an emoji. Pass `""` to clear a reaction.
    React {
        id: String,
        to: String,
        message_id: String,
        emoji: String,
    },

    /// Delete a message. `for_everyone = true` requests a revoke.
    Delete {
        id: String,
        to: String,
        message_id: String,
        for_everyone: bool,
    },

    /// "Typing…" indicator for a chat.
    Typing { to: String },

    /// End the session and drop stored credentials.
    Logout,

    /// Start (or restart) the login flow using a scanned QR code. Any
    /// linking attempt already in progress is aborted first.
    LinkQr,

    /// Start (or restart) the login flow using whatsmeow's phone-pairing
    /// code instead of a QR scan. `phone`: digits only, 8-15 digits, not
    /// leading `0`.
    LinkPhone { phone: String },

    /// Abort the linking attempt in progress (QR or phone) and go back to
    /// `idle`. A no-op if nothing is linking.
    LinkCancel,

    /// List contacts and joined groups. Requires `status` `connected`;
    /// replies with the `chats` event correlated by `id`.
    ListChats { id: String, limit: u32 },
}

// ── Bridge → Driver ───────────────────────────────────────────────────

/// An attachment on an inbound message.
#[derive(Debug, Clone, Deserialize)]
pub struct InboundAttachment {
    pub kind: MediaKind,
    pub mime: String,
    pub filename: Option<String>,
    /// Base64-encoded raw bytes.
    pub data_b64: String,
    pub caption: Option<String>,
}

/// An entry in a `chats` reply: a contact or a joined group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatItem {
    pub jid: String,
    pub name: String,
    #[serde(default)]
    pub is_group: bool,
    #[serde(default)]
    pub last_ts: i64,
}

/// Everything the bridge can tell the driver about.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Sidecar booted.
    Ready,

    /// The session's state machine. The single source of truth for session
    /// state; replaces the standalone `pairing_success`/`connected`/
    /// `disconnected` event types for bridges that speak the new protocol.
    Status {
        state: String,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        jid: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },

    /// A QR code string needs to be shown to the user. WhatsApp rotates
    /// these roughly every 20 seconds until somebody scans one or the
    /// login attempt times out.
    Qr { code: String },

    /// The 8-character code to enter on the phone, emitted once per
    /// `link_phone` attempt. `expires_at` is a Unix timestamp (seconds).
    PairingCode { code: String, expires_at: i64 },

    /// The device has just been paired. `Connected` follows.
    PairingSuccess {
        #[serde(default)]
        jid: Option<String>,
    },

    /// Socket is connected and authenticated. Safe to send.
    Connected {
        #[serde(default)]
        jid: Option<String>,
    },

    /// Socket dropped. Auto-reconnect is internal.
    Disconnected { reason: String },

    /// Inbound message.
    Message {
        id: String,
        from: String,
        chat: String,
        is_group: bool,
        #[serde(default)]
        group_name: Option<String>,
        author: String,
        #[serde(default)]
        push_name: Option<String>,
        timestamp: i64,
        #[serde(default)]
        text: String,
        #[serde(default)]
        reply_to: Option<String>,
        #[serde(default, deserialize_with = "null_as_empty_vec")]
        attachments: Vec<InboundAttachment>,
    },

    /// Reply to `list_chats`: every contact plus every joined group,
    /// capped at the requested `limit`, correlated by `id`.
    Chats { id: String, items: Vec<ChatItem> },

    /// A command completed successfully.
    Ack {
        id: String,
        #[serde(default)]
        message_id: Option<String>,
    },

    /// Something went wrong. `id` may be absent (fatal / out-of-band).
    Error {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        code: Option<String>,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_serializes_link_qr() {
        let json = serde_json::to_string(&Command::LinkQr).unwrap();
        assert_eq!(json, r#"{"type":"link_qr"}"#);
    }

    #[test]
    fn command_serializes_link_phone() {
        let cmd = Command::LinkPhone {
            phone: "5491123456789".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"link_phone","phone":"5491123456789"}"#);
    }

    #[test]
    fn command_serializes_link_cancel() {
        let json = serde_json::to_string(&Command::LinkCancel).unwrap();
        assert_eq!(json, r#"{"type":"link_cancel"}"#);
    }

    #[test]
    fn command_serializes_list_chats() {
        let cmd = Command::ListChats {
            id: "r1".into(),
            limit: 50,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"list_chats","id":"r1","limit":50}"#);
    }

    #[test]
    fn event_parse_status() {
        let json = r#"{"type":"status","state":"linking","mode":"qr"}"#;
        let evt: Event = serde_json::from_str(json).unwrap();
        match evt {
            Event::Status { state, mode, jid, reason } => {
                assert_eq!(state, "linking");
                assert_eq!(mode.as_deref(), Some("qr"));
                assert!(jid.is_none());
                assert!(reason.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn event_parse_pairing_code() {
        let json = r#"{"type":"pairing_code","code":"K3M9QX2P","expires_at":1790000000}"#;
        let evt: Event = serde_json::from_str(json).unwrap();
        match evt {
            Event::PairingCode { code, expires_at } => {
                assert_eq!(code, "K3M9QX2P");
                assert_eq!(expires_at, 1790000000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn event_parse_chats() {
        let json = r#"{"type":"chats","id":"r1","items":[{"jid":"g@g.us","name":"Familia","is_group":true,"last_ts":0}]}"#;
        let evt: Event = serde_json::from_str(json).unwrap();
        match evt {
            Event::Chats { id, items } => {
                assert_eq!(id, "r1");
                assert_eq!(items.len(), 1);
                assert_eq!(
                    items[0],
                    ChatItem {
                        jid: "g@g.us".into(),
                        name: "Familia".into(),
                        is_group: true,
                        last_ts: 0,
                    }
                );
            }
            _ => panic!("wrong variant"),
        }
    }
}
