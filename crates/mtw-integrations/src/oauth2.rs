//! OAuth2 support for API integrations.
//!
//! Provides a common OAuth2 client with pre-configured profiles for each
//! supported API that uses OAuth2 authentication. Includes automatic token
//! caching and refresh before expiry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// OAuth2 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    /// Application client ID.
    pub client_id: String,
    /// Application client secret.
    pub client_secret: String,
    /// Redirect URI for the OAuth2 flow.
    pub redirect_uri: String,
    /// Authorization endpoint URL.
    pub auth_url: String,
    /// Token endpoint URL.
    pub token_url: String,
    /// Requested scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Token response from the OAuth2 provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2TokenResponse {
    /// Access token.
    pub access_token: String,
    /// Token type (typically "Bearer").
    pub token_type: String,
    /// Expiration time in seconds.
    pub expires_in: Option<u64>,
    /// Refresh token (if available).
    pub refresh_token: Option<String>,
    /// Granted scopes (space-separated).
    pub scope: Option<String>,
}

/// PKCE code verifier/challenge for enhanced security.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// Code verifier (random string).
    pub verifier: String,
    /// Code challenge (S256 hash of verifier).
    pub challenge: String,
    /// Challenge method (always "S256").
    pub method: &'static str,
}

/// Cached token with expiry tracking.
#[derive(Debug, Clone)]
struct TokenCache {
    token: OAuth2TokenResponse,
    expires_at: Instant,
}

/// OAuth2 client with automatic token caching and refresh.
///
/// Thread-safe via `Arc<RwLock>` — can be shared across tasks.
/// Tokens are refreshed automatically 30 seconds before expiry.
pub struct OAuth2Client {
    config: OAuth2Config,
    http_client: reqwest::Client,
    cache: Arc<RwLock<Option<TokenCache>>>,
}

impl OAuth2Client {
    pub fn new(config: OAuth2Config) -> Self {
        Self {
            config,
            http_client: reqwest::Client::new(),
            cache: Arc::new(RwLock::new(None)),
        }
    }

    pub fn config(&self) -> &OAuth2Config {
        &self.config
    }

    /// Generate the authorization URL for the user to visit.
    pub fn authorization_url(&self, state: &str) -> String {
        let scopes = self.config.scopes.join(" ");
        format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
            self.config.auth_url,
            self.config.client_id,
            self.config.redirect_uri,
            scopes,
            state,
        )
    }

    /// Get a valid access token, refreshing automatically if expired or near expiry.
    ///
    /// Returns a cached token if it has more than 30 seconds of validity remaining.
    /// Otherwise fetches a new one from the token endpoint.
    pub async fn get_token(&self) -> Result<String, String> {
        // Check cache first (read lock — cheap)
        {
            let cache = self.cache.read().await;
            if let Some(ref cached) = *cache {
                if cached.expires_at > Instant::now() + Duration::from_secs(30) {
                    return Ok(cached.token.access_token.clone());
                }
            }
        }

        // Cache miss or near-expiry — fetch a new token
        self.fetch_and_cache_token("client_credentials", None).await
    }

    /// Exchange an authorization code for an access token.
    pub async fn exchange_code(&self, code: &str) -> Result<OAuth2TokenResponse, String> {
        self.fetch_and_cache_token("authorization_code", Some(code))
            .await?;

        let cache = self.cache.read().await;
        Ok(cache.as_ref().unwrap().token.clone())
    }

    /// Refresh an expired access token using a refresh token.
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<OAuth2TokenResponse, String> {
        let mut params = vec![
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ];

        let scope_str = self.config.scopes.join(" ");
        if !self.config.scopes.is_empty() {
            params.push(("scope", &scope_str));
        }

        let response = self
            .http_client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Token refresh request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown".to_string());
            return Err(format!("Token refresh returned {}: {}", status, body));
        }

        let token_response = self.parse_token_response(response).await?;

        // Cache the refreshed token
        {
            let expires_in = token_response.expires_in.unwrap_or(3600);
            let mut cache = self.cache.write().await;
            *cache = Some(TokenCache {
                token: token_response.clone(),
                expires_at: Instant::now() + Duration::from_secs(expires_in),
            });
        }

        Ok(token_response)
    }

    /// Force a token refresh, invalidating the cache.
    pub async fn force_refresh(&self) -> Result<String, String> {
        {
            let mut cache = self.cache.write().await;
            *cache = None;
        }
        self.get_token().await
    }

    /// Get the current cached token without fetching.
    pub async fn current_token(&self) -> Option<OAuth2TokenResponse> {
        let cache = self.cache.read().await;
        cache.as_ref().map(|c| c.token.clone())
    }

    /// Set the current token manually (e.g. loaded from storage).
    pub async fn set_token(&self, token: OAuth2TokenResponse) {
        let expires_in = token.expires_in.unwrap_or(3600);
        let mut cache = self.cache.write().await;
        *cache = Some(TokenCache {
            token,
            expires_at: Instant::now() + Duration::from_secs(expires_in),
        });
    }

    /// Check if the cached token is still valid (has >30s remaining).
    pub async fn is_token_valid(&self) -> bool {
        let cache = self.cache.read().await;
        match cache.as_ref() {
            Some(cached) => cached.expires_at > Instant::now() + Duration::from_secs(30),
            None => false,
        }
    }

    async fn fetch_and_cache_token(
        &self,
        grant_type: &str,
        code: Option<&str>,
    ) -> Result<String, String> {
        let mut params = vec![
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("grant_type", grant_type),
        ];

        if let Some(code) = code {
            params.push(("code", code));
            params.push(("redirect_uri", self.config.redirect_uri.as_str()));
        }

        let scope_str = self.config.scopes.join(" ");
        if !self.config.scopes.is_empty() {
            params.push(("scope", &scope_str));
        }

        let response = self
            .http_client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Token request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown".to_string());
            return Err(format!("Token endpoint returned {}: {}", status, body));
        }

        let token_response = self.parse_token_response(response).await?;
        let access_token = token_response.access_token.clone();
        let expires_in = token_response.expires_in.unwrap_or(3600);

        // Cache the token
        {
            let mut cache = self.cache.write().await;
            *cache = Some(TokenCache {
                token: token_response,
                expires_at: Instant::now() + Duration::from_secs(expires_in),
            });
        }

        Ok(access_token)
    }

    async fn parse_token_response(
        &self,
        response: reqwest::Response,
    ) -> Result<OAuth2TokenResponse, String> {
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse token response: {}", e))?;

        let access_token = body["access_token"]
            .as_str()
            .ok_or_else(|| "No access_token in response".to_string())?
            .to_string();

        Ok(OAuth2TokenResponse {
            access_token,
            token_type: body["token_type"]
                .as_str()
                .unwrap_or("Bearer")
                .to_string(),
            expires_in: body["expires_in"].as_u64(),
            refresh_token: body["refresh_token"].as_str().map(|s| s.to_string()),
            scope: body["scope"].as_str().map(|s| s.to_string()),
        })
    }
}

