//! # mtw-bridge
//!
//! Bridge for delegating operations to external services.
//! Communicates via Unix domain socket (fast, ~0.05ms round-trip) or TCP.
//! Completely project-agnostic — the bridge just forwards tool calls.
//!
//! ## Config
//!
//! ```toml
//! [store.bridge]
//! socket = "/tmp/my-app.sock"
//! ```
//!
//! Or for TCP:
//! ```toml
//! [store.bridge]
//! address = "127.0.0.1:3087"
//! ```

pub mod protocol;
pub mod server;
pub mod unix;

pub use server::{BridgeServer, BridgeToolHandler};

use async_trait::async_trait;
use mtw_core::MtwError;

/// Bridge trait — delegates operations to an external process
#[async_trait]
pub trait MtwBridge: Send + Sync {
    /// Call a tool by name with JSON arguments
    async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, MtwError>;

    /// Check if the bridge target is reachable
    async fn health(&self) -> Result<bool, MtwError>;
}

/// Bridge configuration
///
/// Minimal:
/// ```toml
/// [store.bridge]
/// socket = "/tmp/my-app.sock"
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct BridgeConfig {
    /// Unix domain socket path (preferred — fastest)
    #[serde(default)]
    pub socket: Option<String>,

    /// TCP address fallback (for cross-machine or Docker)
    #[serde(default)]
    pub address: Option<String>,

    /// Timeout per call in milliseconds (default: 30000)
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30000
}

impl BridgeConfig {
    /// Returns true if any bridge target is configured
    pub fn is_configured(&self) -> bool {
        self.socket.is_some() || self.address.is_some()
    }
}

/// Create a bridge from config
pub async fn from_config(config: &BridgeConfig) -> Result<Box<dyn MtwBridge>, MtwError> {
    if let Some(socket_path) = &config.socket {
        let bridge = unix::UnixBridge::connect(socket_path, config.timeout_ms).await?;
        return Ok(Box::new(bridge));
    }

    if let Some(_address) = &config.address {
        return Err(MtwError::Config(
            "TCP bridge not yet implemented — use socket for now".into(),
        ));
    }

    Err(MtwError::Config(
        "bridge: either 'socket' or 'address' must be set".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_config_parse() {
        let toml = r#"socket = "/tmp/my-app.sock""#;
        let config: BridgeConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.socket.unwrap(), "/tmp/my-app.sock");
        assert!(config.address.is_none());
        assert_eq!(config.timeout_ms, 30000);
    }

    #[test]
    fn test_bridge_config_is_configured() {
        let empty = BridgeConfig::default();
        assert!(!empty.is_configured());

        let with_socket = BridgeConfig {
            socket: Some("/tmp/test.sock".into()),
            ..Default::default()
        };
        assert!(with_socket.is_configured());
    }
}
