//! Linear API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "linear",
    name: "Linear",
    base_url: "https://api.linear.app",
    docs_url: "https://developers.linear.app/docs",
    oauth2_supported: true,
};

pub mod scopes {
    pub const READ: &str = "read";
    pub const WRITE: &str = "write";
    pub const ISSUES_CREATE: &str = "issues:create";
    pub const COMMENTS_CREATE: &str = "comments:create";
    pub const ADMIN: &str = "admin";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearConfig {
    /// Personal API key or OAuth2 token.
    pub api_key: String,
    /// Default team ID.
    pub default_team_id: Option<String>,
}

pub struct LinearClient {
    config: LinearConfig,
    status: IntegrationStatus,
}

impl LinearClient {
    pub fn new(config: LinearConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &LinearConfig {
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
