use mtw_core::error::MtwError;
use mtw_core::module::{ModuleDep, ModuleManifest, ModuleType, Permission};

/// Fluent builder for constructing a ModuleManifest
#[derive(Debug, Clone)]
pub struct ModuleManifestBuilder {
    name: Option<String>,
    version: Option<String>,
    module_type: Option<ModuleType>,
    description: String,
    author: String,
    license: String,
    repository: Option<String>,
    dependencies: Vec<ModuleDep>,
    config_schema: Option<serde_json::Value>,
    permissions: Vec<Permission>,
    minimum_core: Option<String>,
}

impl ModuleManifestBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            name: None,
            version: None,
            module_type: None,
            description: String::new(),
            author: String::new(),
            license: String::new(),
            repository: None,
            dependencies: Vec::new(),
            config_schema: None,
            permissions: Vec::new(),
            minimum_core: None,
        }
    }

    /// Set the module name
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the module version
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the module type
    pub fn module_type(mut self, module_type: ModuleType) -> Self {
        self.module_type = Some(module_type);
        self
    }

    /// Set the description
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the author
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    /// Set the license
    pub fn license(mut self, license: impl Into<String>) -> Self {
        self.license = license.into();
        self
    }

    /// Set the repository URL
    pub fn repository(mut self, repository: impl Into<String>) -> Self {
        self.repository = Some(repository.into());
        self
    }

    /// Add a dependency
    pub fn dependency(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.dependencies.push(ModuleDep {
            name: name.into(),
            version: version.into(),
            optional: false,
        });
        self
    }

    /// Add an optional dependency
    pub fn optional_dependency(
        mut self,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        self.dependencies.push(ModuleDep {
            name: name.into(),
            version: version.into(),
            optional: true,
        });
        self
    }

    /// Set the config schema
    pub fn config_schema(mut self, schema: serde_json::Value) -> Self {
        self.config_schema = Some(schema);
        self
    }

    /// Add a permission
    pub fn permission(mut self, permission: Permission) -> Self {
        self.permissions.push(permission);
        self
    }

    /// Set the minimum core version
    pub fn minimum_core(mut self, version: impl Into<String>) -> Self {
        self.minimum_core = Some(version.into());
        self
    }

    /// Build the manifest, returning an error if required fields are missing
    pub fn build(self) -> Result<ModuleManifest, MtwError> {
        let name = self.name.ok_or_else(|| {
            MtwError::Config("module name is required".to_string())
        })?;
        let version = self.version.ok_or_else(|| {
            MtwError::Config("module version is required".to_string())
        })?;
        let module_type = self.module_type.ok_or_else(|| {
            MtwError::Config("module type is required".to_string())
        })?;

        Ok(ModuleManifest {
            name,
            version,
            module_type,
            description: self.description,
            author: self.author,
            license: self.license,
            repository: self.repository,
            dependencies: self.dependencies,
            config_schema: self.config_schema,
            permissions: self.permissions,
            minimum_core: self.minimum_core,
        })
    }
}

impl Default for ModuleManifestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a manifest using the builder pattern
///
/// # Example
/// ```
/// use mtw_sdk::builder::create_manifest;
/// use mtw_core::module::ModuleType;
///
/// let manifest = create_manifest("my-module", "1.0.0", ModuleType::Middleware).unwrap();
/// assert_eq!(manifest.name, "my-module");
/// ```
pub fn create_manifest(
    name: impl Into<String>,
    version: impl Into<String>,
    module_type: ModuleType,
) -> Result<ModuleManifest, MtwError> {
    ModuleManifestBuilder::new()
        .name(name)
        .version(version)
        .module_type(module_type)
        .build()
}

/// Create a default manifest with common fields pre-filled
pub fn default_manifest(name: impl Into<String>) -> ModuleManifestBuilder {
    ModuleManifestBuilder::new()
        .name(name)
        .version("0.1.0")
        .module_type(ModuleType::Middleware)
        .license("MIT")
        .minimum_core("0.1.0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_basic() {
        let manifest = ModuleManifestBuilder::new()
            .name("test-module")
            .version("1.0.0")
            .module_type(ModuleType::Middleware)
            .description("A test module")
            .author("test-author")
            .license("MIT")
            .build()
            .unwrap();

        assert_eq!(manifest.name, "test-module");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.module_type, ModuleType::Middleware);
        assert_eq!(manifest.description, "A test module");
        assert_eq!(manifest.author, "test-author");
        assert_eq!(manifest.license, "MIT");
    }

    #[test]
    fn test_builder_missing_name() {
        let result = ModuleManifestBuilder::new()
            .version("1.0.0")
            .module_type(ModuleType::Middleware)
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_missing_version() {
        let result = ModuleManifestBuilder::new()
            .name("test")
            .module_type(ModuleType::Middleware)
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_missing_type() {
        let result = ModuleManifestBuilder::new()
            .name("test")
            .version("1.0.0")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_with_dependencies() {
        let manifest = ModuleManifestBuilder::new()
            .name("test")
            .version("1.0.0")
            .module_type(ModuleType::Auth)
            .dependency("mtw-core", "^0.1.0")
            .optional_dependency("mtw-redis", "^1.0.0")
            .build()
            .unwrap();

        assert_eq!(manifest.dependencies.len(), 2);
        assert!(!manifest.dependencies[0].optional);
        assert!(manifest.dependencies[1].optional);
    }

    #[test]
    fn test_builder_with_permissions() {
        let manifest = ModuleManifestBuilder::new()
            .name("test")
            .version("1.0.0")
            .module_type(ModuleType::Integration)
            .permission(Permission::Network)
            .permission(Permission::Environment)
            .build()
            .unwrap();

        assert_eq!(manifest.permissions.len(), 2);
    }

    #[test]
    fn test_builder_with_repository() {
        let manifest = ModuleManifestBuilder::new()
            .name("test")
            .version("1.0.0")
            .module_type(ModuleType::Middleware)
            .repository("https://github.com/test/test")
            .build()
            .unwrap();

        assert_eq!(
            manifest.repository,
            Some("https://github.com/test/test".to_string())
        );
    }

    #[test]
    fn test_create_manifest() {
        let manifest =
            create_manifest("my-module", "1.0.0", ModuleType::Middleware).unwrap();
        assert_eq!(manifest.name, "my-module");
        assert_eq!(manifest.version, "1.0.0");
    }

    #[test]
    fn test_default_manifest() {
        let manifest = default_manifest("my-module")
            .description("My cool module")
            .build()
            .unwrap();

        assert_eq!(manifest.name, "my-module");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.module_type, ModuleType::Middleware);
        assert_eq!(manifest.license, "MIT");
        assert_eq!(manifest.minimum_core, Some("0.1.0".to_string()));
    }
}
