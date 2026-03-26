//! Google Cloud Storage integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "gcs",
    name: "Google Cloud Storage",
    base_url: "https://storage.googleapis.com",
    docs_url: "https://cloud.google.com/storage/docs/json_api",
    oauth2_supported: true,
};

pub mod scopes {
    pub const READ_ONLY: &str = "https://www.googleapis.com/auth/devstorage.read_only";
    pub const READ_WRITE: &str = "https://www.googleapis.com/auth/devstorage.read_write";
    pub const FULL_CONTROL: &str = "https://www.googleapis.com/auth/devstorage.full_control";
}

/// Authentication method for Google Cloud.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcsAuthMethod {
    /// Service account JSON key file path.
    ServiceAccountKey(String),
    /// OAuth2 access token.
    AccessToken(String),
    /// Use application default credentials.
    ApplicationDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcsConfig {
    /// Project ID.
    pub project_id: String,
    /// Authentication method.
    pub auth: GcsAuthMethod,
    /// Default bucket name.
    pub bucket: Option<String>,
}

pub struct GcsClient {
    config: GcsConfig,
    status: IntegrationStatus,
}

impl GcsClient {
    pub fn new(config: GcsConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &GcsConfig {
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
