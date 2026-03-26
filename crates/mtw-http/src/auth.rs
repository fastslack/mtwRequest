use std::fmt;

/// Authentication strategies for HTTP requests.
pub enum AuthStrategy {
    /// Bearer token authentication.
    Bearer(String),
    /// Basic username/password authentication.
    Basic { username: String, password: String },
    /// API key sent in a specific header.
    ApiKey { header: String, value: String },
    /// OAuth2 with refresh support.
    OAuth2 {
        token: String,
        refresh_url: String,
        client_id: String,
        client_secret: String,
    },
    /// Custom authentication function applied to each request builder.
    Custom(Box<dyn Fn(&mut reqwest::RequestBuilder) + Send + Sync>),
}

impl fmt::Debug for AuthStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer(_) => f.write_str("Bearer(***)"),
            Self::Basic { username, .. } => write!(f, "Basic({}:***)", username),
            Self::ApiKey { header, .. } => write!(f, "ApiKey({}:***)", header),
            Self::OAuth2 { client_id, .. } => write!(f, "OAuth2(client_id={})", client_id),
            Self::Custom(_) => f.write_str("Custom(fn)"),
        }
    }
}

impl AuthStrategy {
    /// Apply authentication to a reqwest::RequestBuilder.
    pub fn apply(&self, mut builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::Bearer(token) => builder.bearer_auth(token),
            Self::Basic { username, password } => builder.basic_auth(username, Some(password)),
            Self::ApiKey { header, value } => builder.header(header.as_str(), value.as_str()),
            Self::OAuth2 { token, .. } => builder.bearer_auth(token),
            Self::Custom(f) => {
                f(&mut builder);
                builder
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_debug_redacts_secrets() {
        let bearer = AuthStrategy::Bearer("secret".into());
        let debug_str = format!("{:?}", bearer);
        assert!(!debug_str.contains("secret"));
        assert!(debug_str.contains("***"));
    }

    #[test]
    fn test_basic_debug() {
        let basic = AuthStrategy::Basic {
            username: "user".into(),
            password: "pass".into(),
        };
        let debug_str = format!("{:?}", basic);
        assert!(debug_str.contains("user"));
        assert!(!debug_str.contains("pass"));
    }
}
