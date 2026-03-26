# Integrations Guide

The `mtw-integrations` crate provides ready-to-use connectors for 20 third-party APIs, 10 AI model providers, OAuth2 support for 12 services, and an RSS feed reader.

Source: `crates/mtw-integrations/`

---

## Overview

### 20 API Integrations

| Category | Services |
|----------|----------|
| **Code & DevOps** | GitHub, GitLab, Jira, Linear, Vercel, Cloudflare, Docker Hub |
| **Communication** | Slack, Discord, Telegram, Twilio, SendGrid |
| **Payments** | Stripe, PayPal |
| **Storage & BaaS** | AWS S3, Google Cloud Storage, Firebase, Supabase |
| **Productivity** | Notion, Airtable |

Source: `crates/mtw-integrations/src/apis/`

### 10 AI Model Providers

| Provider | Module | Notable Models |
|----------|--------|----------------|
| Anthropic | `ai::anthropic` | Claude Opus 4, Sonnet 4 |
| OpenAI | `ai::openai` | GPT-4o, GPT-4 Turbo |
| Google | `ai::google` | Gemini Pro, Gemini Ultra |
| Mistral | `ai::mistral` | Mistral Large, Mixtral |
| Cohere | `ai::cohere` | Command R+ |
| DeepSeek | `ai::deepseek` | DeepSeek V2 |
| Meta | `ai::meta` | Llama 3 |
| xAI | `ai::xai` | Grok |
| HuggingFace | `ai::huggingface` | Various open models |
| Ollama | `ai::ollama` | Any local model |

Source: `crates/mtw-integrations/src/ai/`

### 12 OAuth2 Providers

GitHub, GitLab, Slack, Discord, Stripe, PayPal, Google, Notion, Airtable, Jira, Linear, Vercel

Source: `crates/mtw-integrations/src/oauth2.rs`

---

## Using an API Integration (GitHub Example)

Each integration provides a config struct, an `IntegrationInfo` constant, and a client stub:

```rust
use mtw_integrations::apis::github::{GitHubConfig, GitHubClient, GITHUB_INFO};

// Check integration info
println!("API: {}", GITHUB_INFO.name);       // "GitHub"
println!("Base URL: {}", GITHUB_INFO.base_url); // "https://api.github.com"
println!("Docs: {}", GITHUB_INFO.docs_url);
println!("OAuth2: {}", GITHUB_INFO.oauth2_supported); // true

// Configure the client
let config = GitHubConfig {
    api_token: std::env::var("GITHUB_TOKEN").unwrap(),
    base_url: None,  // Use default
    // ... other config options
};

let client = GitHubClient::new(config);

// Use the client (actual HTTP calls are planned for future phases)
// let repos = client.list_repos("fastslack").await?;
```

### Integration Metadata

Every integration exposes an `IntegrationInfo` struct:

```rust
pub struct IntegrationInfo {
    pub id: &'static str,           // "github"
    pub name: &'static str,         // "GitHub"
    pub base_url: &'static str,     // "https://api.github.com"
    pub docs_url: &'static str,     // "https://docs.github.com/en/rest"
    pub oauth2_supported: bool,     // true
}
```

### Integration Status

Health checks return:

```rust
pub enum IntegrationStatus {
    Connected,
    Disconnected,
    Error(String),
}
```

---

## OAuth2 Setup

### Pre-configured Providers

```rust
use mtw_integrations::oauth2::*;

// GitHub
let config = github_oauth2(
    "your-client-id".into(),
    "your-client-secret".into(),
    "http://localhost:3000/callback".into(),
);

// Google
let config = google_oauth2(
    "your-client-id".into(),
    "your-client-secret".into(),
    "http://localhost:3000/callback".into(),
);

// Slack
let config = slack_oauth2("id".into(), "secret".into(), "http://localhost:3000/cb".into());

// All 12 available:
// github_oauth2, gitlab_oauth2, slack_oauth2, discord_oauth2,
// stripe_oauth2, paypal_oauth2, google_oauth2, notion_oauth2,
// airtable_oauth2, jira_oauth2, linear_oauth2, vercel_oauth2
```

### OAuth2 Flow

```rust
use mtw_integrations::oauth2::{OAuth2Client, github_oauth2};

// 1. Create client
let config = github_oauth2("client-id".into(), "secret".into(), "http://localhost:3000/cb".into());
let mut client = OAuth2Client::new(config);

// 2. Generate authorization URL
let state = "random-csrf-token-123";
let auth_url = client.authorization_url(state);
// Redirect user to auth_url

// 3. After user authorizes, exchange the code
let token = client.exchange_code("authorization-code-from-callback").await?;

// 4. Store the token
client.set_token(token);

// 5. Refresh when expired
let new_token = client.refresh_token("refresh-token-value").await?;
client.set_token(new_token);
```

### OAuth2Config Structure

```rust
pub struct OAuth2Config {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub auth_url: String,         // Authorization endpoint
    pub token_url: String,        // Token endpoint
    pub scopes: Vec<String>,      // Requested scopes
}
```

### Pre-configured Scopes

