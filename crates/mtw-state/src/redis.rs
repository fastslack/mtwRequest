use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};

use crate::MtwStateStore;

/// Configuration for the Redis state store adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// Redis connection URL
    pub url: String,
    /// Key prefix for namespacing
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

fn default_prefix() -> String {
    "mtw:".to_string()
}

impl RedisConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            prefix: default_prefix(),
        }
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }
}

/// Redis state store adapter (stub implementation)
pub struct RedisStore {
    config: RedisConfig,
}

impl RedisStore {
    pub fn new(config: RedisConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &RedisConfig {
        &self.config
    }

    /// Build a prefixed key
    fn prefixed_key(&self, key: &str) -> String {
        format!("{}{}", self.config.prefix, key)
    }
}

#[async_trait]
impl MtwStateStore for RedisStore {
    async fn get(&self, _key: &str) -> Result<Option<serde_json::Value>, MtwError> {
        Err(MtwError::Internal(
            "redis state store not yet implemented".into(),
        ))
    }

    async fn set(&self, _key: &str, _value: serde_json::Value) -> Result<(), MtwError> {
        Err(MtwError::Internal(
            "redis state store not yet implemented".into(),
        ))
    }

    async fn delete(&self, _key: &str) -> Result<bool, MtwError> {
        Err(MtwError::Internal(
            "redis state store not yet implemented".into(),
        ))
    }

    async fn exists(&self, _key: &str) -> Result<bool, MtwError> {
        Err(MtwError::Internal(
            "redis state store not yet implemented".into(),
        ))
    }

    async fn keys(&self, _pattern: &str) -> Result<Vec<String>, MtwError> {
        Err(MtwError::Internal(
            "redis state store not yet implemented".into(),
        ))
    }

    async fn ttl_set(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _ttl_secs: u64,
    ) -> Result<(), MtwError> {
        Err(MtwError::Internal(
            "redis state store not yet implemented".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_config() {
        let config = RedisConfig::new("redis://localhost:6379");
        assert_eq!(config.url, "redis://localhost:6379");
        assert_eq!(config.prefix, "mtw:");
    }

    #[test]
    fn test_redis_config_with_prefix() {
        let config = RedisConfig::new("redis://localhost:6379").with_prefix("app:");
        assert_eq!(config.prefix, "app:");
    }

    #[test]
    fn test_prefixed_key() {
        let store = RedisStore::new(RedisConfig::new("redis://localhost:6379"));
        assert_eq!(store.prefixed_key("user:1"), "mtw:user:1");
    }

    #[tokio::test]
    async fn test_stub_returns_error() {
        let store = RedisStore::new(RedisConfig::new("redis://localhost:6379"));
        assert!(store.get("key").await.is_err());
        assert!(store.set("key", serde_json::json!(1)).await.is_err());
        assert!(store.delete("key").await.is_err());
        assert!(store.exists("key").await.is_err());
        assert!(store.keys("*").await.is_err());
        assert!(store
            .ttl_set("key", serde_json::json!(1), 60)
            .await
            .is_err());
    }
}
