use mtw_core::module::Permission;
use serde::{Deserialize, Serialize};

/// Configuration for the WASM sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Maximum memory in bytes
    pub max_memory: u64,
    /// Maximum execution time in milliseconds
    pub max_execution_time: u64,
    /// Whether to enable WASI
    pub enable_wasi: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory: 64 * 1024 * 1024, // 64MB
            max_execution_time: 30_000,     // 30 seconds
            enable_wasi: false,
        }
    }
}

/// Sandbox permissions — controls what a WASM module can access
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxPermissions {
    /// Allowed network hosts
    pub allowed_hosts: Vec<String>,
    /// Allowed filesystem paths
    pub allowed_paths: Vec<String>,
    /// Allowed environment variables
    pub allowed_env_vars: Vec<String>,
    /// Whether subprocess spawning is allowed
    pub allow_subprocess: bool,
}

impl SandboxPermissions {
    /// Create permissions from a list of Permission enums
    pub fn from_permissions(permissions: &[Permission]) -> Self {
        let mut sandbox = Self::default();
        for perm in permissions {
            match perm {
                Permission::Network => {
                    sandbox.allowed_hosts.push("*".to_string());
                }
                Permission::FileSystem => {
                    sandbox.allowed_paths.push("*".to_string());
                }
                Permission::Environment => {
                    sandbox.allowed_env_vars.push("*".to_string());
                }
                Permission::Subprocess => {
                    sandbox.allow_subprocess = true;
                }
                Permission::Database => {
                    // Database access implies network access to database hosts
                    sandbox.allowed_hosts.push("*".to_string());
                }
                Permission::Custom(_) => {
                    // Custom permissions are application-specific
                }
            }
        }
        sandbox
    }
}

/// Errors specific to sandbox operations
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("sandbox error: {0}")]
    Sandbox(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("module load error: {0}")]
    LoadError(String),
}

/// WASM sandbox for running untrusted modules
///
/// All methods are currently stubs.
pub struct WasmSandbox {
    config: SandboxConfig,
    permissions: SandboxPermissions,
}

impl WasmSandbox {
    /// Create a new WASM sandbox with the given configuration
    pub fn new(config: SandboxConfig, permissions: SandboxPermissions) -> Self {
        Self {
            config,
            permissions,
        }
    }

    /// Get the sandbox configuration
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Get the sandbox permissions
    pub fn permissions(&self) -> &SandboxPermissions {
        &self.permissions
    }

    /// Load a WASM module into the sandbox
    pub fn load_module(&self, _wasm_bytes: &[u8]) -> Result<(), SandboxError> {
        Err(SandboxError::NotImplemented(
            "WASM module loading is not yet implemented".to_string(),
        ))
    }

    /// Execute a function in the loaded WASM module
    pub fn execute(&self, _function: &str, _args: &[u8]) -> Result<Vec<u8>, SandboxError> {
        Err(SandboxError::NotImplemented(
            "WASM execution is not yet implemented".to_string(),
        ))
    }

    /// Validate that a module's requested permissions are allowed
    pub fn validate_permissions(
        &self,
        _requested: &[Permission],
    ) -> Result<(), SandboxError> {
        Err(SandboxError::NotImplemented(
            "permission validation is not yet implemented".to_string(),
        ))
    }
}
