use async_trait::async_trait;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{AuthClaims, AuthToken, Credentials, MtwAuth};

/// JWT algorithm options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JwtAlgorithm {
    Hs256,
    Hs384,
    Hs512,
}

impl JwtAlgorithm {
    fn to_jsonwebtoken(&self) -> jsonwebtoken::Algorithm {
        match self {
            JwtAlgorithm::Hs256 => jsonwebtoken::Algorithm::HS256,
            JwtAlgorithm::Hs384 => jsonwebtoken::Algorithm::HS384,
            JwtAlgorithm::Hs512 => jsonwebtoken::Algorithm::HS512,
        }
    }
}

/// Configuration for JWT authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    /// Signing secret
    pub secret: String,
    /// Algorithm to use
    #[serde(default = "default_algorithm")]
    pub algorithm: JwtAlgorithm,
    /// Token expiration in seconds
    #[serde(default = "default_expiration")]
    pub expiration_secs: u64,
    /// Refresh token expiration in seconds
    #[serde(default = "default_refresh_expiration")]
    pub refresh_expiration_secs: u64,
    /// Issuer claim
    pub issuer: Option<String>,
    /// Audience claim
    pub audience: Option<String>,
}

fn default_algorithm() -> JwtAlgorithm {
    JwtAlgorithm::Hs256
}

fn default_expiration() -> u64 {
    3600 // 1 hour
}

fn default_refresh_expiration() -> u64 {
    86400 * 7 // 7 days
}

impl JwtConfig {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            algorithm: default_algorithm(),
            expiration_secs: default_expiration(),
            refresh_expiration_secs: default_refresh_expiration(),
            issuer: None,
            audience: None,
        }
    }

    pub fn with_expiration(mut self, secs: u64) -> Self {
        self.expiration_secs = secs;
        self
    }

    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }
}

/// Internal JWT claims structure used for encoding/decoding
#[derive(Debug, Serialize, Deserialize)]
struct JwtPayload {
    sub: String,
    iat: u64,
    exp: u64,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    custom: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<String>,
    /// Whether this is a refresh token
    #[serde(default)]
    is_refresh: bool,
}

/// JWT authentication implementation
pub struct JwtAuth {
    config: JwtConfig,
}

impl JwtAuth {
    pub fn new(config: JwtConfig) -> Self {
        Self { config }
    }

    /// Create a token for a given subject with roles and custom claims
    pub fn create_token(
        &self,
        sub: impl Into<String>,
        roles: Vec<String>,
        custom: HashMap<String, serde_json::Value>,
    ) -> Result<AuthToken, MtwError> {
        let now = current_timestamp();
        let exp = now + self.config.expiration_secs;

        let payload = JwtPayload {
            sub: sub.into(),
            iat: now,
            exp,
            roles: roles.clone(),
            custom: custom.clone(),
            iss: self.config.issuer.clone(),
            aud: self.config.audience.clone(),
            is_refresh: false,
        };

        let header = Header::new(self.config.algorithm.to_jsonwebtoken());
        let token = encode(
            &header,
            &payload,
            &EncodingKey::from_secret(self.config.secret.as_bytes()),
        )
        .map_err(|e| MtwError::Auth(format!("failed to create token: {}", e)))?;

        // Create refresh token
        let refresh_payload = JwtPayload {
            sub: payload.sub.clone(),
            iat: now,
            exp: now + self.config.refresh_expiration_secs,
            roles,
            custom,
            iss: self.config.issuer.clone(),
            aud: self.config.audience.clone(),
            is_refresh: true,
        };

        let refresh_token = encode(
            &header,
            &refresh_payload,
            &EncodingKey::from_secret(self.config.secret.as_bytes()),
        )
        .map_err(|e| MtwError::Auth(format!("failed to create refresh token: {}", e)))?;

        Ok(AuthToken {
            token,
            token_type: "Bearer".to_string(),
            expires_at: exp,
            refresh_token: Some(refresh_token),
        })
    }

    /// Decode and validate a JWT token
    fn decode_token(&self, token: &str) -> Result<JwtPayload, MtwError> {
        let mut validation = Validation::new(self.config.algorithm.to_jsonwebtoken());

        if let Some(ref issuer) = self.config.issuer {
            validation.set_issuer(&[issuer]);
        }

        if let Some(ref audience) = self.config.audience {
            validation.set_audience(&[audience]);
        }

        let token_data = decode::<JwtPayload>(
            token,
            &DecodingKey::from_secret(self.config.secret.as_bytes()),
            &validation,
        )
        .map_err(|e| MtwError::Auth(format!("invalid token: {}", e)))?;

        Ok(token_data.claims)
    }
}

#[async_trait]
impl MtwAuth for JwtAuth {
    fn name(&self) -> &str {
        "jwt"
    }

    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken, MtwError> {
        match credentials {
            Credentials::Token(token) => {
                // Validate existing token and reissue
                let claims = self.decode_token(token)?;
                self.create_token(claims.sub, claims.roles, claims.custom)
            }
            Credentials::Basic { username, .. } => {
                // For JWT, we just create a token for the user
                // Real auth logic (password verification) would be in a higher layer
                self.create_token(username, vec![], HashMap::new())
            }
            Credentials::ApiKey(_) => Err(MtwError::Auth(
                "JWT auth does not support API key credentials".into(),
            )),
        }
    }

