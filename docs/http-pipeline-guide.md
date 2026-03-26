# HTTP Pipeline Guide

The `mtw-http` crate provides an HTTP client with a composable response pipeline. Pipeline stages process HTTP responses in order, enabling patterns like retry, caching, rate limit handling, auth refresh, and more.

Source: `crates/mtw-http/`

---

## What is the Response Pipeline?

The response pipeline is a chain of stages that process an HTTP response after it is received. Each stage can:

- **Continue** -- pass the (possibly modified) response to the next stage
- **Retry** -- trigger a request retry (e.g., after auth token refresh)
- **Error** -- abort the pipeline with an error
- **Cached** -- return a cached response, skipping remaining stages

Stages are sorted by priority (lower number = runs first), similar to the middleware chain.

---

## Core Types

### PipelineStage Trait

```rust
// crates/mtw-http/src/pipeline.rs

#[async_trait]
pub trait PipelineStage: Send + Sync {
    /// Stage name (for logging)
    fn name(&self) -> &str;

    /// Priority (lower = runs first). Default: 100
    fn priority(&self) -> i32 { 100 }

    /// Process the response
    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError>;
}
```

### PipelineAction

```rust
pub enum PipelineAction {
    Continue(MtwResponse),   // Pass to next stage
    Retry(MtwRequest),       // Retry the request
    Error(MtwError),         // Abort with error
    Cached(MtwResponse),     // Return cached response
}
```

### PipelineContext

```rust
pub struct PipelineContext {
    pub request: MtwRequest,                       // The original request
    pub attempt: u32,                              // Current attempt number
    pub max_retries: u32,                          // Max retries (default: 3)
    pub metadata: HashMap<String, serde_json::Value>,  // Shared state between stages
}
```

### ResponsePipeline

```rust
let mut pipeline = ResponsePipeline::new();
pipeline.add_stage(Arc::new(MyStage));  // Auto-sorted by priority
pipeline.add_stage(Arc::new(AnotherStage));

let response = MtwResponse::new(200);
let mut ctx = PipelineContext::new(request);
let result = pipeline.execute(response, &mut ctx).await?;
```

---

## MtwRequest

The HTTP request builder (`crates/mtw-http/src/request.rs`):

```rust
use mtw_http::request::MtwRequest;
use std::time::Duration;

let req = MtwRequest::get("https://api.example.com/users")
    .header("accept", "application/json")
    .header("x-api-key", "secret")
    .query("page", "1")
    .query("per_page", "50")
    .timeout(Duration::from_secs(30))
    .meta("source", json!("api-client"));

let req = MtwRequest::post("https://api.example.com/users")
    .json(json!({ "name": "Alice", "email": "alice@example.com" }));

let req = MtwRequest::put("https://api.example.com/users/123")
    .body("plain text body");

let req = MtwRequest::delete("https://api.example.com/users/123");
```

### HTTP Methods

`Get`, `Post`, `Put`, `Patch`, `Delete`, `Head`, `Options`

### Body Types

```rust
pub enum Body {
    Empty,
    Text(String),
    Json(serde_json::Value),
    Bytes(Bytes),
}

// Automatic conversions via From trait:
let body: Body = "hello".into();
let body: Body = String::from("hello").into();
let body: Body = json!({"key": "value"}).into();
let body: Body = vec![1u8, 2, 3].into();
```

---

## MtwResponse

The enhanced HTTP response (`crates/mtw-http/src/response.rs`):

```rust
pub struct MtwResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: ResponseBody,
    pub timing: ResponseTiming,
    pub metadata: HashMap<String, Value>,
    pub rate_limit: Option<RateLimitInfo>,
    pub pagination: Option<PaginationInfo>,
    pub cache_info: Option<CacheInfo>,
}
```

### Reading the Response

```rust
// Deserialize JSON body
let users: Vec<User> = response.json()?;

// Raw text
let text: &str = response.text()?;

// Raw bytes
let bytes: &[u8] = response.bytes();

// Status checks
response.is_success()       // 2xx
response.is_client_error()  // 4xx
response.is_server_error()  // 5xx
```

### ResponseBody

```rust
pub enum ResponseBody {
    Bytes(Bytes),       // Raw bytes
    Json(Value),        // Pre-parsed JSON (set by JsonParseStage)
    Empty,              // No body
}
```

### Extracted Metadata

Pipeline stages can extract and attach structured metadata:

```rust
// Rate limit info (from X-RateLimit-* headers)
pub struct RateLimitInfo {
    pub limit: Option<u32>,
    pub remaining: Option<u32>,
    pub reset_at: Option<u64>,
    pub retry_after: Option<u64>,
}

// Pagination info (from Link headers or body)
pub struct PaginationInfo {
    pub next: Option<String>,
    pub prev: Option<String>,
    pub total: Option<u64>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub has_more: bool,
}

// Cache info (from Cache-Control, ETag, Last-Modified)
pub struct CacheInfo {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub cache_control: Option<String>,
    pub max_age: Option<u64>,
    pub is_cached: bool,
}

// Timing info
pub struct ResponseTiming {
    pub started_at: Instant,
    pub duration: Duration,
    pub dns_time: Option<Duration>,
    pub connect_time: Option<Duration>,
}
```

---

## AuthStrategy

HTTP request authentication (`crates/mtw-http/src/auth.rs`):

```rust
pub enum AuthStrategy {
    Bearer(String),
    Basic { username: String, password: String },
    ApiKey { header: String, value: String },
    OAuth2 { token: String, refresh_url: String, client_id: String, client_secret: String },
    Custom(Box<dyn Fn(&mut reqwest::RequestBuilder) + Send + Sync>),
}

// Usage
let auth = AuthStrategy::Bearer("my-token".into());
let auth = AuthStrategy::ApiKey {
    header: "X-API-Key".into(),
    value: "secret".into(),
};
```

