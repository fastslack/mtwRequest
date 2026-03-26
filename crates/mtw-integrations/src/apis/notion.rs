//! Notion API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "notion",
    name: "Notion",
    base_url: "https://api.notion.com/v1",
    docs_url: "https://developers.notion.com/reference",
    oauth2_supported: true,
};

pub mod scopes {
    pub const READ_CONTENT: &str = "read_content";
    pub const UPDATE_CONTENT: &str = "update_content";
    pub const INSERT_CONTENT: &str = "insert_content";
    pub const READ_USER: &str = "read_user";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionConfig {
    /// Internal integration token or OAuth2 token.
    pub token: String,
    /// Notion API version header.
    #[serde(default = "default_api_version")]
    pub api_version: String,
    /// Default database ID for queries.
    pub default_database_id: Option<String>,
}

fn default_api_version() -> String {
    "2022-06-28".to_string()
}

pub struct NotionClient {
    config: NotionConfig,
    status: IntegrationStatus,
}

impl NotionClient {
    pub fn new(config: NotionConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &NotionConfig {
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
