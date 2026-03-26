//! PayPal API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "paypal",
    name: "PayPal",
    base_url: "https://api-m.paypal.com",
    docs_url: "https://developer.paypal.com/docs/api/overview/",
    oauth2_supported: true,
};

/// PayPal environments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayPalEnvironment {
    Sandbox,
    Live,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayPalConfig {
    /// Client ID.
    pub client_id: String,
    /// Client secret.
    pub client_secret: String,
    /// Environment (sandbox or live).
    #[serde(default = "default_environment")]
    pub environment: PayPalEnvironment,
    /// Webhook ID for event notifications.
    pub webhook_id: Option<String>,
}

fn default_environment() -> PayPalEnvironment {
    PayPalEnvironment::Sandbox
}

impl PayPalConfig {
    pub fn base_url(&self) -> &str {
        match self.environment {
            PayPalEnvironment::Sandbox => "https://api-m.sandbox.paypal.com",
            PayPalEnvironment::Live => "https://api-m.paypal.com",
        }
    }
}

pub struct PayPalClient {
    config: PayPalConfig,
    status: IntegrationStatus,
}

impl PayPalClient {
    pub fn new(config: PayPalConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &PayPalConfig {
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
