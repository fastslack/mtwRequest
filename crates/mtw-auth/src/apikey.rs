use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{AuthClaims, AuthToken, Credentials, MtwAuth};

/// Configuration for API key authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyConfig {
    /// Prefix for generated keys (e.g., "mtw_")
    #[serde(default = "default_prefix")]
    pub key_prefix: String,
    /// Length of the random part of the key
    #[serde(default = "default_key_length")]
    pub key_length: usize,
}

fn default_prefix() -> String {
    "mtw_".to_string()
}

fn default_key_length() -> usize {
    32
}

impl Default for ApiKeyConfig {
    fn default() -> Self {
        Self {
            key_prefix: default_prefix(),
            key_length: default_key_length(),
        }
    }
}

/// Stored API key metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    /// The full API key
    pub key: String,
    /// Owner / subject identifier
    pub owner: String,
    /// Roles assigned to this key
    pub roles: Vec<String>,
    /// Custom claims
    pub custom: HashMap<String, serde_json::Value>,
    /// Created at timestamp
    pub created_at: u64,
    /// Optional expiration timestamp
    pub expires_at: Option<u64>,
    /// Whether the key has been revoked
    pub revoked: bool,
}

/// API key authentication implementation
pub struct ApiKeyAuth {
    config: ApiKeyConfig,
    /// In-memory key store: key -> entry
    keys: Arc<DashMap<String, ApiKeyEntry>>,
}

impl ApiKeyAuth {
    pub fn new(config: ApiKeyConfig) -> Self {
        Self {
            config,
            keys: Arc::new(DashMap::new()),
        }
    }

    /// Generate a new API key for a given owner
    pub fn generate_key(
        &self,
        owner: impl Into<String>,
        roles: Vec<String>,
        custom: HashMap<String, serde_json::Value>,
        expires_at: Option<u64>,
    ) -> ApiKeyEntry {
        let key = format!(
            "{}{}",
            self.config.key_prefix,
            generate_random_string(self.config.key_length)
        );

        let entry = ApiKeyEntry {
            key: key.clone(),
            owner: owner.into(),
            roles,
            custom,
            created_at: current_timestamp(),
            expires_at,
            revoked: false,
        };

        self.keys.insert(key, entry.clone());
        entry
    }

    /// Revoke an API key
    pub fn revoke_key(&self, key: &str) -> Result<(), MtwError> {
        let mut entry = self
            .keys
            .get_mut(key)
            .ok_or_else(|| MtwError::Auth("API key not found".into()))?;
        entry.revoked = true;
        Ok(())
    }

    /// Validate an API key and return its entry
    pub fn validate_key(&self, key: &str) -> Result<ApiKeyEntry, MtwError> {
        let entry = self
            .keys
            .get(key)
            .ok_or_else(|| MtwError::Auth("invalid API key".into()))?;

        if entry.revoked {
            return Err(MtwError::Auth("API key has been revoked".into()));
        }

        if let Some(exp) = entry.expires_at {
            if current_timestamp() > exp {
                return Err(MtwError::Auth("API key has expired".into()));
            }
        }

        Ok(entry.clone())
    }

    /// List all keys for a given owner
    pub fn keys_for_owner(&self, owner: &str) -> Vec<ApiKeyEntry> {
        self.keys
            .iter()
            .filter(|entry| entry.owner == owner)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Total number of stored keys
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }
}

#[async_trait]
impl MtwAuth for ApiKeyAuth {
    fn name(&self) -> &str {
        "apikey"
    }

    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken, MtwError> {
        match credentials {
            Credentials::ApiKey(key) => {
                let entry = self.validate_key(key)?;
                Ok(AuthToken {
                    token: entry.key,
                    token_type: "ApiKey".to_string(),
                    expires_at: entry.expires_at.unwrap_or(u64::MAX),
                    refresh_token: None,
                })
            }
            _ => Err(MtwError::Auth(
                "API key auth only supports ApiKey credentials".into(),
            )),
        }
    }

    async fn validate(&self, token: &str) -> Result<AuthClaims, MtwError> {
        let entry = self.validate_key(token)?;
        Ok(AuthClaims {
            sub: entry.owner,
            iat: entry.created_at,
            exp: entry.expires_at.unwrap_or(u64::MAX),
            roles: entry.roles,
            custom: entry.custom,
        })
    }

