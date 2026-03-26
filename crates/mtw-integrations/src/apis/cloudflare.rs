//! Cloudflare API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "cloudflare",
    name: "Cloudflare",
    base_url: "https://api.cloudflare.com/client/v4",
    docs_url: "https://developers.cloudflare.com/api/",
    oauth2_supported: false,
};

/// Cloudflare authentication method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudflareAuthMethod {
    /// API token (recommended).
    ApiToken(String),
    /// Global API key + email.
    ApiKey {
        api_key: String,
        email: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudflareConfig {
    /// Authentication method.
    pub auth: CloudflareAuthMethod,
    /// Account ID.
    pub account_id: Option<String>,
    /// Default zone ID.
    pub zone_id: Option<String>,
}

pub struct CloudflareClient {
    config: CloudflareConfig,
    status: IntegrationStatus,
}

impl CloudflareClient {
    pub fn new(config: CloudflareConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &CloudflareConfig {
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
