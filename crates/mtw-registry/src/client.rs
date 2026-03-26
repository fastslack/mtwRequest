use serde::{Deserialize, Serialize};

/// Registry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// URL of the registry API
    pub registry_url: String,
    /// Authentication token
    pub auth_token: Option<String>,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            registry_url: "https://registry.mtw.dev".to_string(),
            auth_token: None,
        }
    }
}

/// Information about a module in the registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
    pub rating: f32,
}

/// Search filters for the registry
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchFilters {
    /// Filter by module type
    pub module_type: Option<String>,
    /// Filter by author
    pub author: Option<String>,
    /// Filter by keyword
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
    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("registry error: {0}")]
    Registry(String),

    #[error("authentication required")]
    AuthRequired,

    #[error("module not found: {0}")]
    NotFound(String),
}

/// Registry API client
///
/// All methods are currently stubs and return "not implemented" errors.
pub struct RegistryClient {
    config: RegistryConfig,
}

impl RegistryClient {
    /// Create a new registry client
    pub fn new(config: RegistryConfig) -> Self {
        Self { config }
    }

    /// Get the registry configuration
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    /// Search for modules in the registry
    pub fn search(
        &self,
        _query: &str,
        _filters: &SearchFilters,
    ) -> Result<Vec<ModuleInfo>, RegistryError> {
        Err(RegistryError::NotImplemented(
            "registry search is not yet implemented".to_string(),
        ))
    }

    /// Get information about a specific module
    pub fn get_module(
        &self,
        _name: &str,
        _version: &str,
    ) -> Result<ModuleInfo, RegistryError> {
        Err(RegistryError::NotImplemented(
            "registry get_module is not yet implemented".to_string(),
        ))
    }

    /// Publish a module to the registry
    pub fn publish(
        &self,
        _manifest: &crate::manifest::RegistryManifest,
        _package: &[u8],
    ) -> Result<PublishResult, RegistryError> {
        Err(RegistryError::NotImplemented(
            "registry publish is not yet implemented".to_string(),
        ))
    }

    /// Download a module package
    pub fn download(
        &self,
        _name: &str,
        _version: &str,
    ) -> Result<Vec<u8>, RegistryError> {
        Err(RegistryError::NotImplemented(
            "registry download is not yet implemented".to_string(),
        ))
    }
}
