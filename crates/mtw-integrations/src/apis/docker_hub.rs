//! Docker Hub API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "docker_hub",
    name: "Docker Hub",
    base_url: "https://hub.docker.com/v2",
    docs_url: "https://docs.docker.com/docker-hub/api/latest/",
    oauth2_supported: false,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerHubConfig {
    /// Docker Hub username.
    pub username: String,
    /// Personal access token.
    pub access_token: String,
    /// Default namespace (organization or username).
    pub default_namespace: Option<String>,
}

pub struct DockerHubClient {
    config: DockerHubConfig,
    status: IntegrationStatus,
}

impl DockerHubClient {
    pub fn new(config: DockerHubConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &DockerHubConfig {
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