    async fn validate(&self, token: &str) -> Result<AuthClaims, MtwError> {
        let payload = self.decode_token(token)?;

        if payload.is_refresh {
            return Err(MtwError::Auth(
                "refresh tokens cannot be used for authentication".into(),
            ));
        }

        Ok(AuthClaims {
            sub: payload.sub,
            iat: payload.iat,
            exp: payload.exp,
            roles: payload.roles,
            custom: payload.custom,
        })
    }

    async fn refresh(&self, token: &str) -> Result<AuthToken, MtwError> {
        let payload = self.decode_token(token)?;

        if !payload.is_refresh {
            return Err(MtwError::Auth(
                "only refresh tokens can be used to refresh".into(),
            ));
        }

        self.create_token(payload.sub, payload.roles, payload.custom)
    }
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

    fn test_config() -> JwtConfig {
        JwtConfig::new("super-secret-key-for-testing-only")
            .with_expiration(3600)
            .with_issuer("mtw-test")
    }

    #[test]
    fn test_jwt_config() {
        let config = test_config();
        assert_eq!(config.expiration_secs, 3600);
        assert_eq!(config.issuer, Some("mtw-test".to_string()));
    }

    #[test]
    fn test_create_token() {
        let auth = JwtAuth::new(test_config());
        let token = auth
            .create_token("user-123", vec!["admin".to_string()], HashMap::new())
            .unwrap();
        assert!(!token.token.is_empty());
        assert_eq!(token.token_type, "Bearer");
        assert!(token.refresh_token.is_some());
        assert!(token.expires_at > 0);
    }

    #[tokio::test]
    async fn test_validate_token() {
        let auth = JwtAuth::new(test_config());
        let token = auth
            .create_token(
                "user-456",
                vec!["editor".to_string()],
                HashMap::new(),
            )
            .unwrap();

        let claims = auth.validate(&token.token).await.unwrap();
        assert_eq!(claims.sub, "user-456");
        assert!(claims.roles.contains(&"editor".to_string()));
        assert!(claims.exp > claims.iat);
    }

    #[tokio::test]
    async fn test_validate_invalid_token() {
        let auth = JwtAuth::new(test_config());
        let result = auth.validate("invalid-token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_refresh_token() {
        let auth = JwtAuth::new(test_config());
        let original = auth
            .create_token("user-789", vec!["user".to_string()], HashMap::new())
            .unwrap();

        let refresh_token = original.refresh_token.unwrap();

        // Sleep briefly so the new token gets a different iat timestamp
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let new_token = auth.refresh(&refresh_token).await.unwrap();

        assert!(!new_token.token.is_empty());

        let claims = auth.validate(&new_token.token).await.unwrap();
        assert_eq!(claims.sub, "user-789");
    }

    #[tokio::test]
    async fn test_cannot_refresh_with_access_token() {
        let auth = JwtAuth::new(test_config());
        let token = auth
            .create_token("user-1", vec![], HashMap::new())
            .unwrap();

        let result = auth.refresh(&token.token).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cannot_auth_with_refresh_token() {
        let auth = JwtAuth::new(test_config());
        let token = auth
            .create_token("user-1", vec![], HashMap::new())
            .unwrap();

        let refresh = token.refresh_token.unwrap();
        let result = auth.validate(&refresh).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_authenticate_basic() {
        let auth = JwtAuth::new(test_config());
        let creds = Credentials::Basic {
            username: "testuser".to_string(),
            password: "pass".to_string(),
        };
        let token = auth.authenticate(&creds).await.unwrap();
        let claims = auth.validate(&token.token).await.unwrap();
        assert_eq!(claims.sub, "testuser");
    }

    #[tokio::test]
    async fn test_authenticate_api_key_rejected() {
        let auth = JwtAuth::new(test_config());
        let creds = Credentials::ApiKey("key-123".to_string());
        let result = auth.authenticate(&creds).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_custom_claims() {
        let auth = JwtAuth::new(test_config());
        let mut custom = HashMap::new();
        custom.insert("team".to_string(), serde_json::json!("engineering"));
        custom.insert("level".to_string(), serde_json::json!(5));

        let token = auth
            .create_token("user-custom", vec![], custom)
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let claims = rt.block_on(auth.validate(&token.token)).unwrap();
        assert_eq!(
            claims.custom.get("team"),
            Some(&serde_json::json!("engineering"))
        );
        assert_eq!(claims.custom.get("level"), Some(&serde_json::json!(5)));
    }

    #[tokio::test]
    async fn test_wrong_secret_fails_validation() {
        let auth1 = JwtAuth::new(JwtConfig::new("secret-one"));
        let auth2 = JwtAuth::new(JwtConfig::new("secret-two"));

        let token = auth1
            .create_token("user-1", vec![], HashMap::new())
            .unwrap();

        let result = auth2.validate(&token.token).await;
        assert!(result.is_err());
    }
}
