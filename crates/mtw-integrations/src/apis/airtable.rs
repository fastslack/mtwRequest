//! Airtable API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "airtable",
    name: "Airtable",
    base_url: "https://api.airtable.com/v0",
    docs_url: "https://airtable.com/developers/web/api/introduction",
    oauth2_supported: true,
};

pub mod scopes {
    pub const DATA_RECORDS_READ: &str = "data.records:read";
    pub const DATA_RECORDS_WRITE: &str = "data.records:write";
    pub const SCHEMA_BASES_READ: &str = "schema.bases:read";
    pub const SCHEMA_BASES_WRITE: &str = "schema.bases:write";
    pub const WEBHOOK_MANAGE: &str = "webhook:manage";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirtableConfig {
    /// Personal access token or OAuth2 token.
    pub token: String,
    /// Default base ID.
    pub base_id: Option<String>,
}

pub struct AirtableClient {
    config: AirtableConfig,
    status: IntegrationStatus,
}

impl AirtableClient {
    pub fn new(config: AirtableConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &AirtableConfig {
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
