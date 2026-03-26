//! Lightweight testing helpers for module developers.
//!
//! For a full-featured testing harness, see the `mtw-test` crate.

use mtw_core::module::{ModuleContext, ModuleManifest, MtwModule, SharedState};
use mtw_core::error::MtwError;
use std::sync::Arc;

/// Create a ModuleContext suitable for testing
pub struct TestModuleContext;

impl TestModuleContext {
    /// Create a default test context with empty config
    pub fn new() -> ModuleContext {
        ModuleContext {
            config: serde_json::json!({}),
            shared: Arc::new(SharedState::new()),
        }
    }

    /// Create a test context with specific configuration
    pub fn with_config(config: serde_json::Value) -> ModuleContext {
        ModuleContext {
            config,
            shared: Arc::new(SharedState::new()),
        }
    }

    /// Create a test context with shared state pre-populated
    pub fn with_shared(data: Vec<(String, serde_json::Value)>) -> ModuleContext {
        let shared = SharedState::new();
        for (key, value) in data {
            shared.set(key, value);
        }
        ModuleContext {
            config: serde_json::json!({}),
            shared: Arc::new(shared),
        }
    }
}

/// Assert that a module loads successfully.
///
/// # Example
/// ```ignore
/// assert_module_loads(&mut my_module);
/// ```
pub async fn assert_module_loads(module: &mut dyn MtwModule) -> Result<(), MtwError> {
    let ctx = TestModuleContext::new();
    module.on_load(&ctx).await
}

/// Assert that a middleware passes a message through without halting.
///
/// Returns the resulting message after processing, or an error if the
/// middleware halted or returned an error.
pub async fn assert_middleware_passes(
    middleware: &dyn mtw_router::middleware::MtwMiddleware,
    msg: mtw_protocol::MtwMessage,
    ctx: &mtw_router::middleware::MiddlewareContext,
) -> Result<mtw_protocol::MtwMessage, MtwError> {
    match middleware.on_inbound(msg, ctx).await? {
        mtw_router::middleware::MiddlewareAction::Continue(m) => Ok(m),
        mtw_router::middleware::MiddlewareAction::Transform(m) => Ok(m),
        mtw_router::middleware::MiddlewareAction::Redirect { msg, .. } => Ok(msg),
        mtw_router::middleware::MiddlewareAction::Halt => {
            Err(MtwError::Internal("middleware halted the message".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mtw_core::module::{HealthStatus, ModuleType};

    struct DummyModule {
        manifest: ModuleManifest,
    }

    impl DummyModule {
        fn new() -> Self {
            Self {
                manifest: ModuleManifest {
                    name: "dummy".to_string(),
                    version: "0.1.0".to_string(),
                    module_type: ModuleType::Middleware,
                    description: "test".to_string(),
                    author: "test".to_string(),
                    license: "MIT".to_string(),
                    repository: None,
                    dependencies: vec![],
                    config_schema: None,
                    permissions: vec![],
                    minimum_core: None,
                },
            }
        }
    }

    #[async_trait]
    impl MtwModule for DummyModule {
        fn manifest(&self) -> &ModuleManifest {
            &self.manifest
        }
        async fn on_load(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
            Ok(())
        }
        async fn on_start(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
            Ok(())
        }
        async fn on_stop(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
            Ok(())
        }
    }

    #[test]
    fn test_create_test_context() {
        let ctx = TestModuleContext::new();
        assert_eq!(ctx.config, serde_json::json!({}));
    }

    #[test]
    fn test_create_context_with_config() {
        let ctx = TestModuleContext::with_config(serde_json::json!({"key": "value"}));
        assert_eq!(ctx.config["key"], "value");
    }

    #[test]
    fn test_create_context_with_shared() {
        let ctx = TestModuleContext::with_shared(vec![
            ("key".to_string(), serde_json::json!("value")),
        ]);
        assert_eq!(ctx.shared.get("key"), Some(serde_json::json!("value")));
    }

    #[tokio::test]
    async fn test_assert_module_loads() {
        let mut module = DummyModule::new();
        assert!(assert_module_loads(&mut module).await.is_ok());
    }
}