| Provider | Default Scopes |
|----------|---------------|
| GitHub | `repo`, `user` |
| GitLab | `api`, `read_user` |
| Slack | `chat:write`, `channels:read` |
| Discord | `bot`, `identify` |
| Stripe | `read_write` |
| PayPal | `openid` |
| Google | `devstorage.read_write` |
| Notion | (none) |
| Airtable | `data.records:read`, `data.records:write` |
| Jira | `read:jira-work`, `write:jira-work` |
| Linear | `read`, `write` |
| Vercel | (none) |

---

## AI Model Provider Configuration

Each AI provider has an `AiProviderConfig` and model constants:

```rust
use mtw_integrations::ai::{AiProviderConfig, AiProviderInfo, ModelInfo};

// Common config for all providers
let config = AiProviderConfig {
    api_key: std::env::var("API_KEY").unwrap(),
    base_url: None,                   // Override default URL
    default_model: Some("claude-sonnet-4-6".into()),
    default_temperature: Some(0.7),
    default_max_tokens: Some(4096),
    timeout_secs: 120,                // Request timeout
};
```

### Provider-specific Configuration

```toml
# mtw.toml

# Anthropic
[[modules]]
name = "mtw-ai-anthropic"
config = {
    api_key = "${ANTHROPIC_API_KEY}",
    default_model = "claude-sonnet-4-6",
    timeout_secs = 120
}

# OpenAI
[[modules]]
name = "mtw-ai-openai"
config = {
    api_key = "${OPENAI_API_KEY}",
    default_model = "gpt-4o",
    default_temperature = 0.7
}

# Ollama (local)
[[modules]]
name = "mtw-ai-ollama"
config = {
    api_key = "",
    base_url = "http://localhost:11434",
    default_model = "llama3"
}

# Google Gemini
[[modules]]
name = "mtw-ai-google"
config = {
    api_key = "${GOOGLE_AI_KEY}",
    default_model = "gemini-pro"
}
```

### AiProviderInfo

```rust
pub struct AiProviderInfo {
    pub id: &'static str,              // "anthropic"
    pub name: &'static str,            // "Anthropic"
    pub base_url: &'static str,        // "https://api.anthropic.com"
    pub docs_url: &'static str,        // "https://docs.anthropic.com"
    pub supports_streaming: bool,
    pub supports_tool_calling: bool,
    pub supports_vision: bool,
    pub supports_embeddings: bool,
}
```

### ModelInfo

```rust
pub struct ModelInfo {
    pub id: &'static str,              // "claude-sonnet-4-20250514"
    pub name: &'static str,            // "Claude Sonnet 4"
    pub context_window: usize,         // 200000
    pub max_output_tokens: usize,      // 8192
    pub supports_vision: bool,
    pub supports_tool_calling: bool,
}
```

---

## RSS Feed Reader

```rust
use mtw_integrations::rss::{RssConfig, RssFeed, RssItem};

let config = RssConfig {
    url: "https://example.com/feed.xml".into(),
    // ... config options
};

// RssFeed and RssItem types for parsed feeds
```

Source: `crates/mtw-integrations/src/rss.rs`

---

## Creating Custom Integrations

To create a custom integration module:

1. Create a config struct:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyApiConfig {
    pub api_key: String,
    pub base_url: Option<String>,
}
```

2. Define integration info:

```rust
use mtw_integrations::apis::IntegrationInfo;

pub const MY_API_INFO: IntegrationInfo = IntegrationInfo {
    id: "my-api",
    name: "My API",
    base_url: "https://api.myservice.com",
    docs_url: "https://docs.myservice.com",
    oauth2_supported: false,
};
```

3. Create the client:

```rust
pub struct MyApiClient {
    config: MyApiConfig,
    http: reqwest::Client,
}

impl MyApiClient {
    pub fn new(config: MyApiConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub async fn get_data(&self) -> Result<serde_json::Value, mtw_core::MtwError> {
        let base = self.config.base_url.as_deref()
            .unwrap_or(MY_API_INFO.base_url);

        let resp = self.http.get(format!("{}/data", base))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await
            .map_err(|e| mtw_core::MtwError::Internal(e.to_string()))?;

        resp.json()
            .await
            .map_err(|e| mtw_core::MtwError::Internal(e.to_string()))
    }
}
```

4. Optionally, package as an mtwRequest module with `MtwModule` implementation and a manifest.

---

## Using Integrations with the HTTP Pipeline

Combine integrations with `mtw-http` pipeline stages:

```rust
use mtw_http::{MtwRequest, ResponsePipeline, PipelineContext};

let mut pipeline = ResponsePipeline::new();
// Add retry, rate-limit, and cache stages
pipeline.add_stage(Arc::new(retry_stage));
pipeline.add_stage(Arc::new(rate_limit_stage));

let req = MtwRequest::get("https://api.github.com/repos/fastslack/mtw-request")
    .header("authorization", format!("Bearer {}", github_token))
    .header("accept", "application/vnd.github.v3+json");

let mut ctx = PipelineContext::new(req);
// Execute through the pipeline...
```

---

## Next Steps

- [HTTP Pipeline Guide](./http-pipeline-guide.md) -- response pipeline details
- [AI Agents Guide](./ai-agents-guide.md) -- use AI providers with agents
- [Auth Guide](./auth-guide.md) -- OAuth2 for authentication
