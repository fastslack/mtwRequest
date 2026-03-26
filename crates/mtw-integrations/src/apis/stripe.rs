//! Stripe Payments API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "stripe",
    name: "Stripe",
    base_url: "https://api.stripe.com/v1",
    docs_url: "https://docs.stripe.com/api",
    oauth2_supported: true,
};

pub mod scopes {
    pub const READ_WRITE: &str = "read_write";
    pub const READ_ONLY: &str = "read_only";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeConfig {
    /// Secret API key (sk_live_... or sk_test_...).
    pub secret_key: String,
    /// Publishable key (pk_live_... or pk_test_...).
    pub publishable_key: Option<String>,
    /// Webhook signing secret (whsec_...).
    pub webhook_secret: Option<String>,
    /// API version to pin to.
    pub api_version: Option<String>,
}

pub struct StripeClient {
    config: StripeConfig,
    status: IntegrationStatus,
}

impl StripeClient {
    pub fn new(config: StripeConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &StripeConfig {
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
