//! Typed social events. Each variant maps to a channel and is carried inside a
//! `MtwMessage` payload so it can be signed, federated and routed through the
//! existing mtwRequest infrastructure.

use mtw_identity::MtwIdentity;
use mtw_protocol::{MsgType, MtwMessage, Payload};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SocialError};

pub const CHANNEL_PROFILES: &str = "social.profiles";
pub const CHANNEL_POSTS: &str = "social.posts";
pub const CHANNEL_FOLLOWS: &str = "social.follows";
pub const CHANNEL_REACTIONS: &str = "social.reactions";
pub const CHANNEL_REPOSTS: &str = "social.reposts";
pub const CHANNEL_REPLIES: &str = "social.replies";
pub const CHANNEL_DELETES: &str = "social.deletes";

/// All first-class social event kinds.
///
/// `Profile` and `Follow` are *replaceable*: only the latest per author wins.
/// Everything else is append-only (with `Delete` providing a cooperative
/// soft-delete signal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SocialEvent {
    Profile(Profile),
    Post(Post),
    Follow(Follow),
    Reaction(Reaction),
    Repost(Repost),
    Reply(Reply),
    Delete(Delete),
}

impl SocialEvent {
    /// Channel a given event publishes to / is consumed from.
    pub fn channel(&self) -> &'static str {
        match self {
            SocialEvent::Profile(_) => CHANNEL_PROFILES,
            SocialEvent::Post(_) => CHANNEL_POSTS,
            SocialEvent::Follow(_) => CHANNEL_FOLLOWS,
            SocialEvent::Reaction(_) => CHANNEL_REACTIONS,
            SocialEvent::Repost(_) => CHANNEL_REPOSTS,
            SocialEvent::Reply(_) => CHANNEL_REPLIES,
            SocialEvent::Delete(_) => CHANNEL_DELETES,
        }
    }

    /// Convert the event into a signed `MtwMessage` ready for routing or
    /// federation.
    pub fn into_message(self, identity: &MtwIdentity) -> Result<MtwMessage> {
        let channel = self.channel();
        let payload = Payload::Json(serde_json::to_value(&self)?);
        let mut msg = MtwMessage::new(MsgType::Event, payload).with_channel(channel);
        identity.sign_message(&mut msg)?;
        Ok(msg)
    }

    /// Decode a `SocialEvent` from a `MtwMessage`. The signature **must**
    /// already have been verified by the caller via
    /// [`mtw_identity::verify_message`].
    pub fn from_message(msg: &MtwMessage) -> Result<Self> {
        let value = match &msg.payload {
            Payload::Json(v) => v,
            _ => return Err(SocialError::InvalidPayload),
        };
        let event: SocialEvent = serde_json::from_value(value.clone())?;
        Ok(event)
    }
}

/// Author profile. Replaceable — keep only the latest per pubkey.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
}

/// A short-form note (think tweet/skeet/note). Append-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

/// A follow list. Replaceable — the latest per pubkey supersedes prior lists.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Follow {
    pub follows: Vec<String>,
}

/// A reaction targeting another event (the "content" is freeform — emoji,
/// "+", "-", whatever; matches Nostr semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub target_id: String,
    pub target_pubkey: String,
    pub content: String,
}

/// A repost (verbatim share) of another event, with optional comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repost {
    pub target_id: String,
    pub target_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// A reply that references a parent post (and optionally the thread root).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub content: String,
    pub parent_id: String,
    pub parent_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
}

/// Cooperative deletion of one or more events authored by the sender.
/// Cooperative because relays may or may not honor it — see Nostr NIP-09.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delete {
    pub target_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attachment {
    Image {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    Video {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    Link {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
}
