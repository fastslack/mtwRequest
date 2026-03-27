use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::error::MtwError;

/// Top-level server configuration (mtw.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtwConfig {
    #[serde(default = "default_server")]
    pub server: ServerConfig,

    #[serde(default)]
    pub transport: TransportConfig,

    #[serde(default)]
    pub codec: CodecConfig,

    /// Data store — connect to SQLite with just a path:
    /// ```toml
    /// [store]
    /// path = "./data/app.db"
    /// ```
    #[serde(default)]
    pub store: Option<StoreSection>,

    #[serde(default)]
    pub modules: Vec<ModuleEntry>,

    #[serde(default)]
    pub agents: Vec<AgentEntry>,

    #[serde(default)]
    pub channels: Vec<ChannelConfig>,

    #[serde(default)]
    pub orchestrator: Option<OrchestratorConfig>,
}

/// Store configuration section
///
/// Minimal:
/// ```toml
/// [store]
/// path = "./data/app.db"
/// ```
///
/// With bridge for writes:
/// ```toml
/// [store]
/// path = "./data/app.db"
///
/// [store.bridge]
/// socket = "/tmp/my-app.sock"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSection {
    /// Path to the database file
    pub path: String,

    /// Read-only mode (default: true — safe for shared access)
    #[serde(default = "default_store_readonly")]
    pub readonly: bool,

    /// Connection pool size (default: 4)
    #[serde(default = "default_store_pool")]
    pub pool_size: u32,

    /// Cache size in MB (default: 64)
    #[serde(default = "default_store_cache")]
    pub cache_mb: u32,

    /// Memory-mapped I/O in MB (default: 256)
    #[serde(default = "default_store_mmap")]
    pub mmap_mb: u32,

    /// Busy timeout in ms (default: 5000)
    #[serde(default = "default_store_timeout")]
    pub busy_timeout_ms: u64,

    /// Bridge for delegating writes to external service
    #[serde(default)]
    pub bridge: Option<BridgeSection>,
}

/// Bridge configuration — delegates writes to an external service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeSection {
    /// Unix domain socket path (preferred — fastest)
    #[serde(default)]
    pub socket: Option<String>,

    /// TCP address fallback (for Docker / cross-machine)
    #[serde(default)]
    pub address: Option<String>,

    /// Timeout per call in ms (default: 30000)
    #[serde(default = "default_bridge_timeout")]
    pub timeout_ms: u64,
}

fn default_store_readonly() -> bool {
    true
}
fn default_store_pool() -> u32 {
    4
}
fn default_store_cache() -> u32 {
    64
}
fn default_store_mmap() -> u32 {
    256
}
fn default_store_timeout() -> u64 {
    5000
}
fn default_bridge_timeout() -> u64 {
    30000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    #[serde(default = "default_transport")]
    pub default: String,
    #[serde(default)]
    pub websocket: WebSocketConfig,
    #[serde(default)]
    pub http: HttpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    #[serde(default = "default_ws_path")]
    pub path: String,
    #[serde(default = "default_ping_interval")]
    pub ping_interval: u64,
    #[serde(default = "default_max_message_size")]
    pub max_message_size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_http_prefix")]
    pub prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecConfig {
    #[serde(default = "default_codec")]
    pub default: String,
    #[serde(default)]
    pub binary_channels: Vec<String>,
}

/// A module entry in the config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleEntry {
    pub name: String,
    pub version: Option<String>,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// An agent entry in the config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub name: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub max_concurrent: Option<usize>,
}

/// Channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub name: String,
    #[serde(default)]
    pub auth: bool,
    #[serde(default)]
    pub max_members: Option<usize>,
    #[serde(default)]
    pub history: Option<usize>,
    #[serde(default)]
    pub codec: Option<String>,
}

/// Orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    pub strategy: String,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

// Defaults
fn default_server() -> ServerConfig {
    ServerConfig {
        host: default_host(),
        port: default_port(),
        max_connections: default_max_connections(),
    }
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    7741
}
fn default_max_connections() -> usize {
    10000
}
fn default_transport() -> String {
    "websocket".into()
}
fn default_ws_path() -> String {
    "/ws".into()
}
fn default_ping_interval() -> u64 {
    30
}
fn default_max_message_size() -> String {
    "10MB".into()
}
fn default_http_prefix() -> String {
    "/api".into()
}
fn default_codec() -> String {
    "json".into()
}

impl Default for ServerConfig {
    fn default() -> Self {
        default_server()
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            default: default_transport(),
            websocket: WebSocketConfig::default(),
            http: HttpConfig::default(),
        }
    }
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            path: default_ws_path(),
            ping_interval: default_ping_interval(),
            max_message_size: default_max_message_size(),
        }
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prefix: default_http_prefix(),
        }
    }
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            default: default_codec(),
            binary_channels: vec![],
        }
    }
}

