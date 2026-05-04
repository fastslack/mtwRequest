//! Registry HTTP client — talks to an mtw marketplace backend.
//!
//! ## API contract
//!
//! | Method | Path                                   | Purpose                          |
//! |--------|----------------------------------------|----------------------------------|
//! | GET    | `/v1/modules?q=...&type=...&author=...`| Search modules                   |
//! | GET    | `/v1/modules/{name}/{version}`         | Get module metadata              |
//! | POST   | `/v1/modules` (multipart)              | Publish a module (auth required) |
//! | GET    | `/v1/modules/{name}/{version}/tarball` | Download the module tarball      |
//!
//! Authentication uses a `Bearer <token>` header with `RegistryConfig::auth_token`.
//!
//! All methods return `RegistryError::NetworkError` for transport failures,
//! `RegistryError::NotFound` for 404s, `RegistryError::AuthRequired` for 401/403,
//! and `RegistryError::Registry` for any other non-2xx response.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

/// Registry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Base URL of the registry API (e.g. `https://registry.mtw.dev`)
    pub registry_url: String,
    /// Bearer token used for authenticated requests
    pub auth_token: Option<String>,
    /// Request timeout in seconds
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_timeout_secs() -> u64 {
    30
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            registry_url: "https://registry.mtw.dev".to_string(),
            auth_token: None,
            timeout_secs: default_timeout_secs(),
        }
    }
}

/// Information about a module in the registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub rating: f32,
}

/// Search filters for the registry
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchFilters {
    pub module_type: Option<String>,
    pub author: Option<String>,
    pub keyword: Option<String>,
}

/// Result of publishing a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub name: String,
    pub version: String,
    pub url: String,
}

/// Errors specific to registry operations
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("network error: {0}")]
    Network(String),
    #[error("registry returned {status}: {message}")]
    Registry { status: u16, message: String },
    #[error("authentication required")]
    AuthRequired,
    #[error("module not found: {0}")]
    NotFound(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

impl From<reqwest::Error> for RegistryError {
    fn from(err: reqwest::Error) -> Self {
        RegistryError::Network(err.to_string())
    }
}

/// Registry API client
pub struct RegistryClient {
    config: RegistryConfig,
    http: Client,
}

impl RegistryClient {
    /// Create a new registry client.
    ///
    /// Inherits the process-wide outbound profile from
    /// [`mtw_net::default_client_builder`] when one is installed, so a
    /// single `[net].default_profile` flag covers registry traffic
    /// alongside everything else.
    pub fn new(config: RegistryConfig) -> Self {
        let http = mtw_net::default_client_builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent(concat!("mtw-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        Self { config, http }
    }

    /// Get the registry configuration
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.config.registry_url.trim_end_matches('/'), path)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.config.auth_token {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    /// Search for modules in the registry
    pub async fn search(
        &self,
        query: &str,
        filters: &SearchFilters,
    ) -> Result<Vec<ModuleInfo>, RegistryError> {
        let mut req = self.http.get(self.endpoint("/v1/modules"));
        let mut q: Vec<(&str, &str)> = vec![("q", query)];
        if let Some(t) = filters.module_type.as_deref() {
            q.push(("type", t));
        }
        if let Some(a) = filters.author.as_deref() {
            q.push(("author", a));
        }
        if let Some(k) = filters.keyword.as_deref() {
            q.push(("keyword", k));
        }
        req = req.query(&q);
        let resp = self.auth(req).send().await?;
        let body = handle_status(resp, "search").await?;
        body.json::<Vec<ModuleInfo>>()
            .await
            .map_err(|e| RegistryError::InvalidResponse(e.to_string()))
    }

    /// Get information about a specific module
    pub async fn get_module(
        &self,
        name: &str,
        version: &str,
    ) -> Result<ModuleInfo, RegistryError> {
        let path = format!("/v1/modules/{name}/{version}");
        let resp = self.auth(self.http.get(self.endpoint(&path))).send().await?;
        let body = handle_status(resp, &format!("{name}@{version}")).await?;
        body.json::<ModuleInfo>()
            .await
            .map_err(|e| RegistryError::InvalidResponse(e.to_string()))
    }

    /// Publish a module to the registry
    pub async fn publish(
        &self,
        manifest: &crate::manifest::RegistryManifest,
        package: Vec<u8>,
    ) -> Result<PublishResult, RegistryError> {
        if self.config.auth_token.is_none() {
            return Err(RegistryError::AuthRequired);
        }
        let manifest_json = serde_json::to_string(manifest)
            .map_err(|e| RegistryError::InvalidResponse(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .text("manifest", manifest_json)
            .part(
                "package",
                reqwest::multipart::Part::bytes(package)
                    .file_name(format!("{}-{}.tar.gz", manifest.name, manifest.version))
                    .mime_str("application/gzip")
                    .map_err(|e| RegistryError::InvalidResponse(e.to_string()))?,
            );

        let resp = self
            .auth(self.http.post(self.endpoint("/v1/modules")))
            .multipart(form)
            .send()
            .await?;
        let body = handle_status(resp, "publish").await?;
        body.json::<PublishResult>()
            .await
            .map_err(|e| RegistryError::InvalidResponse(e.to_string()))
    }

    /// Download a module package
    pub async fn download(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Vec<u8>, RegistryError> {
        let path = format!("/v1/modules/{name}/{version}/tarball");
        let resp = self.auth(self.http.get(self.endpoint(&path))).send().await?;
        let body = handle_status(resp, &format!("{name}@{version}")).await?;
        let bytes = body.bytes().await?;
        Ok(bytes.to_vec())
    }
}

async fn handle_status(
    resp: reqwest::Response,
    context: &str,
) -> Result<reqwest::Response, RegistryError> {
    match resp.status() {
        s if s.is_success() => Ok(resp),
        StatusCode::NOT_FOUND => Err(RegistryError::NotFound(context.to_string())),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(RegistryError::AuthRequired),
        status => {
            let message = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            Err(RegistryError::Registry {
                status: status.as_u16(),
                message,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_points_to_registry_dev() {
        let cfg = RegistryConfig::default();
        assert_eq!(cfg.registry_url, "https://registry.mtw.dev");
        assert!(cfg.auth_token.is_none());
        assert_eq!(cfg.timeout_secs, 30);
    }

    #[test]
    fn endpoint_strips_trailing_slash() {
        let cfg = RegistryConfig {
            registry_url: "https://example.com/".into(),
            ..RegistryConfig::default()
        };
        let client = RegistryClient::new(cfg);
        assert_eq!(client.endpoint("/foo"), "https://example.com/foo");
    }

    #[tokio::test]
    async fn publish_without_token_errors() {
        let client = RegistryClient::new(RegistryConfig::default());
        let manifest = crate::manifest::RegistryManifest {
            name: "x".into(),
            version: "0.1.0".into(),
            module_type: mtw_core::module::ModuleType::Middleware,
            description: String::new(),
            author: String::new(),
            license: String::new(),
            repository: None,
            minimum_core: None,
            permissions: crate::manifest::PermissionSet::default(),
            dependencies: vec![],
            config_schema: None,
        };
        let err = client.publish(&manifest, vec![]).await.unwrap_err();
        matches!(err, RegistryError::AuthRequired);
    }
}
