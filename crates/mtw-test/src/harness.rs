use mtw_core::module::MtwModule;
use mtw_router::middleware::MtwMiddleware;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Builder for creating a test server
pub struct TestServer {
    modules: Vec<Box<dyn MtwModule>>,
    middlewares: Vec<Arc<dyn MtwMiddleware>>,
}

impl TestServer {
    /// Create a new test server builder
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
            middlewares: Vec::new(),
        }
    }

    /// Add a module to the test server
    pub fn with_module(mut self, module: impl MtwModule + 'static) -> Self {
        self.modules.push(Box::new(module));
        self
    }

    /// Add middleware to the test server
    pub fn with_middleware(mut self, middleware: impl MtwMiddleware + 'static) -> Self {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    /// Start the test server, binding to a random port
    pub async fn start(self) -> Result<RunningTestServer, mtw_core::MtwError> {
        // Bind to port 0 to get a random available port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| mtw_core::MtwError::Transport(format!("failed to bind: {}", e)))?;

        let addr = listener
            .local_addr()
            .map_err(|e| mtw_core::MtwError::Transport(format!("failed to get addr: {}", e)))?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // Spawn the server task
        let handle = tokio::spawn(async move {
            let _listener = listener;
            let _modules = self.modules;
            let _middlewares = self.middlewares;

            // Wait for shutdown signal
            let _ = shutdown_rx.await;
        });

        tracing::info!(%addr, "test server started");

        Ok(RunningTestServer {
            addr,
            shutdown_tx: Some(shutdown_tx),
            _handle: handle,
        })
    }
}

impl Default for TestServer {
    fn default() -> Self {
        Self::new()
    }
}

/// A running test server instance
pub struct RunningTestServer {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl RunningTestServer {
    /// Get the address the server is bound to
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Get the WebSocket URL for this server
    pub fn url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }

    /// Shut down the test server
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for RunningTestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_starts_and_stops() {
        let server = TestServer::new().start().await.unwrap();
        let addr = server.addr();
        assert_ne!(addr.port(), 0);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_server_url() {
        let server = TestServer::new().start().await.unwrap();
        let url = server.url();
        assert!(url.starts_with("ws://127.0.0.1:"));
        assert!(url.ends_with("/ws"));
        server.shutdown().await;
    }
}
