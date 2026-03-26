//! Supabase integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "supabase",
    name: "Supabase",
    base_url: "https://api.supabase.com",
    docs_url: "https://supabase.com/docs/guides/api",
    oauth2_supported: false,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupabaseConfig {
    /// Project URL (e.g. "https://<project>.supabase.co").
    pub project_url: String,
    /// Anon/public key.
    pub anon_key: String,
    /// Service role key (for server-side operations).
    pub service_role_key: Option<String>,
    /// JWT secret for custom JWT verification.
    pub jwt_secret: Option<String>,
}

pub struct SupabaseClient {
    config: SupabaseConfig,
    status: IntegrationStatus,
}

impl SupabaseClient {
    pub fn new(config: SupabaseConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &SupabaseConfig {
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
