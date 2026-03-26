use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::MtwError;

/// Module type classification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleType {
    Transport,
    Middleware,
    AIProvider,
    AIAgent,
    Codec,
    Auth,
    Storage,
    Channel,
    Integration,
    UI,
}

/// Permissions a module can request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Network,
    FileSystem,
    Environment,
    Subprocess,
    Database,
    Custom(String),
}

/// Module dependency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDep {
    pub name: String,
    pub version: String,
    pub optional: bool,
}

/// Module manifest — loaded from mtw-module.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub module_type: ModuleType,
    pub description: String,
    pub author: String,
    pub license: String,
    pub repository: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<ModuleDep>,
    pub config_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    pub minimum_core: Option<String>,
}

/// Health status reported by modules
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}

/// Context passed to modules during lifecycle events
pub struct ModuleContext {
    /// Module configuration values
    pub config: serde_json::Value,
    /// Shared state accessible to all modules
    pub shared: Arc<SharedState>,
}

/// Shared state between modules
pub struct SharedState {
    data: dashmap::DashMap<String, serde_json::Value>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            data: dashmap::DashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.data.get(key).map(|v| v.value().clone())
    }

    pub fn set(&self, key: impl Into<String>, value: serde_json::Value) {
        self.data.insert(key.into(), value);
    }

    pub fn remove(&self, key: &str) -> Option<serde_json::Value> {
        self.data.remove(key).map(|(_, v)| v)
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// The core module trait — everything implements this
#[async_trait]
pub trait MtwModule: Send + Sync {
    /// Module manifest
    fn manifest(&self) -> &ModuleManifest;

    /// Called when the module is loaded into the runtime
    async fn on_load(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Called when the server starts accepting connections
    async fn on_start(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Called when the server is shutting down
    async fn on_stop(&mut self, ctx: &ModuleContext) -> Result<(), MtwError>;

    /// Health check — override for custom health reporting
    async fn health(&self) -> HealthStatus {
        HealthStatus::Healthy
    }
}

/// Module registry — holds all loaded modules
pub struct ModuleRegistry {
    modules: HashMap<String, Box<dyn MtwModule>>,
    load_order: Vec<String>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            load_order: Vec::new(),
        }
    }

    /// Register a module
    pub fn register(&mut self, module: Box<dyn MtwModule>) -> Result<(), MtwError> {
        let name = module.manifest().name.clone();
        if self.modules.contains_key(&name) {
            return Err(MtwError::Module {
                module: name,
                message: "module already registered".into(),
            });
        }
        tracing::info!(module = %name, "registered module");
        self.load_order.push(name.clone());
        self.modules.insert(name, module);
        Ok(())
    }

    /// Load all registered modules
    pub async fn load_all(&mut self, ctx: &ModuleContext) -> Result<(), MtwError> {
        for name in &self.load_order.clone() {
            if let Some(module) = self.modules.get_mut(name) {
                tracing::info!(module = %name, "loading module");
                module.on_load(ctx).await.map_err(|e| {
                    MtwError::module(name, format!("failed to load: {}", e))
                })?;
            }
        }
        Ok(())
    }

    /// Start all loaded modules
    pub async fn start_all(&mut self, ctx: &ModuleContext) -> Result<(), MtwError> {
        for name in &self.load_order.clone() {
            if let Some(module) = self.modules.get_mut(name) {
                tracing::info!(module = %name, "starting module");
                module.on_start(ctx).await.map_err(|e| {
                    MtwError::module(name, format!("failed to start: {}", e))
                })?;
            }
        }
        Ok(())
    }

    /// Stop all modules in reverse order
    pub async fn stop_all(&mut self, ctx: &ModuleContext) -> Result<(), MtwError> {
        for name in self.load_order.iter().rev() {
            if let Some(module) = self.modules.get_mut(name) {
                tracing::info!(module = %name, "stopping module");
                if let Err(e) = module.on_stop(ctx).await {
                    tracing::error!(module = %name, error = %e, "error stopping module");
                }
            }
        }
        Ok(())
    }

    /// Get a module by name
    pub fn get(&self, name: &str) -> Option<&dyn MtwModule> {
        self.modules.get(name).map(|m| m.as_ref())
    }

    /// List all registered module names
    pub fn list(&self) -> &[String] {
        &self.load_order
    }

    /// Check health of all modules
    pub async fn health_check(&self) -> HashMap<String, HealthStatus> {
        let mut results = HashMap::new();
        for (name, module) in &self.modules {
            results.insert(name.clone(), module.health().await);
        }
        results
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestModule {
        manifest: ModuleManifest,
        loaded: bool,
        started: bool,
    }

    impl TestModule {
        fn new(name: &str) -> Self {
            Self {
                manifest: ModuleManifest {
                    name: name.to_string(),
                    version: "0.1.0".to_string(),
                    module_type: ModuleType::Middleware,
                    description: "test module".to_string(),
                    author: "test".to_string(),
                    license: "MIT".to_string(),
                    repository: None,
                    dependencies: vec![],
                    config_schema: None,
                    permissions: vec![],
                    minimum_core: None,
                },
                loaded: false,
                started: false,
            }
        }
    }

    #[async_trait]
    impl MtwModule for TestModule {
        fn manifest(&self) -> &ModuleManifest {
            &self.manifest
        }

        async fn on_load(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
            self.loaded = true;
            Ok(())
        }

        async fn on_start(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
            self.started = true;
            Ok(())
        }

        async fn on_stop(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
            self.started = false;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_module_lifecycle() {
        let mut registry = ModuleRegistry::new();
        registry
            .register(Box::new(TestModule::new("test-mod")))
            .unwrap();

        let ctx = ModuleContext {
            config: serde_json::json!({}),
            shared: Arc::new(SharedState::new()),
        };

        registry.load_all(&ctx).await.unwrap();
        registry.start_all(&ctx).await.unwrap();
        registry.stop_all(&ctx).await.unwrap();

        assert_eq!(registry.list(), &["test-mod"]);
    }

    #[test]
    fn test_duplicate_registration() {
        let mut registry = ModuleRegistry::new();
        registry
            .register(Box::new(TestModule::new("dup")))
            .unwrap();
        let result = registry.register(Box::new(TestModule::new("dup")));
        assert!(result.is_err());
    }

    #[test]
    fn test_shared_state() {
        let state = SharedState::new();
        state.set("key", serde_json::json!("value"));
        assert_eq!(state.get("key"), Some(serde_json::json!("value")));
        state.remove("key");
        assert_eq!(state.get("key"), None);
    }
}
