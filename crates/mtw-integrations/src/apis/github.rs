//! GitHub API integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "github",
    name: "GitHub",
    base_url: "https://api.github.com",
    docs_url: "https://docs.github.com/en/rest",
    oauth2_supported: true,
};

/// GitHub OAuth2 scopes.
pub mod scopes {
    pub const REPO: &str = "repo";
    pub const USER: &str = "user";
    pub const READ_ORG: &str = "read:org";
    pub const GIST: &str = "gist";
    pub const NOTIFICATIONS: &str = "notifications";
    pub const WORKFLOW: &str = "workflow";
    pub const ADMIN_ORG: &str = "admin:org";
    pub const DELETE_REPO: &str = "delete_repo";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    /// Personal access token or OAuth2 token.
    pub token: String,
    /// API base URL (override for GitHub Enterprise).
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default organization to scope requests to.
    pub default_org: Option<String>,
    /// Default repository (owner/repo format).
    pub default_repo: Option<String>,
}

fn default_base_url() -> String {
    INFO.base_url.to_string()
}

pub struct GitHubClient {
    config: GitHubConfig,
    status: IntegrationStatus,
}

impl GitHubClient {
    pub fn new(config: GitHubConfig) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &GitHubConfig {
        &self.config
    }

    pub fn status(&self) -> &IntegrationStatus {
        &self.status
    }

    pub fn info() -> &'static IntegrationInfo {
        &INFO
    }

    /// Stub: verify the token is valid.
    pub async fn connect(&mut self) -> Result<(), String> {
        // TODO: implement actual API call to verify token
        self.status = IntegrationStatus::Connected;
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.status = IntegrationStatus::Disconnected;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_serialization() {
        let config = GitHubConfig {
            token: "ghp_test123".to_string(),
            base_url: "https://api.github.com".to_string(),
            default_org: Some("my-org".to_string()),
            default_repo: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: GitHubConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.token, "ghp_test123");
        assert_eq!(parsed.default_org, Some("my-org".to_string()));
    }

    #[test]
    fn test_client_creation() {
        let client = GitHubClient::new(GitHubConfig {
            token: "test".to_string(),
            base_url: default_base_url(),
            default_org: None,
            default_repo: None,
        });
        assert_eq!(*client.status(), IntegrationStatus::Disconnected);
        assert_eq!(GitHubClient::info().id, "github");
    }
}
