//! Telegram Bot API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "telegram",
    name: "Telegram",
    base_url: "https://api.telegram.org",
    docs_url: "https://core.telegram.org/bots/api",
    oauth2_supported: false,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Bot token from BotFather.
    pub bot_token: String,
    /// Webhook URL for receiving updates (alternative to polling).
    pub webhook_url: Option<String>,
    /// Default chat ID to send messages to.
    pub default_chat_id: Option<String>,
    /// Enable polling mode.
    #[serde(default)]
    pub polling: bool,
    /// Polling interval in seconds.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    5
}

pub struct TelegramClient {
    config: TelegramConfig,
    status: IntegrationStatus,
}

impl TelegramClient {
    pub fn new(config: TelegramConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &TelegramConfig {
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
