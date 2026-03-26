//! # mtw-integrations
//!
//! Third-party API integrations, AI model providers, OAuth2 support, and RSS
//! feed handling for the mtwRequest framework.
//!
//! This crate provides stub implementations that define the types, config
//! structs, and trait implementations needed to integrate with external
//! services. Actual HTTP/API calls will be implemented in future phases.

pub mod ai;
pub mod apis;
pub mod oauth2;
pub mod rss;

/// Re-export commonly used types.
pub mod prelude {
    pub use crate::ai::*;
    pub use crate::oauth2::{OAuth2Client, OAuth2Config, OAuth2TokenResponse};
    pub use crate::rss::{RssConfig, RssFeed, RssItem};
}
