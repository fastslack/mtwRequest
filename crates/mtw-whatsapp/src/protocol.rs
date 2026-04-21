//! Wire types for the whatsapp-bridge socket protocol. Mirrors
//! `services/whatsapp-bridge/protocol.md`. Both directions live here so
//! users of the crate never need to remember the discriminator strings.

use serde::{Deserialize, Serialize};

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

/// Everything the bridge can tell the driver about.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Sidecar booted.
    Ready,

    /// A QR code string needs to be shown to the user. WhatsApp rotates
    /// these roughly every 20 seconds until somebody scans one or the
    /// login attempt times out.
    Qr { code: String },

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
        #[serde(default)]
        attachments: Vec<InboundAttachment>,
    },

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
