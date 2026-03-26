# Auth Guide

This guide covers authentication in mtwRequest: JWT tokens, API keys, OAuth2 integration, and the auth middleware that enforces authentication on the message pipeline.

---

## Authentication Overview

mtwRequest provides a pluggable authentication system through the `MtwAuth` trait (in `crates/mtw-auth/src/lib.rs`). The built-in implementations include:

- **JWT** -- JSON Web Tokens with HMAC signing, refresh tokens, and custom claims
- **API Keys** -- Generated keys with validation and revocation
- **OAuth2** -- 12 pre-configured providers (GitHub, Google, Slack, etc.)

Authentication is enforced by the `AuthMiddleware`, which intercepts inbound messages and validates tokens before they reach the message handler.

---

## The MtwAuth Trait

```rust
#[async_trait]
pub trait MtwAuth: Send + Sync {
    /// Provider name (e.g., "jwt", "apikey")
    fn name(&self) -> &str;

    /// Authenticate with credentials and receive a token
    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken, MtwError>;

    /// Validate a token and extract claims
    async fn validate(&self, token: &str) -> Result<AuthClaims, MtwError>;

    /// Refresh an expired token using a refresh token
    async fn refresh(&self, token: &str) -> Result<AuthToken, MtwError>;
}
```

### Credential Types

```rust
pub enum Credentials {
    Token(String),                                  // Bearer token
    ApiKey(String),                                 // API key
    Basic { username: String, password: String },   // Username/password
}
```

### AuthToken (returned after authentication)

```rust
pub struct AuthToken {
    pub token: String,                  // The access token
    pub token_type: String,             // "Bearer"
    pub expires_at: u64,                // Unix timestamp (seconds)
    pub refresh_token: Option<String>,  // For token refresh
}
```

### AuthClaims (extracted from a validated token)

```rust
pub struct AuthClaims {
    pub sub: String,                             // Subject (user ID)
    pub iat: u64,                                // Issued at
    pub exp: u64,                                // Expires at
    pub roles: Vec<String>,                      // User roles
    pub custom: HashMap<String, Value>,          // Custom claims
}
```

---

## JWT Authentication

The JWT implementation is in `crates/mtw-auth/src/jwt.rs`.

### Setup

```rust
use mtw_auth::jwt::{JwtAuth, JwtConfig};

let config = JwtConfig::new("your-super-secret-key")
    .with_expiration(3600)           // 1 hour access token
    .with_issuer("my-app");          // Optional issuer claim

let jwt = JwtAuth::new(config);
```

### JwtConfig Options

| Field | Default | Description |
|-------|---------|-------------|
| `secret` | (required) | HMAC signing secret |
| `algorithm` | `Hs256` | Algorithm: `Hs256`, `Hs384`, `Hs512` |
| `expiration_secs` | `3600` | Access token lifetime (1 hour) |
| `refresh_expiration_secs` | `604800` | Refresh token lifetime (7 days) |
| `issuer` | `None` | `iss` claim for validation |
| `audience` | `None` | `aud` claim for validation |

### Creating Tokens

```rust
use std::collections::HashMap;

// Basic token
let token = jwt.create_token("user-123", vec!["admin".into()], HashMap::new())?;

// Token with custom claims
let mut custom = HashMap::new();
custom.insert("team".to_string(), json!("engineering"));
custom.insert("level".to_string(), json!(5));

let token = jwt.create_token("user-123", vec!["admin".into()], custom)?;

// token.token          -> "eyJhbGciOiJIUzI1NiIs..."
// token.token_type     -> "Bearer"
// token.expires_at     -> 1711846800
// token.refresh_token  -> Some("eyJhbGciOiJIUzI1NiIs...")
```

### Validating Tokens

```rust
let claims = jwt.validate(&token.token).await?;

assert_eq!(claims.sub, "user-123");
assert!(claims.roles.contains(&"admin".to_string()));
assert_eq!(claims.custom.get("team"), Some(&json!("engineering")));
```

Validation checks:
- Signature verification (HMAC)
- Expiration (`exp` claim)
- Issuer (`iss`) if configured
- Audience (`aud`) if configured
- Rejects refresh tokens used as access tokens

