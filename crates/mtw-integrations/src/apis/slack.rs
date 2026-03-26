//! Slack API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "slack",
    name: "Slack",
    base_url: "https://slack.com/api",
    docs_url: "https://api.slack.com/",
    oauth2_supported: true,
};

pub mod scopes {
    pub const CHAT_WRITE: &str = "chat:write";
    pub const CHANNELS_READ: &str = "channels:read";
    pub const CHANNELS_HISTORY: &str = "channels:history";
    pub const USERS_READ: &str = "users:read";
    pub const FILES_WRITE: &str = "files:write";
    pub const REACTIONS_WRITE: &str = "reactions:write";
    pub const IM_WRITE: &str = "im:write";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    /// Bot token (xoxb-...).
    pub bot_token: String,
    /// App-level token for socket mode (xapp-...).
    pub app_token: Option<String>,
    /// Signing secret for verifying incoming webhooks.
    pub signing_secret: Option<String>,
    /// Default channel to post to.
    pub default_channel: Option<String>,
}

pub struct SlackClient {
    config: SlackConfig,
    status: IntegrationStatus,
}

impl SlackClient {
    pub fn new(config: SlackConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &SlackConfig {
        &self.config
    }

    pub fn status(&self) -> &IntegrationStatus {
        &self.status
    }

    pub fn info() -> &'static IntegrationInfo {
        &INFO
    }

    pub async fn connect(&mut self) -> Result<(), String> {
        self.status = IntegrationStatus::Connected;
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.status = IntegrationStatus::Disconnected;
    }
}
