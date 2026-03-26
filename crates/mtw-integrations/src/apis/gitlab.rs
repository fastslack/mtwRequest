//! GitLab API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "gitlab",
    name: "GitLab",
    base_url: "https://gitlab.com/api/v4",
    docs_url: "https://docs.gitlab.com/ee/api/rest/",
    oauth2_supported: true,
};

pub mod scopes {
    pub const API: &str = "api";
    pub const READ_USER: &str = "read_user";
    pub const READ_API: &str = "read_api";
    pub const READ_REPOSITORY: &str = "read_repository";
    pub const WRITE_REPOSITORY: &str = "write_repository";
    pub const OPENID: &str = "openid";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabConfig {
    /// Private token or OAuth2 token.
    pub token: String,
    /// API base URL (override for self-hosted GitLab).
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default project ID or path.
    pub default_project: Option<String>,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

pub struct GitLabClient {
    config: GitLabConfig,
    status: IntegrationStatus,
}

impl GitLabClient {
    pub fn new(config: GitLabConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &GitLabConfig {
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
