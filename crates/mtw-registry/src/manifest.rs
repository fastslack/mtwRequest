use mtw_core::module::{ModuleDep, ModuleType, Permission};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Permission flags for a module manifest
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionSet {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub filesystem: bool,
    #[serde(default)]
    pub environment: bool,
    #[serde(default)]
    pub subprocess: bool,
}

impl PermissionSet {
    /// Convert to a list of Permission enums
    pub fn to_permissions(&self) -> Vec<Permission> {
        let mut perms = Vec::new();
        if self.network {
            perms.push(Permission::Network);
        }
        if self.filesystem {
            perms.push(Permission::FileSystem);
        }
        if self.environment {
            perms.push(Permission::Environment);
        }
        if self.subprocess {
            perms.push(Permission::Subprocess);
        }
        perms
    }
}

/// Raw TOML structure for the [module] section
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawModule {
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub module_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    pub repository: Option<String>,
    pub minimum_core: Option<String>,
}

/// Raw TOML structure for [dependencies] section
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawDependency {
    pub version: Option<String>,
    pub optional: Option<bool>,
}

/// Raw TOML manifest file
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawManifest {
    pub module: RawModule,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub dependencies: HashMap<String, toml::Value>,
    pub config: Option<serde_json::Value>,
}

/// Module manifest — parsed from mtw-module.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryManifest {
    pub name: String,
    pub version: String,
    pub module_type: ModuleType,
    pub description: String,
    pub author: String,
    pub license: String,
    pub repository: Option<String>,
    pub minimum_core: Option<String>,
    pub permissions: PermissionSet,
    pub dependencies: Vec<ModuleDep>,
    pub config_schema: Option<serde_json::Value>,
}

/// Errors specific to manifest parsing
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("invalid module type: {0}")]
    InvalidModuleType(String),

    #[error("invalid semver version: {0}")]
    InvalidVersion(String),

    #[error("missing required field: {0}")]
    MissingField(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Parse a module type string into the ModuleType enum
fn parse_module_type(s: &str) -> Result<ModuleType, ManifestError> {
    match s.to_lowercase().as_str() {
        "transport" => Ok(ModuleType::Transport),
        "middleware" => Ok(ModuleType::Middleware),
        "ai_provider" | "aiprovider" => Ok(ModuleType::AIProvider),
        "ai_agent" | "aiagent" => Ok(ModuleType::AIAgent),
        "codec" => Ok(ModuleType::Codec),
        "auth" => Ok(ModuleType::Auth),
        "storage" => Ok(ModuleType::Storage),
        "channel" => Ok(ModuleType::Channel),
        "integration" => Ok(ModuleType::Integration),
        "ui" => Ok(ModuleType::UI),
        other => Err(ManifestError::InvalidModuleType(other.to_string())),
    }
}

/// Validate a version string as semver (basic validation)
fn validate_semver(version: &str) -> Result<(), ManifestError> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(ManifestError::InvalidVersion(version.to_string()));
    }
    for part in &parts {
        if part.parse::<u64>().is_err() {
            return Err(ManifestError::InvalidVersion(version.to_string()));
        }
    }
    Ok(())
}

/// Parse dependencies from the TOML value map
fn parse_dependencies(deps: &HashMap<String, toml::Value>) -> Vec<ModuleDep> {
    deps.iter()
        .map(|(name, value)| {
            match value {
                toml::Value::String(version) => ModuleDep {
                    name: name.clone(),
                    version: version.clone(),
                    optional: false,
                },
                toml::Value::Table(table) => {
                    let version = table
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("*")
                        .to_string();
                    let optional = table
                        .get("optional")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    ModuleDep {
                        name: name.clone(),
                        version,
                        optional,
                    }
                }
                _ => ModuleDep {
                    name: name.clone(),
                    version: "*".to_string(),
                    optional: false,
                },
            }
        })
        .collect()
}

impl RegistryManifest {
    /// Parse a manifest from a TOML string
    pub fn from_toml(toml_str: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(toml_str)?;
        Self::from_raw(raw)
    }

