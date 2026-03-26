use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

use crate::MtwStateStore;

/// An entry in the memory store
#[derive(Debug, Clone)]
struct Entry {
    value: serde_json::Value,
    expires_at: Option<Instant>,
}

impl Entry {
    fn new(value: serde_json::Value) -> Self {
        Self {
            value,
            expires_at: None,
        }
    }

    fn with_ttl(value: serde_json::Value, ttl: Duration) -> Self {
        Self {
            value,
            expires_at: Some(Instant::now() + ttl),
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Instant::now() > exp)
            .unwrap_or(false)
    }
}

/// In-memory state store backed by DashMap with optional TTL support
pub struct MemoryStore {
    data: Arc<DashMap<String, Entry>>,
    /// Notify handle to signal shutdown of the cleanup task
    shutdown: Arc<Notify>,
}

impl MemoryStore {
    /// Create a new MemoryStore without automatic cleanup
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Create a new MemoryStore with a background cleanup task
    /// that removes expired entries at the given interval
    pub fn with_cleanup(interval: Duration) -> Self {
        let store = Self::new();
        let data = store.data.clone();
        let shutdown = store.shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {
                        let expired_keys: Vec<String> = data
                            .iter()
                            .filter(|entry| entry.value().is_expired())
                            .map(|entry| entry.key().clone())
                            .collect();

                        for key in &expired_keys {
                            data.remove(key);
                        }

                        if !expired_keys.is_empty() {
                            tracing::debug!(count = expired_keys.len(), "cleaned up expired entries");
                        }
                    }
                    _ = shutdown.notified() => {
                        tracing::debug!("memory store cleanup task shutting down");
                        break;
                    }
                }
            }
        });

        store
    }

    /// Signal the background cleanup task to stop
    pub fn shutdown(&self) {
        self.shutdown.notify_one();
    }

    /// Get the number of entries (including expired ones that haven't been cleaned yet)
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if the store is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Remove all entries
    pub fn clear(&self) {
        self.data.clear();
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MemoryStore {
    fn drop(&mut self) {
        self.shutdown.notify_one();
    }
}

#[async_trait]
impl MtwStateStore for MemoryStore {
    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, MtwError> {
        match self.data.get(key) {
            Some(entry) => {
                if entry.is_expired() {
                    // Lazily remove expired entries
                    drop(entry);
                    self.data.remove(key);
                    Ok(None)
                } else {
                    Ok(Some(entry.value.clone()))
                }
            }
            None => Ok(None),
        }
    }

    async fn set(&self, key: &str, value: serde_json::Value) -> Result<(), MtwError> {
        self.data.insert(key.to_string(), Entry::new(value));
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<bool, MtwError> {
        Ok(self.data.remove(key).is_some())
    }

    async fn exists(&self, key: &str) -> Result<bool, MtwError> {
        match self.data.get(key) {
            Some(entry) => {
                if entry.is_expired() {
                    drop(entry);
                    self.data.remove(key);
                    Ok(false)
                } else {
                    Ok(true)
                }
            }
            None => Ok(false),
        }
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>, MtwError> {
        let keys: Vec<String> = self
            .data
            .iter()
            .filter(|entry| !entry.value().is_expired())
            .filter(|entry| glob_match(pattern, entry.key()))
            .map(|entry| entry.key().clone())
            .collect();
        Ok(keys)
    }

    async fn ttl_set(
        &self,
        key: &str,
        value: serde_json::Value,
        ttl_secs: u64,
    ) -> Result<(), MtwError> {
        let entry = Entry::with_ttl(value, Duration::from_secs(ttl_secs));
        self.data.insert(key.to_string(), entry);
        Ok(())
    }
}

/// Simple glob-style pattern matching (supports `*` as wildcard)
fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let parts: Vec<&str> = pattern.split('*').collect();

    if parts.len() == 1 {
        // No wildcard -- exact match
        return pattern == text;
    }

    let mut pos = 0;

    // First part must match the start
    if !parts[0].is_empty() {
        if !text.starts_with(parts[0]) {
            return false;
        }
        pos = parts[0].len();
    }

    // Last part must match the end
    let last = parts[parts.len() - 1];
    if !last.is_empty() && !text.ends_with(last) {
        return false;
    }

    // Middle parts must appear in order
    for &part in &parts[1..parts.len() - 1] {
        if part.is_empty() {
            continue;
        }
        match text[pos..].find(part) {
            Some(idx) => pos += idx + part.len(),
            None => return false,
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("user:*", "user:123"));
        assert!(glob_match("user:*", "user:abc:def"));
        assert!(!glob_match("user:*", "session:123"));
        assert!(glob_match("*:session", "user:session"));
        assert!(!glob_match("*:session", "user:token"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "other"));
        assert!(glob_match("a*b*c", "aXbYc"));
        assert!(!glob_match("a*b*c", "aXcYb"));
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let store = MemoryStore::new();
        store
            .set("key1", serde_json::json!("value1"))
            .await
            .unwrap();

        let val = store.get("key1").await.unwrap();
        assert_eq!(val, Some(serde_json::json!("value1")));
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let store = MemoryStore::new();
        let val = store.get("missing").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn test_delete() {
        let store = MemoryStore::new();
        store
            .set("key1", serde_json::json!("value1"))
            .await
            .unwrap();

        let deleted = store.delete("key1").await.unwrap();
        assert!(deleted);

        let deleted_again = store.delete("key1").await.unwrap();
        assert!(!deleted_again);

        let val = store.get("key1").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn test_exists() {
        let store = MemoryStore::new();
        assert!(!store.exists("key1").await.unwrap());

        store
            .set("key1", serde_json::json!(true))
            .await
            .unwrap();
        assert!(store.exists("key1").await.unwrap());
    }

    #[tokio::test]
    async fn test_keys_pattern() {
        let store = MemoryStore::new();
        store
            .set("user:1", serde_json::json!("alice"))
            .await
            .unwrap();
        store
            .set("user:2", serde_json::json!("bob"))
            .await
            .unwrap();
        store
            .set("session:1", serde_json::json!("sess"))
            .await
            .unwrap();

        let mut user_keys = store.keys("user:*").await.unwrap();
        user_keys.sort();
        assert_eq!(user_keys, vec!["user:1", "user:2"]);

        let all_keys = store.keys("*").await.unwrap();
        assert_eq!(all_keys.len(), 3);

        let session_keys = store.keys("session:*").await.unwrap();
        assert_eq!(session_keys.len(), 1);
    }

    #[tokio::test]
    async fn test_ttl_set_and_expiration() {
        let store = MemoryStore::new();

        // Set with a very short TTL
        store
            .ttl_set("ephemeral", serde_json::json!("temp"), 0)
            .await
            .unwrap();

        // The entry should be considered expired almost immediately (TTL=0 means expired now)
        tokio::time::sleep(Duration::from_millis(10)).await;

        let val = store.get("ephemeral").await.unwrap();
        assert!(val.is_none(), "expired entry should return None");
    }

    #[tokio::test]
    async fn test_ttl_not_expired() {
        let store = MemoryStore::new();
        store
            .ttl_set("lasting", serde_json::json!("data"), 3600)
            .await
            .unwrap();

        let val = store.get("lasting").await.unwrap();
        assert_eq!(val, Some(serde_json::json!("data")));
    }

    #[tokio::test]
    async fn test_overwrite() {
        let store = MemoryStore::new();
        store
            .set("key", serde_json::json!("v1"))
            .await
            .unwrap();
        store
            .set("key", serde_json::json!("v2"))
            .await
            .unwrap();

        let val = store.get("key").await.unwrap();
        assert_eq!(val, Some(serde_json::json!("v2")));
    }

    #[tokio::test]
    async fn test_complex_values() {
        let store = MemoryStore::new();
        let complex = serde_json::json!({
            "name": "Alice",
            "age": 30,
            "tags": ["admin", "user"],
            "nested": { "key": "value" }
        });

        store.set("complex", complex.clone()).await.unwrap();
        let val = store.get("complex").await.unwrap().unwrap();
        assert_eq!(val, complex);
    }

    #[tokio::test]
    async fn test_len_and_clear() {
        let store = MemoryStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        store.set("a", serde_json::json!(1)).await.unwrap();
        store.set("b", serde_json::json!(2)).await.unwrap();
        assert_eq!(store.len(), 2);
        assert!(!store.is_empty());

        store.clear();
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn test_expired_key_not_in_exists() {
        let store = MemoryStore::new();
        store
            .ttl_set("temp", serde_json::json!("val"), 0)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!store.exists("temp").await.unwrap());
    }

    #[tokio::test]
    async fn test_expired_keys_filtered_from_pattern() {
        let store = MemoryStore::new();
        store
            .ttl_set("user:expired", serde_json::json!("old"), 0)
            .await
            .unwrap();
        store
            .set("user:active", serde_json::json!("new"))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let keys = store.keys("user:*").await.unwrap();
        assert_eq!(keys, vec!["user:active"]);
    }
}
