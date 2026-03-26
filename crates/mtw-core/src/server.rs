use std::sync::Arc;

use crate::config::MtwConfig;
use crate::error::MtwError;
use crate::hooks::HookRegistry;
use crate::module::{ModuleContext, ModuleRegistry, SharedState};

/// The main mtwRequest server
pub struct MtwServer {
    config: MtwConfig,
    modules: ModuleRegistry,
    hooks: Arc<HookRegistry>,
    shared: Arc<SharedState>,
}

impl MtwServer {
    /// Create a new server with the given configuration
    pub fn new(config: MtwConfig) -> Self {
        Self {
            config,
            modules: ModuleRegistry::new(),
            hooks: Arc::new(HookRegistry::new()),
            shared: Arc::new(SharedState::new()),
        }
    }

    /// Create a server from a config file
    pub fn from_config_file(path: &str) -> Result<Self, MtwError> {
        let config = MtwConfig::from_file(path)?;
        Ok(Self::new(config))
    }

    /// Register a module with the server
    pub fn module(mut self, module: Box<dyn crate::module::MtwModule>) -> Result<Self, MtwError> {
        self.modules.register(module)?;
        Ok(self)
    }

    /// Get a reference to the hook registry
    pub fn hooks(&self) -> &Arc<HookRegistry> {
        &self.hooks
    }

    /// Get a reference to the shared state
    pub fn shared(&self) -> &Arc<SharedState> {
        &self.shared
    }

    /// Get a reference to the config
    pub fn config(&self) -> &MtwConfig {
        &self.config
    }

    /// Get the module context
    fn module_context(&self) -> ModuleContext {
        ModuleContext {
            config: serde_json::json!({}),
            shared: self.shared.clone(),
        }
    }

    /// Start the server
    pub async fn start(&mut self) -> Result<(), MtwError> {
        tracing::info!(
            host = %self.config.server.host,
            port = %self.config.server.port,
            "starting mtwRequest server"
        );

        // Load all modules
        let ctx = self.module_context();
        self.modules.load_all(&ctx).await?;

        // Start all modules
        self.modules.start_all(&ctx).await?;

        tracing::info!("mtwRequest server started");
        Ok(())
    }

    /// Gracefully shut down the server
    pub async fn shutdown(&mut self) -> Result<(), MtwError> {
        tracing::info!("shutting down mtwRequest server");

        let ctx = self.module_context();
        self.modules.stop_all(&ctx).await?;

        tracing::info!("mtwRequest server stopped");
        Ok(())
    }

    /// Run the server until a shutdown signal is received
    pub async fn run(&mut self) -> Result<(), MtwError> {
        self.start().await?;

        // Wait for shutdown signal (Ctrl+C)
        tokio::signal::ctrl_c()
            .await
            .map_err(|e| MtwError::Internal(format!("failed to listen for shutdown: {}", e)))?;

        tracing::info!("received shutdown signal");
        self.shutdown().await?;

        Ok(())
    }
}

/// Builder for constructing an MtwServer with a fluent API
pub struct MtwServerBuilder {
    config: MtwConfig,
    modules: Vec<Box<dyn crate::module::MtwModule>>,
}

impl MtwServerBuilder {
    pub fn new() -> Self {
        Self {
            config: MtwConfig::default_config(),
            modules: Vec::new(),
        }
    }

    /// Load config from a file
    pub fn config_file(mut self, path: &str) -> Result<Self, MtwError> {
        self.config = MtwConfig::from_file(path)?;
        Ok(self)
    }

    /// Set config directly
    pub fn config(mut self, config: MtwConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the server port
    pub fn port(mut self, port: u16) -> Self {
        self.config.server.port = port;
        self
    }

    /// Set the server host
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.config.server.host = host.into();
        self
    }

    /// Add a module
    pub fn module(mut self, module: Box<dyn crate::module::MtwModule>) -> Self {
        self.modules.push(module);
        self
    }

    /// Build the server
    pub fn build(self) -> Result<MtwServer, MtwError> {
        let mut server = MtwServer::new(self.config);
        for module in self.modules {
            server = server.module(module)?;
        }
        Ok(server)
    }
}

impl Default for MtwServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        let server = MtwServerBuilder::new()
            .port(3000)
            .host("127.0.0.1")
            .build()
            .unwrap();

        assert_eq!(server.config().server.port, 3000);
        assert_eq!(server.config().server.host, "127.0.0.1");
    }

    #[tokio::test]
    async fn test_server_start_stop() {
        let mut server = MtwServerBuilder::new().build().unwrap();

        server.start().await.unwrap();
        server.shutdown().await.unwrap();
    }
}