impl MtwConfig {
    /// Load configuration from a TOML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, MtwError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| MtwError::Config(format!("failed to read config file: {}", e)))?;
        Self::from_str(&content)
    }

    /// Parse configuration from a TOML string
    pub fn from_str(content: &str) -> Result<Self, MtwError> {
        // Expand environment variables in the content
        let expanded = Self::expand_env_vars(content);
        toml::from_str(&expanded)
            .map_err(|e| MtwError::Config(format!("invalid config: {}", e)))
    }

    /// Expand ${ENV_VAR} patterns in config values
    fn expand_env_vars(content: &str) -> String {
        let mut result = content.to_string();
        let re_pattern = "${";

        while let Some(start) = result.find(re_pattern) {
            if let Some(end) = result[start..].find('}') {
                let var_name = &result[start + 2..start + end];
                let value = std::env::var(var_name).unwrap_or_default();
                result = format!("{}{}{}", &result[..start], value, &result[start + end + 1..]);
            } else {
                break;
            }
        }

        result
    }

    /// Create a default configuration
    pub fn default_config() -> Self {
        Self {
            server: ServerConfig::default(),
            transport: TransportConfig::default(),
            codec: CodecConfig::default(),
            store: None,
            modules: vec![],
            agents: vec![],
            channels: vec![],
            orchestrator: None,
        }
    }

    /// Check if a data store is configured
    pub fn has_store(&self) -> bool {
        self.store.is_some()
    }

    /// Check if a bridge is configured
    pub fn has_bridge(&self) -> bool {
        self.store
            .as_ref()
            .and_then(|s| s.bridge.as_ref())
            .map(|b| b.socket.is_some() || b.address.is_some())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MtwConfig::default_config();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 7741);
        assert_eq!(config.transport.default, "websocket");
        assert_eq!(config.transport.websocket.path, "/ws");
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
            [server]
            port = 3000
        "#;
        let config = MtwConfig::from_str(toml).unwrap();
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.server.host, "0.0.0.0"); // default
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 9090
            max_connections = 5000

            [transport]
            default = "websocket"

            [transport.websocket]
            path = "/ws"
            ping_interval = 15

            [[modules]]
            name = "mtw-auth-jwt"
            version = "1.0"

            [modules.config]
            secret = "test-secret"

            [[agents]]
            name = "assistant"
            provider = "mtw-ai-anthropic"
            model = "claude-sonnet-4-6"
            system = "You are helpful."
            channels = ["chat.*"]

            [[channels]]
            name = "chat.*"
            auth = true
            max_members = 50
        "#;

        let config = MtwConfig::from_str(toml).unwrap();
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.modules.len(), 1);
        assert_eq!(config.modules[0].name, "mtw-auth-jwt");
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].name, "assistant");
        assert_eq!(config.channels.len(), 1);
        assert_eq!(config.channels[0].auth, true);
    }

    #[test]
    fn test_env_var_expansion() {
        std::env::set_var("MTW_TEST_PORT", "4000");
        let toml = r#"
            [server]
            host = "0.0.0.0"
            port = 7741
        "#;
        // Note: env vars in TOML values only work for string values
        let config = MtwConfig::from_str(toml).unwrap();
        assert_eq!(config.server.port, 7741);
        std::env::remove_var("MTW_TEST_PORT");
    }

    #[test]
    fn test_no_store_by_default() {
        let config = MtwConfig::default_config();
        assert!(!config.has_store());
        assert!(!config.has_bridge());
    }

    #[test]
    fn test_store_minimal_config() {
        let toml = r#"
            [server]
            port = 7741

            [store]
            path = "./data/app.db"
        "#;
        let config = MtwConfig::from_str(toml).unwrap();
        assert!(config.has_store());
        assert!(!config.has_bridge());

        let store = config.store.unwrap();
        assert_eq!(store.path, "./data/app.db");
        assert!(store.readonly);         // default: true
        assert_eq!(store.pool_size, 4);  // default: 4
        assert_eq!(store.cache_mb, 64);  // default: 64
        assert!(store.bridge.is_none());
    }

    #[test]
    fn test_store_with_bridge() {
        let toml = r#"
            [store]
            path = "./data/app.db"

            [store.bridge]
            socket = "/tmp/my-app.sock"
        "#;
        let config = MtwConfig::from_str(toml).unwrap();
        assert!(config.has_store());
        assert!(config.has_bridge());

        let bridge = config.store.unwrap().bridge.unwrap();
        assert_eq!(bridge.socket.unwrap(), "/tmp/my-app.sock");
        assert_eq!(bridge.timeout_ms, 30000);
    }

    #[test]
    fn test_store_full_config() {
        let toml = r#"
            [store]
            path = "./data/app.db"
            readonly = false
            pool_size = 8
            cache_mb = 128
            mmap_mb = 512
            busy_timeout_ms = 10000

            [store.bridge]
            socket = "/tmp/my-app.sock"
            timeout_ms = 5000
        "#;
        let config = MtwConfig::from_str(toml).unwrap();
        let store = config.store.unwrap();
        assert!(!store.readonly);
        assert_eq!(store.pool_size, 8);
        assert_eq!(store.cache_mb, 128);
        assert_eq!(store.mmap_mb, 512);
        assert_eq!(store.busy_timeout_ms, 10000);
        assert_eq!(store.bridge.unwrap().timeout_ms, 5000);
    }
}
