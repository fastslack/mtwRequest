//! Firebase integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "firebase",
    name: "Firebase",
    base_url: "https://firebase.googleapis.com",
    docs_url: "https://firebase.google.com/docs/reference/rest",
    oauth2_supported: true,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirebaseConfig {
    /// Project ID.
    pub project_id: String,
    /// Service account JSON key (as a string).
    pub service_account_key: Option<String>,
    /// Web API key (for client-side auth).
    pub api_key: Option<String>,
    /// Firestore database ID (defaults to "(default)").
    #[serde(default = "default_database_id")]
    pub database_id: String,
    /// Storage bucket name.
    pub storage_bucket: Option<String>,
}

fn default_database_id() -> String {
    "(default)".to_string()
}

pub struct FirebaseClient {
    config: FirebaseConfig,
    status: IntegrationStatus,
}

impl FirebaseClient {
    pub fn new(config: FirebaseConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &FirebaseConfig {
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