The `AuthStrategy::apply()` method adds authentication to a `reqwest::RequestBuilder`. Debug output redacts secrets automatically.

---

## Built-in Pipeline Stages

The following stages are designed for the pipeline system. Implement them by creating structs that implement `PipelineStage`:

| Stage | Priority | Description |
|-------|----------|-------------|
| AuthRefresh | 5 | Refresh OAuth2 token on 401 and retry |
| RateLimitHandler | 10 | Wait and retry on 429 (rate limited) |
| RetryHandler | 15 | Retry on 5xx errors with exponential backoff |
| CircuitBreaker | 20 | Stop requests to failing endpoints |
| CacheCheck | 25 | Return cached responses (ETag, Last-Modified) |
| JsonParse | 50 | Parse JSON body into `ResponseBody::Json` |
| RateLimitExtract | 60 | Extract X-RateLimit-* headers |
| PaginationExtract | 65 | Extract pagination from Link headers |
| CacheExtract | 70 | Extract Cache-Control, ETag headers |
| TimingRecord | 80 | Record request duration |
| ErrorTransform | 90 | Convert HTTP errors to MtwError |
| MetadataEnrich | 95 | Add custom metadata to response |
| LoggingStage | 100 | Log request/response details |
| MetricsStage | 105 | Record metrics (latency, status codes) |
| ValidationStage | 110 | Validate response schema |
| TransformStage | 120 | Transform response body |

---

## Creating a Custom Pipeline Stage

```rust
use async_trait::async_trait;
use mtw_http::pipeline::{PipelineStage, PipelineAction, PipelineContext};
use mtw_http::response::MtwResponse;
use mtw_core::MtwError;

struct RetryOn5xx {
    max_retries: u32,
}

#[async_trait]
impl PipelineStage for RetryOn5xx {
    fn name(&self) -> &str { "retry-5xx" }
    fn priority(&self) -> i32 { 15 }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        if response.is_server_error() && context.attempt < self.max_retries {
            context.attempt += 1;
            tracing::warn!(
                status = response.status,
                attempt = context.attempt,
                "retrying after server error"
            );
            return Ok(PipelineAction::Retry(context.request.clone()));
        }
        Ok(PipelineAction::Continue(response))
    }
}
```

---

## Pipeline Stage Ordering (Priority System)

Stages are sorted by priority (ascending) when added to the pipeline. Lower priority numbers run first:

```
Request
  |
  v
[AuthRefresh (5)] --> [RateLimitHandler (10)] --> [RetryHandler (15)]
  |
  v
[CircuitBreaker (20)] --> [CacheCheck (25)] --> [JsonParse (50)]
  |
  v
[RateLimitExtract (60)] --> [PaginationExtract (65)] --> [CacheExtract (70)]
  |
  v
[TimingRecord (80)] --> [ErrorTransform (90)] --> [Logging (100)]
  |
  v
Response
```

If any stage returns `Retry`, `Error`, or `Cached`, the remaining stages are skipped.

---

## Common Patterns

### API Client with Retry + Auth Refresh

```rust
let mut pipeline = ResponsePipeline::new();
pipeline.add_stage(Arc::new(AuthRefreshStage::new(oauth2_client)));
pipeline.add_stage(Arc::new(RetryOn5xx { max_retries: 3 }));
pipeline.add_stage(Arc::new(JsonParseStage));
pipeline.add_stage(Arc::new(LoggingStage));

let req = MtwRequest::get("https://api.example.com/data")
    .header("authorization", "Bearer expired-token");
let mut ctx = PipelineContext::new(req);

// If 401: AuthRefreshStage refreshes the token and returns Retry
// If 5xx: RetryOn5xx retries up to 3 times
// Otherwise: JsonParseStage parses and LoggingStage logs
let result = pipeline.execute(initial_response, &mut ctx).await?;
```

### Cached API with Rate Limit Awareness

```rust
let mut pipeline = ResponsePipeline::new();
pipeline.add_stage(Arc::new(CacheCheckStage::new(cache)));
pipeline.add_stage(Arc::new(RateLimitHandlerStage));
pipeline.add_stage(Arc::new(RateLimitExtractStage));
pipeline.add_stage(Arc::new(CacheExtractStage));

// CacheCheckStage returns Cached if we have a valid cached response
// RateLimitHandlerStage waits and retries on 429
// Extract stages populate response.rate_limit and response.cache_info
```

### Paginated Data Fetching

```rust
use mtw_http::paginator::Paginator;

let paginator = Paginator::new(client, pipeline);

// Automatically follows pagination links
let all_items: Vec<Item> = paginator
    .fetch_all(MtwRequest::get("https://api.example.com/items"))
    .await?;
```

### Circuit Breaker for Microservices

```rust
struct CircuitBreakerStage {
    failure_threshold: u32,
    reset_timeout_secs: u64,
    // track failure counts per endpoint
}

#[async_trait]
impl PipelineStage for CircuitBreakerStage {
    fn name(&self) -> &str { "circuit-breaker" }
    fn priority(&self) -> i32 { 20 }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        if self.is_circuit_open(&context.request.url) {
            return Ok(PipelineAction::Error(
                MtwError::Internal("circuit breaker open".into())
            ));
        }

        if response.is_server_error() {
            self.record_failure(&context.request.url);
        } else {
            self.record_success(&context.request.url);
        }

        Ok(PipelineAction::Continue(response))
    }
}
```

---

## Next Steps

- [Integrations Guide](./integrations-guide.md) -- use the HTTP pipeline with API integrations
- [API Reference](./api-reference.md) -- full type documentation
