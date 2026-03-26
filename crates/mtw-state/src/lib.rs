pub mod memory;
pub mod redis;

use async_trait::async_trait;
use mtw_core::MtwError;

/// State store trait -- abstraction over key-value storage backends
#[async_trait]
pub trait MtwStateStore: Send + Sync {
    /// Get a value by key
    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, MtwError>;

    /// Set a key-value pair
    async fn set(&self, key: &str, value: serde_json::Value) -> Result<(), MtwError>;

    /// Delete a key, returning true if it existed
    async fn delete(&self, key: &str) -> Result<bool, MtwError>;

    /// Check if a key exists
    async fn exists(&self, key: &str) -> Result<bool, MtwError>;

    /// List keys matching a glob-style pattern (supports `*` wildcard)
    async fn keys(&self, pattern: &str) -> Result<Vec<String>, MtwError>;

    /// Set a key-value pair with a TTL (time-to-live) in seconds
    async fn ttl_set(
        &self,
        key: &str,
        value: serde_json::Value,
        ttl_secs: u64,
    ) -> Result<(), MtwError>;
}