### Refreshing Tokens

```rust
// Use the refresh token to get a new access token
let new_token = jwt.refresh(&token.refresh_token.unwrap()).await?;

// The new token has fresh expiration
let claims = jwt.validate(&new_token.token).await?;
```

Rules:
- Only refresh tokens can be used for refresh (access tokens are rejected)
- Refresh tokens cannot be used for authentication (validated and rejected)
- New tokens inherit the same subject, roles, and custom claims

### JWT in mtw.toml

```toml
[[modules]]
name = "mtw-auth-jwt"
version = "1.0"
config = {
    secret = "${JWT_SECRET}",
    algorithm = "hs256",
    expiration = 7200,
    refresh_expiration = 604800,
    issuer = "my-app"
}
```

---

## API Key Authentication

The API key system is in `crates/mtw-auth/src/apikey.rs`. API keys are long-lived tokens suitable for service-to-service authentication.

```rust
use mtw_auth::apikey::ApiKeyAuth;

let apikey_auth = ApiKeyAuth::new();

// Generate a key
let key_info = apikey_auth.generate("my-service", vec!["read".into()]);

// Validate
let claims = apikey_auth.validate(&key_info.key).await?;

// Revoke
apikey_auth.revoke(&key_info.key).await?;
```

---

## Auth Middleware

The `AuthMiddleware` (in `crates/mtw-auth/src/middleware.rs`) intercepts inbound messages and validates authentication before allowing them through the pipeline.

### How It Works

```
Client                  AuthMiddleware              Handler
  |                          |                         |
  |--- Message ------------->|                         |
  |   (metadata.auth_token)  |                         |
  |                          |-- validate token ------>|
  |                          |                         |
  |                          |-- if valid:             |
  |                          |   add auth_claims       |
  |                          |   add auth_user         |
  |                          |--- Continue ----------->|
  |                          |                         |
  |                          |-- if invalid:           |
  |                          |--- Error (401) -------->|
```

### Setup

```rust
use mtw_auth::middleware::AuthMiddleware;
use mtw_auth::jwt::{JwtAuth, JwtConfig};
use mtw_router::middleware::MiddlewareChain;
use std::sync::Arc;

let jwt = Arc::new(JwtAuth::new(JwtConfig::new("secret")));

let auth_mw = AuthMiddleware::new(jwt)
    .with_token_key("auth_token")          // Metadata key for the token
    .with_bypass(MsgType::Subscribe);      // Skip auth for subscribe messages

let mut chain = MiddlewareChain::new();
chain.add(Arc::new(auth_mw));
```

### Default Bypass Types

These message types skip authentication by default:
- `Connect` -- connection handshake
- `Ping` -- keep-alive
- `Pong` -- keep-alive response

### What the Middleware Does

1. Checks if the message type is in the bypass list
2. Extracts the token from `msg.metadata["auth_token"]` (configurable key)
3. Calls `auth.validate(token)` to verify the token
4. On success: attaches `auth_claims` and `auth_user` to the message metadata
5. On failure: returns an `MtwError::Auth` error (message is rejected)

### Accessing Auth Claims in Handlers

After the middleware runs, downstream handlers can access the authenticated user:

```rust
// In your message handler
if let Some(user) = msg.metadata.get("auth_user") {
    tracing::info!("message from user: {}", user);
}

if let Some(claims) = msg.metadata.get("auth_claims") {
    let claims: AuthClaims = serde_json::from_value(claims.clone())?;
    if claims.roles.contains(&"admin".to_string()) {
        // Admin-only logic
    }
}
```

### Priority

The auth middleware runs at priority **10** (very early in the chain), ensuring authentication happens before any business logic middleware.

---

## OAuth2 Integration

The `mtw-integrations` crate provides OAuth2 support with 12 pre-configured providers. Source: `crates/mtw-integrations/src/oauth2.rs`.

### Pre-configured Providers

