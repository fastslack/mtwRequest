//! Twilio SMS API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "twilio",
    name: "Twilio",
    base_url: "https://api.twilio.com/2010-04-01",
    docs_url: "https://www.twilio.com/docs/sms/api",
    oauth2_supported: false,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwilioConfig {
    /// Account SID.
    pub account_sid: String,
    /// Auth token.
    pub auth_token: String,
    /// Default "from" phone number (E.164 format).
    pub from_number: String,
    /// Messaging service SID (alternative to from_number).
    pub messaging_service_sid: Option<String>,
}

pub struct TwilioClient {
    config: TwilioConfig,
    status: IntegrationStatus,
}

impl TwilioClient {
    pub fn new(config: TwilioConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &TwilioConfig {
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
