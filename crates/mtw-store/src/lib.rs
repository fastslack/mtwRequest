//! # mtw-store
//!
//! Native data store for mtwRequest with SQLite connection pooling.
//! Completely project-agnostic — works with any SQLite database.
//!
//! ## Quick Start
//!
//! In `mtw.toml`:
//! ```toml
//! [store]
//! path = "./data/app.db"
//! ```
//!
//! That's it. mtwRequest auto-detects SQLite, enables WAL mode,
//! creates a read-only connection pool, and is ready to query.

pub mod sqlite;

use async_trait::async_trait;
use mtw_core::MtwError;

/// Result of a store query — always JSON
pub type StoreResult = Result<serde_json::Value, MtwError>;

/// Trait for data store backends.
/// No assumptions about schema — works with any database.
#[async_trait]
pub trait MtwStore: Send + Sync {
    /// Execute a read query by table name, returns all rows as JSON array.
    /// The caller controls filtering via `params`.
    async fn query(&self, table: &str, params: serde_json::Value) -> StoreResult;

    /// Execute a raw SQL query (read-only), returns JSON array of rows.
    /// The SQL is provided by the caller — the store makes no assumptions.
    async fn query_raw(&self, sql: &str, params: &[serde_json::Value]) -> StoreResult;

    /// Check if the store is healthy and accessible
    async fn health(&self) -> Result<StoreHealth, MtwError>;

    /// Get store metadata (table count, size, etc.)
    async fn info(&self) -> Result<serde_json::Value, MtwError>;
}

/// Store health status
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoreHealth {
    pub available: bool,
    pub driver: String,
    pub path: String,
    pub pool_size: u32,
    pub pool_idle: u32,
}

/// Create a store from config — auto-detects driver from path
pub fn from_config(config: &StoreConfig) -> Result<Box<dyn MtwStore>, MtwError> {
    let driver = config.driver();

    match driver.as_str() {
        "sqlite" => {
            let store = sqlite::SqliteStore::open(config)?;
            Ok(Box::new(store))
        }
        other => Err(MtwError::Config(format!(
            "unsupported store driver: '{}' (supported: sqlite)",
            other
        ))),
    }
}

/// Store configuration — designed to be dead simple
///
/// Minimal config (just a path):
/// ```toml
/// [store]
/// path = "./data/app.db"
/// ```
///
/// Full config with all options:
/// ```toml
/// [store]
/// path = "./data/app.db"
/// readonly = true
/// pool_size = 4
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoreConfig {
    /// Path to the database file (required)
    pub path: String,

    /// Force read-only mode (default: true for safety)
    #[serde(default = "default_readonly")]
    pub readonly: bool,

    /// Connection pool size (default: 4)
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Cache size in MB (default: 64)
    #[serde(default = "default_cache_mb")]
    pub cache_mb: u32,

    /// Enable memory-mapped I/O in MB (default: 256)
    #[serde(default = "default_mmap_mb")]
    pub mmap_mb: u32,

    /// Busy timeout in milliseconds (default: 5000)
    #[serde(default = "default_busy_timeout")]
    pub busy_timeout_ms: u64,
}

impl StoreConfig {
    /// Auto-detect driver from file extension
    pub fn driver(&self) -> String {
        if self.path.ends_with(".db")
            || self.path.ends_with(".sqlite")
            || self.path.ends_with(".sqlite3")
        {
            "sqlite".into()
        } else {
            // Default to SQLite — it's the most common case
            "sqlite".into()
        }
    }
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            readonly: default_readonly(),
            pool_size: default_pool_size(),
            cache_mb: default_cache_mb(),
            mmap_mb: default_mmap_mb(),
            busy_timeout_ms: default_busy_timeout(),
        }
    }
}

fn default_readonly() -> bool {
    true
}
fn default_pool_size() -> u32 {
    4
}
fn default_cache_mb() -> u32 {
    64
}
fn default_mmap_mb() -> u32 {
    256
}
fn default_busy_timeout() -> u64 {
    5000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_detection() {
        let config = StoreConfig {
            path: "./data/app.db".into(),
            ..Default::default()
        };
        assert_eq!(config.driver(), "sqlite");

        let config2 = StoreConfig {
            path: "./data/app.sqlite3".into(),
            ..Default::default()
        };
        assert_eq!(config2.driver(), "sqlite");
    }

    #[test]
    fn test_defaults() {
        let config = StoreConfig::default();
        assert!(config.readonly);
        assert_eq!(config.pool_size, 4);
        assert_eq!(config.cache_mb, 64);
        assert_eq!(config.mmap_mb, 256);
        assert_eq!(config.busy_timeout_ms, 5000);
    }

    #[test]
    fn test_parse_minimal_toml() {
        let toml = r#"path = "./data/app.db""#;
        let config: StoreConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.path, "./data/app.db");
        assert!(config.readonly);
        assert_eq!(config.pool_size, 4);
    }

    #[test]
    fn test_parse_full_toml() {
        let toml = r#"
            path = "./data/app.db"
            readonly = false
            pool_size = 8
            cache_mb = 128
            mmap_mb = 512
            busy_timeout_ms = 10000
        "#;
        let config: StoreConfig = toml::from_str(toml).unwrap();
        assert!(!config.readonly);
        assert_eq!(config.pool_size, 8);
        assert_eq!(config.cache_mb, 128);
    }
}