    /// Parse a manifest from a file path
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_toml(&contents)
    }

    /// Convert from raw parsed TOML to validated manifest
    fn from_raw(raw: RawManifest) -> Result<Self, ManifestError> {
        // Validate required fields
        if raw.module.name.is_empty() {
            return Err(ManifestError::MissingField("name".to_string()));
        }
        if raw.module.version.is_empty() {
            return Err(ManifestError::MissingField("version".to_string()));
        }

        // Validate semver
        validate_semver(&raw.module.version)?;

        // Validate module type
        let module_type = parse_module_type(&raw.module.module_type)?;

        // Parse dependencies
        let dependencies = parse_dependencies(&raw.dependencies);

        Ok(Self {
            name: raw.module.name,
            version: raw.module.version,
            module_type,
            description: raw.module.description,
            author: raw.module.author,
            license: raw.module.license,
            repository: raw.module.repository,
            minimum_core: raw.module.minimum_core,
            permissions: raw.permissions,
            dependencies,
            config_schema: raw.config,
        })
    }

    /// Validate the manifest (can be called after parsing for additional checks)
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.name.is_empty() {
            return Err(ManifestError::MissingField("name".to_string()));
        }
        if self.version.is_empty() {
            return Err(ManifestError::MissingField("version".to_string()));
        }
        validate_semver(&self.version)?;
        Ok(())
    }

    /// Convert to the core ModuleManifest type
    pub fn to_module_manifest(&self) -> mtw_core::module::ModuleManifest {
        mtw_core::module::ModuleManifest {
            name: self.name.clone(),
            version: self.version.clone(),
            module_type: self.module_type.clone(),
            description: self.description.clone(),
            author: self.author.clone(),
            license: self.license.clone(),
            repository: self.repository.clone(),
            dependencies: self.dependencies.clone(),
            config_schema: self.config_schema.clone(),
            permissions: self.permissions.to_permissions(),
            minimum_core: self.minimum_core.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MANIFEST: &str = r#"
[module]
name = "mtw-auth-jwt"
version = "1.0.0"
type = "auth"
description = "JWT authentication for mtwRequest"
author = "fastslack"
license = "MIT"
repository = "https://github.com/fastslack/mtw-auth-jwt"
minimum_core = "0.1.0"

[permissions]
network = false
filesystem = false
environment = true

[dependencies]
mtw-core = "0.1"
"#;

    #[test]
    fn test_parse_valid_manifest() {
        let manifest = RegistryManifest::from_toml(SAMPLE_MANIFEST).unwrap();
        assert_eq!(manifest.name, "mtw-auth-jwt");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.module_type, ModuleType::Auth);
        assert_eq!(manifest.description, "JWT authentication for mtwRequest");
        assert_eq!(manifest.author, "fastslack");
        assert_eq!(manifest.license, "MIT");
        assert_eq!(
            manifest.repository,
            Some("https://github.com/fastslack/mtw-auth-jwt".to_string())
        );
        assert_eq!(manifest.minimum_core, Some("0.1.0".to_string()));
    }

    #[test]
    fn test_parse_permissions() {
        let manifest = RegistryManifest::from_toml(SAMPLE_MANIFEST).unwrap();
        assert!(!manifest.permissions.network);
        assert!(!manifest.permissions.filesystem);
        assert!(manifest.permissions.environment);
        assert!(!manifest.permissions.subprocess);

        let perms = manifest.permissions.to_permissions();
        assert_eq!(perms.len(), 1);
        assert_eq!(perms[0], Permission::Environment);
    }

    #[test]
    fn test_parse_dependencies() {
        let manifest = RegistryManifest::from_toml(SAMPLE_MANIFEST).unwrap();
        assert_eq!(manifest.dependencies.len(), 1);
        assert_eq!(manifest.dependencies[0].name, "mtw-core");
        assert_eq!(manifest.dependencies[0].version, "0.1");
        assert!(!manifest.dependencies[0].optional);
    }

    #[test]
    fn test_invalid_module_type() {
        let toml = r#"
[module]
name = "test"
version = "1.0.0"
type = "invalid_type"
"#;
        let result = RegistryManifest::from_toml(toml);
        assert!(result.is_err());
        match result.unwrap_err() {
            ManifestError::InvalidModuleType(t) => assert_eq!(t, "invalid_type"),
            e => panic!("unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_invalid_version() {
        let toml = r#"
[module]
name = "test"
version = "not.a.version"
type = "auth"
"#;
        let result = RegistryManifest::from_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_name() {
        let toml = r#"
[module]
name = ""
version = "1.0.0"
type = "auth"
"#;
        let result = RegistryManifest::from_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_module_manifest() {
        let manifest = RegistryManifest::from_toml(SAMPLE_MANIFEST).unwrap();
        let core_manifest = manifest.to_module_manifest();
        assert_eq!(core_manifest.name, "mtw-auth-jwt");
        assert_eq!(core_manifest.module_type, ModuleType::Auth);
        assert_eq!(core_manifest.permissions.len(), 1);
    }

    #[test]
    fn test_all_permissions_set() {
        let toml = r#"
[module]
name = "test"
version = "1.0.0"
type = "middleware"

[permissions]
network = true
filesystem = true
environment = true
subprocess = true
"#;
        let manifest = RegistryManifest::from_toml(toml).unwrap();
        let perms = manifest.permissions.to_permissions();
        assert_eq!(perms.len(), 4);
    }

    #[test]
    fn test_optional_dependency() {
        let toml = r#"
[module]
name = "test"
version = "1.0.0"
type = "middleware"

[dependencies.mtw-redis]
version = "0.2"
optional = true
"#;
        let manifest = RegistryManifest::from_toml(toml).unwrap();
        assert_eq!(manifest.dependencies.len(), 1);
        assert!(manifest.dependencies[0].optional);
    }

    #[test]
    fn test_validate() {
        let manifest = RegistryManifest::from_toml(SAMPLE_MANIFEST).unwrap();
        assert!(manifest.validate().is_ok());
    }
}