// ---------------------------------------------------------------------------
// Pre-configured OAuth2 profiles for supported APIs
// ---------------------------------------------------------------------------

/// Create a pre-configured OAuth2Config for GitHub.
pub fn github_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://github.com/login/oauth/authorize".to_string(),
        token_url: "https://github.com/login/oauth/access_token".to_string(),
        scopes: vec!["repo".to_string(), "user".to_string()],
    }
}

/// Create a pre-configured OAuth2Config for GitLab.
pub fn gitlab_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://gitlab.com/oauth/authorize".to_string(),
        token_url: "https://gitlab.com/oauth/token".to_string(),
        scopes: vec!["api".to_string(), "read_user".to_string()],
    }
}

/// Create a pre-configured OAuth2Config for Slack.
pub fn slack_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://slack.com/oauth/v2/authorize".to_string(),
        token_url: "https://slack.com/api/oauth.v2.access".to_string(),
        scopes: vec!["chat:write".to_string(), "channels:read".to_string()],
    }
}

/// Create a pre-configured OAuth2Config for Discord.
pub fn discord_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://discord.com/api/oauth2/authorize".to_string(),
        token_url: "https://discord.com/api/oauth2/token".to_string(),
        scopes: vec!["bot".to_string(), "identify".to_string()],
    }
}

/// Create a pre-configured OAuth2Config for Stripe Connect.
pub fn stripe_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://connect.stripe.com/oauth/authorize".to_string(),
        token_url: "https://connect.stripe.com/oauth/token".to_string(),
        scopes: vec!["read_write".to_string()],
    }
}

/// Create a pre-configured OAuth2Config for PayPal.
pub fn paypal_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://www.paypal.com/signin/authorize".to_string(),
        token_url: "https://api-m.paypal.com/v1/oauth2/token".to_string(),
        scopes: vec!["openid".to_string()],
    }
}

/// Create a pre-configured OAuth2Config for Google (Cloud Storage, Firebase, etc.).
pub fn google_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
        token_url: "https://oauth2.googleapis.com/token".to_string(),
        scopes: vec![
            "https://www.googleapis.com/auth/devstorage.read_write".to_string(),
        ],
    }
}