    async fn refresh(&self, _token: &str) -> Result<AuthToken, MtwError> {
        Err(MtwError::Auth(
            "API keys cannot be refreshed; generate a new key instead".into(),
        ))
    }
}

/// Generate a random alphanumeric string of the given length
fn generate_random_string(len: usize) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Use ULID for uniqueness combined with timestamp hashing for randomness
    let ulid = ulid::Ulid::new().to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut hasher = DefaultHasher::new();
    ulid.hash(&mut hasher);
    now.hash(&mut hasher);
    let hash1 = hasher.finish();

    let mut hasher2 = DefaultHasher::new();
    hash1.hash(&mut hasher2);
    ulid.hash(&mut hasher2);
    let hash2 = hasher2.finish();

    format!("{:016x}{:016x}", hash1, hash2)
        .chars()
        .take(len)
        .collect()
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_key() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let entry = auth.generate_key("user-1", vec!["admin".to_string()], HashMap::new(), None);

        assert!(entry.key.starts_with("mtw_"));
        assert_eq!(entry.owner, "user-1");
        assert!(entry.roles.contains(&"admin".to_string()));
        assert!(!entry.revoked);
    }

    #[test]
    fn test_unique_keys() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let key1 = auth.generate_key("user-1", vec![], HashMap::new(), None);
        let key2 = auth.generate_key("user-1", vec![], HashMap::new(), None);
        assert_ne!(key1.key, key2.key);
    }

    #[test]
    fn test_validate_key() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let entry = auth.generate_key("user-1", vec![], HashMap::new(), None);

        let validated = auth.validate_key(&entry.key).unwrap();
        assert_eq!(validated.owner, "user-1");
    }

    #[test]
    fn test_validate_nonexistent_key() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let result = auth.validate_key("mtw_nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_revoke_key() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let entry = auth.generate_key("user-1", vec![], HashMap::new(), None);

        auth.revoke_key(&entry.key).unwrap();
        let result = auth.validate_key(&entry.key);
        assert!(result.is_err());
    }

    #[test]
    fn test_expired_key() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        // Create a key that expired 1 second ago
        let entry = auth.generate_key(
            "user-1",
            vec![],
            HashMap::new(),
            Some(current_timestamp().saturating_sub(1)),
        );

        let result = auth.validate_key(&entry.key);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_authenticate_with_api_key() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let entry = auth.generate_key("user-1", vec!["user".to_string()], HashMap::new(), None);

        let creds = Credentials::ApiKey(entry.key.clone());
        let token = auth.authenticate(&creds).await.unwrap();
        assert_eq!(token.token, entry.key);
        assert_eq!(token.token_type, "ApiKey");
    }

    #[tokio::test]
    async fn test_validate_through_trait() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let entry = auth.generate_key(
            "user-2",
            vec!["editor".to_string()],
            HashMap::new(),
            None,
        );

        let claims = auth.validate(&entry.key).await.unwrap();
        assert_eq!(claims.sub, "user-2");
        assert!(claims.roles.contains(&"editor".to_string()));
    }

    #[tokio::test]
    async fn test_refresh_not_supported() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let result = auth.refresh("any-key").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_keys_for_owner() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        auth.generate_key("alice", vec![], HashMap::new(), None);
        auth.generate_key("alice", vec![], HashMap::new(), None);
        auth.generate_key("bob", vec![], HashMap::new(), None);

        let alice_keys = auth.keys_for_owner("alice");
        assert_eq!(alice_keys.len(), 2);

        let bob_keys = auth.keys_for_owner("bob");
        assert_eq!(bob_keys.len(), 1);
    }

    #[test]
    fn test_key_count() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        assert_eq!(auth.key_count(), 0);
        auth.generate_key("user-1", vec![], HashMap::new(), None);
        assert_eq!(auth.key_count(), 1);
        auth.generate_key("user-2", vec![], HashMap::new(), None);
        assert_eq!(auth.key_count(), 2);
    }

    #[tokio::test]
    async fn test_wrong_credential_type() {
        let auth = ApiKeyAuth::new(ApiKeyConfig::default());
        let creds = Credentials::Token("some-token".to_string());
        let result = auth.authenticate(&creds).await;
        assert!(result.is_err());
    }
}
