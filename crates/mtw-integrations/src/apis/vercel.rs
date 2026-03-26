//! Vercel API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "vercel",
    name: "Vercel",
    base_url: "https://api.vercel.com",
    docs_url: "https://vercel.com/docs/rest-api",
    oauth2_supported: true,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VercelConfig {
    /// API token.
    pub token: String,
    /// Team ID (for team-scoped operations).
    pub team_id: Option<String>,
    /// Default project name or ID.
    pub default_project: Option<String>,
}

pub struct VercelClient {
    config: VercelConfig,
    status: IntegrationStatus,
}

impl VercelClient {
    pub fn new(config: VercelConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &VercelConfig {
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
