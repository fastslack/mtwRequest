pub mod apikey;
pub mod jwt;
pub mod middleware;

use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Credentials for authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Credentials {
    /// Bearer token
    Token(String),
    /// API key
    ApiKey(String),
    /// Username and password
    Basic { username: String, password: String },
}

/// Authentication token returned after successful auth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthToken {
    /// The token string
    pub token: String,
    /// Token type (e.g., "Bearer")
    pub token_type: String,
    /// Expiration time as Unix timestamp (seconds)
    pub expires_at: u64,
    /// Refresh token (if supported)
    pub refresh_token: Option<String>,
}

/// Claims extracted from a validated token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims {
    /// Subject (user ID)
    pub sub: String,
    /// Issued at (Unix timestamp)
    pub iat: u64,
    /// Expiration (Unix timestamp)
    pub exp: u64,
    /// Roles assigned to the user
    #[serde(default)]
    pub roles: Vec<String>,
    /// Custom claims
    #[serde(default)]
    pub custom: HashMap<String, serde_json::Value>,
}

/// Authentication trait -- abstraction over different auth strategies
#[async_trait]
pub trait MtwAuth: Send + Sync {
    /// Auth provider name
    fn name(&self) -> &str;

    /// Authenticate with credentials and receive a token
    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken, MtwError>;

    /// Validate a token and extract claims
    async fn validate(&self, token: &str) -> Result<AuthClaims, MtwError>;

    /// Refresh an expired token
    async fn refresh(&self, token: &str) -> Result<AuthToken, MtwError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_serialization() {
        let cred = Credentials::Token("abc123".to_string());
        let json = serde_json::to_string(&cred).unwrap();
        let deserialized: Credentials = serde_json::from_str(&json).unwrap();
        match deserialized {
            Credentials::Token(t) => assert_eq!(t, "abc123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_auth_token() {
        let token = AuthToken {
            token: "jwt-token".to_string(),
            token_type: "Bearer".to_string(),
            expires_at: 1700000000,
            refresh_token: Some("refresh-xyz".to_string()),
        };
        assert_eq!(token.token_type, "Bearer");
        assert!(token.refresh_token.is_some());
    }

    #[test]
    fn test_auth_claims() {
        let claims = AuthClaims {
            sub: "user-123".to_string(),
            iat: 1700000000,
            exp: 1700003600,
            roles: vec!["admin".to_string()],
            custom: HashMap::new(),
        };
        assert_eq!(claims.sub, "user-123");
        assert!(claims.roles.contains(&"admin".to_string()));
    }
}
