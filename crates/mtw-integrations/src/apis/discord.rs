//! Discord API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "discord",
    name: "Discord",
    base_url: "https://discord.com/api/v10",
    docs_url: "https://discord.com/developers/docs",
    oauth2_supported: true,
};

pub mod scopes {
    pub const BOT: &str = "bot";
    pub const IDENTIFY: &str = "identify";
    pub const GUILDS: &str = "guilds";
    pub const MESSAGES_READ: &str = "messages.read";
    pub const APPLICATIONS_COMMANDS: &str = "applications.commands";
    pub const WEBHOOK_INCOMING: &str = "webhook.incoming";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Bot token.
    pub bot_token: String,
    /// Application ID.
    pub application_id: String,
    /// Guild ID for guild-specific commands.
    pub guild_id: Option<String>,
    /// Webhook URL for simple message posting.
    pub webhook_url: Option<String>,
}

pub struct DiscordClient {
    config: DiscordConfig,
    status: IntegrationStatus,
}

impl DiscordClient {
    pub fn new(config: DiscordConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &DiscordConfig {
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
