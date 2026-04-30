//! Decentralized social primitives — profiles, posts, follows, reactions,
//! replies, deletes — built on top of signed `MtwMessage` events.
//!
//! Each social action is a `SocialEvent` carried inside a `MtwMessage`
//! payload, signed by [`mtw_identity::MtwIdentity`], and routed/federated
//! through the existing `mtw-router` and `mtw-federation` infrastructure.
//!
//! ```no_run
//! use std::sync::Arc;
//! use mtw_identity::MtwIdentity;
//! use mtw_social::{SocialService, Profile};
//!
//! let identity = Arc::new(MtwIdentity::generate());
//! let social = SocialService::new(identity);
//!
//! let _ = social.update_profile(Profile {
//!     display_name: Some("Alice".into()),
//!     bio: Some("hello world".into()),
//!     ..Default::default()
//! }).unwrap();
//!
//! let post = social.create_post("first post!", vec![], vec![]).unwrap();
//! // `post` is a signed MtwMessage ready to be routed/federated.
//! # let _ = post;
//! ```

pub mod error;
pub mod event;
pub mod service;

pub use error::{Result, SocialError};
pub use event::{
    Attachment, Delete, Follow, Post, Profile, Reaction, Reply, Repost, SocialEvent,
    CHANNEL_DELETES, CHANNEL_FOLLOWS, CHANNEL_POSTS, CHANNEL_PROFILES, CHANNEL_REACTIONS,
    CHANNEL_REPLIES, CHANNEL_REPOSTS,
};
pub use service::{AnnotatedPost, AnnotatedReply, SocialService};
