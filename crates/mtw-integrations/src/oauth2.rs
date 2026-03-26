//! OAuth2 support for API integrations.
//!
//! Provides a common OAuth2 client with pre-configured profiles for each
//! supported API that uses OAuth2 authentication.

use serde::{Deserialize, Serialize};

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

/// OAuth2 client stub.
pub struct OAuth2Client {
    config: OAuth2Config,
    /// Current token, if any.
    current_token: Option<OAuth2TokenResponse>,
}

impl OAuth2Client {
    pub fn new(config: OAuth2Config) -> Self {
        Self {
            config,
            current_token: None,
        }
    }

    pub fn config(&self) -> &OAuth2Config {
        &self.config
    }

    /// Generate the authorization URL for the user to visit.
    ///
    /// Returns `(url, state)` where `state` is a random CSRF token that
    /// should be validated when the user is redirected back.
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

    /// Exchange an authorization code for an access token.
    ///
    /// Stub: returns an error indicating this is not yet implemented.
    pub async fn exchange_code(&mut self, _code: &str) -> Result<OAuth2TokenResponse, String> {
        // TODO: implement actual HTTP POST to token_url
        Err("exchange_code not yet implemented".to_string())
    }

    /// Refresh an expired access token using a refresh token.
    ///
    /// Stub: returns an error indicating this is not yet implemented.
    pub async fn refresh_token(
        &mut self,
        _refresh_token: &str,
    ) -> Result<OAuth2TokenResponse, String> {
        // TODO: implement actual HTTP POST to token_url with grant_type=refresh_token
        Err("refresh_token not yet implemented".to_string())
    }

    /// Get the current token, if available.
    pub fn current_token(&self) -> Option<&OAuth2TokenResponse> {
        self.current_token.as_ref()
    }

    /// Set the current token (e.g. loaded from storage).
    pub fn set_token(&mut self, token: OAuth2TokenResponse) {
        self.current_token = Some(token);
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

    #[test]
    fn test_token_management() {
        let config = OAuth2Config {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/cb".to_string(),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec![],
        };
        let mut client = OAuth2Client::new(config);
        assert!(client.current_token().is_none());

        client.set_token(OAuth2TokenResponse {
            access_token: "access_123".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            refresh_token: Some("refresh_456".to_string()),
            scope: None,
        });
        assert_eq!(client.current_token().unwrap().access_token, "access_123");
    }

    #[test]
    fn test_all_preconfigured_profiles() {
        // Verify all pre-configured profiles construct without panic.
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