/// Create a pre-configured OAuth2Config for Notion.
pub fn notion_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://api.notion.com/v1/oauth/authorize".to_string(),
        token_url: "https://api.notion.com/v1/oauth/token".to_string(),
        scopes: vec![],
    }
}

/// Create a pre-configured OAuth2Config for Airtable.
pub fn airtable_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://airtable.com/oauth2/v1/authorize".to_string(),
        token_url: "https://airtable.com/oauth2/v1/token".to_string(),
        scopes: vec![
            "data.records:read".to_string(),
            "data.records:write".to_string(),
        ],
    }
}

/// Create a pre-configured OAuth2Config for Jira (Atlassian).
pub fn jira_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://auth.atlassian.com/authorize".to_string(),
        token_url: "https://auth.atlassian.com/oauth/token".to_string(),
        scopes: vec![
            "read:jira-work".to_string(),
            "write:jira-work".to_string(),
        ],
    }
}

/// Create a pre-configured OAuth2Config for Linear.
pub fn linear_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://linear.app/oauth/authorize".to_string(),
        token_url: "https://api.linear.app/oauth/token".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
    }
}

/// Create a pre-configured OAuth2Config for Vercel.
pub fn vercel_oauth2(client_id: String, client_secret: String, redirect_uri: String) -> OAuth2Config {
    OAuth2Config {
        client_id,
        client_secret,
        redirect_uri,
        auth_url: "https://vercel.com/integrations/new".to_string(),
        token_url: "https://api.vercel.com/v2/oauth/access_token".to_string(),
        scopes: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_authorization_url() {
        let config = OAuth2Config {
            client_id: "my_client".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost:3000/callback".to_string(),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
        };
        let client = OAuth2Client::new(config);
        let url = client.authorization_url("random_state_123");
        assert!(url.starts_with("https://example.com/auth?"));
        assert!(url.contains("client_id=my_client"));
        assert!(url.contains("state=random_state_123"));
        assert!(url.contains("scope=read+write") || url.contains("scope=read write"));
    }

    #[test]
    fn test_config_serialization() {
        let config = github_oauth2(
            "id".to_string(),
            "secret".to_string(),
            "http://localhost/cb".to_string(),
        );
        let json = serde_json::to_string(&config).unwrap();
        let parsed: OAuth2Config = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.client_id, "id");
        assert_eq!(
            parsed.auth_url,
            "https://github.com/login/oauth/authorize"
        );
    }

    #[tokio::test]
    async fn test_token_management() {
        let config = OAuth2Config {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/cb".to_string(),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec![],
        };
        let client = OAuth2Client::new(config);
        assert!(client.current_token().await.is_none());
        assert!(!client.is_token_valid().await);

        client
            .set_token(OAuth2TokenResponse {
                access_token: "access_123".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: Some(3600),
                refresh_token: Some("refresh_456".to_string()),
                scope: None,
            })
            .await;

        let token = client.current_token().await.unwrap();
        assert_eq!(token.access_token, "access_123");
        assert!(client.is_token_valid().await);
    }

    #[tokio::test]
    async fn test_token_cache_expiry() {
        let config = OAuth2Config {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/cb".to_string(),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec![],
        };
        let client = OAuth2Client::new(config);

        // Set a token that expires in 10 seconds (within the 30s refresh window)
        client
            .set_token(OAuth2TokenResponse {
                access_token: "short_lived".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: Some(10),
                refresh_token: None,
                scope: None,
            })
            .await;

        // Token exists but should not be considered valid (10s < 30s threshold)
        assert!(client.current_token().await.is_some());
        assert!(!client.is_token_valid().await);
    }

    #[test]
    fn test_all_preconfigured_profiles() {
        let _gh = github_oauth2("id".into(), "s".into(), "u".into());
        let _gl = gitlab_oauth2("id".into(), "s".into(), "u".into());
        let _sl = slack_oauth2("id".into(), "s".into(), "u".into());
        let _dc = discord_oauth2("id".into(), "s".into(), "u".into());
        let _st = stripe_oauth2("id".into(), "s".into(), "u".into());
        let _pp = paypal_oauth2("id".into(), "s".into(), "u".into());
        let _go = google_oauth2("id".into(), "s".into(), "u".into());
        let _no = notion_oauth2("id".into(), "s".into(), "u".into());
        let _at = airtable_oauth2("id".into(), "s".into(), "u".into());
        let _ji = jira_oauth2("id".into(), "s".into(), "u".into());
        let _li = linear_oauth2("id".into(), "s".into(), "u".into());
        let _ve = vercel_oauth2("id".into(), "s".into(), "u".into());
    }
}
