//! SendGrid Email API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "sendgrid",
    name: "SendGrid",
    base_url: "https://api.sendgrid.com/v3",
    docs_url: "https://docs.sendgrid.com/api-reference",
    oauth2_supported: false,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendGridConfig {
    /// SendGrid API key.
    pub api_key: String,
    /// Default sender email address.
    pub from_email: String,
    /// Default sender name.
    pub from_name: Option<String>,
    /// Sandbox mode for testing.
    #[serde(default)]
    pub sandbox_mode: bool,
}

pub struct SendGridClient {
    config: SendGridConfig,
    status: IntegrationStatus,
}

impl SendGridClient {
    pub fn new(config: SendGridConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &SendGridConfig {
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