| Provider | Function | Default Scopes |
|----------|----------|----------------|
| GitHub | `github_oauth2()` | `repo`, `user` |
| GitLab | `gitlab_oauth2()` | `api`, `read_user` |
| Slack | `slack_oauth2()` | `chat:write`, `channels:read` |
| Discord | `discord_oauth2()` | `bot`, `identify` |
| Stripe | `stripe_oauth2()` | `read_write` |
| PayPal | `paypal_oauth2()` | `openid` |
| Google | `google_oauth2()` | `devstorage.read_write` |
| Notion | `notion_oauth2()` | (none) |
| Airtable | `airtable_oauth2()` | `data.records:read`, `data.records:write` |
| Jira | `jira_oauth2()` | `read:jira-work`, `write:jira-work` |
| Linear | `linear_oauth2()` | `read`, `write` |
| Vercel | `vercel_oauth2()` | (none) |

### Usage

```rust
use mtw_integrations::oauth2::{OAuth2Client, github_oauth2};

// Create a client with pre-configured GitHub settings
let config = github_oauth2(
    "your-client-id".into(),
    "your-client-secret".into(),
    "http://localhost:3000/callback".into(),
);

let mut client = OAuth2Client::new(config);

// Generate the authorization URL
let auth_url = client.authorization_url("random-csrf-token");
// Redirect the user to auth_url

// After the user authorizes, exchange the code for a token
let token = client.exchange_code("authorization-code").await?;

// Use the token
client.set_token(token);
```

### Custom OAuth2 Configuration

```rust
use mtw_integrations::oauth2::{OAuth2Client, OAuth2Config};

let config = OAuth2Config {
    client_id: "your-client-id".into(),
    client_secret: "your-client-secret".into(),
    redirect_uri: "http://localhost:3000/callback".into(),
    auth_url: "https://provider.com/oauth/authorize".into(),
    token_url: "https://provider.com/oauth/token".into(),
    scopes: vec!["read".into(), "write".into()],
};

let client = OAuth2Client::new(config);
```

### OAuth2TokenResponse

```rust
pub struct OAuth2TokenResponse {
    pub access_token: String,
    pub token_type: String,           // "Bearer"
    pub expires_in: Option<u64>,      // Seconds until expiration
    pub refresh_token: Option<String>,
    pub scope: Option<String>,        // Granted scopes (space-separated)
}
```

---

## Custom Auth Providers

Implement the `MtwAuth` trait for custom authentication:

```rust
use mtw_auth::{MtwAuth, AuthToken, AuthClaims, Credentials};
use mtw_core::MtwError;
use async_trait::async_trait;

pub struct CustomAuth {
    // your fields
}

#[async_trait]
impl MtwAuth for CustomAuth {
    fn name(&self) -> &str { "custom" }

    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken, MtwError> {
        match credentials {
            Credentials::Basic { username, password } => {
                // Verify against your user store
                // Return an AuthToken on success
                todo!()
            }
            _ => Err(MtwError::Auth("unsupported credential type".into())),
        }
    }

    async fn validate(&self, token: &str) -> Result<AuthClaims, MtwError> {
        // Validate and extract claims from the token
        todo!()
    }

    async fn refresh(&self, token: &str) -> Result<AuthToken, MtwError> {
        // Issue a new token from a refresh token
        todo!()
    }
}
```

---

## Configuration Examples

### JWT + Channels with Auth

```toml
[[modules]]
name = "mtw-auth-jwt"
config = { secret = "${JWT_SECRET}" }

[[channels]]
name = "private.*"
auth = true
max_members = 50

[[channels]]
name = "public.*"
auth = false
```

### Client Authentication (React)

```tsx
<MtwProvider
  url="ws://localhost:8080/ws"
  auth={{ token: "eyJhbGciOiJIUzI1NiIs..." }}
>
  <App />
</MtwProvider>
```

### Client Authentication (JavaScript)

```typescript
const conn = new MtwConnection({
  url: 'ws://localhost:8080/ws',
  auth: {
    token: 'your-jwt-token',
    // or
    apiKey: 'your-api-key',
  },
});
```

---

## Next Steps

- [Server Guide](./server-guide.md) -- configure the server
- [Modules Guide](./modules-guide.md) -- create auth modules
- [Frontend Guide](./frontend-guide.md) -- client-side auth
