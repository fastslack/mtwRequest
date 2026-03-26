//! Jira API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "jira",
    name: "Jira",
    base_url: "https://api.atlassian.com",
    docs_url: "https://developer.atlassian.com/cloud/jira/platform/rest/v3/",
    oauth2_supported: true,
};

pub mod scopes {
    pub const READ_JIRA_WORK: &str = "read:jira-work";
    pub const WRITE_JIRA_WORK: &str = "write:jira-work";
    pub const READ_JIRA_USER: &str = "read:jira-user";
    pub const MANAGE_JIRA_PROJECT: &str = "manage:jira-project";
    pub const MANAGE_JIRA_CONFIGURATION: &str = "manage:jira-configuration";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JiraConfig {
    /// Atlassian site domain (e.g. "mycompany.atlassian.net").
    pub domain: String,
    /// API token (used with email for basic auth).
    pub api_token: String,
    /// User email (for basic auth).
    pub email: String,
    /// Default project key.
    pub default_project: Option<String>,
}

pub struct JiraClient {
    config: JiraConfig,
    status: IntegrationStatus,
}

impl JiraClient {
    pub fn new(config: JiraConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &JiraConfig {
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
