use mtw_core::MtwError;
use reqwest::header::HeaderMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::auth::AuthStrategy;
use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage, ResponsePipeline};
use crate::request::{Body, MtwRequest};
use crate::response::{MtwResponse, ResponseBody, ResponseTiming};

/// A high-level HTTP client with response pipeline support.
pub struct MtwHttpClient {
    inner: reqwest::Client,
    pipeline: ResponsePipeline,
    base_url: Option<String>,
    default_headers: HeaderMap,
    auth: Option<AuthStrategy>,
    max_retries: u32,
}

impl MtwHttpClient {
    /// Create a new client with default settings.
    pub fn new() -> Self {
        Self {
            inner: reqwest::Client::new(),
            pipeline: ResponsePipeline::new(),
            base_url: None,
            default_headers: HeaderMap::new(),
            auth: None,
            max_retries: 3,
        }
    }

    /// Get a builder for configuring the client.
    pub fn builder() -> MtwHttpClientBuilder {
        MtwHttpClientBuilder::new()
    }

    /// Perform a GET request.
    pub async fn get(&self, url: &str) -> Result<MtwResponse, MtwError> {
        self.request(MtwRequest::get(url)).await
    }

    /// Perform a POST request.
    pub async fn post(&self, url: &str, body: impl Into<Body>) -> Result<MtwResponse, MtwError> {
        let req = MtwRequest::post(url).body(body);
        self.request(req).await
    }

    /// Perform a PUT request.
    pub async fn put(&self, url: &str, body: impl Into<Body>) -> Result<MtwResponse, MtwError> {
        let req = MtwRequest::put(url).body(body);
        self.request(req).await
    }

    /// Perform a PATCH request.
    pub async fn patch(&self, url: &str, body: impl Into<Body>) -> Result<MtwResponse, MtwError> {
        let req = MtwRequest::patch(url).body(body);
        self.request(req).await
    }

    /// Perform a DELETE request.
    pub async fn delete(&self, url: &str) -> Result<MtwResponse, MtwError> {
        self.request(MtwRequest::delete(url)).await
    }

    /// Execute an MtwRequest, running it through the response pipeline.
    pub async fn request(&self, req: MtwRequest) -> Result<MtwResponse, MtwError> {
        let mut ctx = PipelineContext::new(req.clone());
        ctx.max_retries = self.max_retries;

        let mut current_req = req;
        loop {
            ctx.attempt += 1;
            if ctx.attempt > ctx.max_retries + 1 {
                return Err(MtwError::Transport(format!(
                    "max retries ({}) exceeded",
                    ctx.max_retries
                )));
            }

            let response = self.execute_request(&current_req).await?;
            let action = self.pipeline.execute(response, &mut ctx).await?;

            match action {
                PipelineAction::Continue(resp) => return Ok(resp),
                PipelineAction::Retry(retry_req) => {
                    tracing::debug!(attempt = ctx.attempt, "retrying request");
                    current_req = retry_req;
                }
                PipelineAction::Error(err) => return Err(err),
                PipelineAction::Cached(resp) => return Ok(resp),
            }
        }
    }

    /// Build and send the actual reqwest request.
    async fn execute_request(&self, req: &MtwRequest) -> Result<MtwResponse, MtwError> {
        let url = if let Some(base) = &self.base_url {
            if req.url.starts_with("http://") || req.url.starts_with("https://") {
                req.url.clone()
            } else {
                format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    req.url.trim_start_matches('/')
                )
            }
        } else {
            req.url.clone()
        };

        let started_at = Instant::now();

        let mut builder = self.inner.request(req.method.as_reqwest(), &url);

        // Default headers
        builder = builder.headers(self.default_headers.clone());

        // Request headers
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        // Query parameters
        if !req.query.is_empty() {
            let params: Vec<(&str, &str)> =
                req.query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            builder = builder.query(&params);
        }

        // Body
        if let Some(body) = &req.body {
            builder = match body {
                Body::Text(s) => builder.body(s.clone()),
                Body::Json(v) => builder.json(v),
                Body::Bytes(b) => builder.body(b.clone()),
                Body::Empty => builder,
            };
        }

        // Auth
        if let Some(auth) = &self.auth {
            builder = auth.apply(builder);
        }

        // Timeout
        if let Some(timeout) = req.timeout {
            builder = builder.timeout(timeout);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| MtwError::Transport(e.to_string()))?;

        let duration = started_at.elapsed();
        let status = resp.status().as_u16();

        let mut headers = std::collections::HashMap::new();
        for (k, v) in resp.headers() {
            if let Ok(val) = v.to_str() {
                headers.insert(k.as_str().to_string(), val.to_string());
            }
        }

        let body_bytes = resp
            .bytes()
            .await
            .map_err(|e| MtwError::Transport(e.to_string()))?;

        let body = if body_bytes.is_empty() {
            ResponseBody::Empty
        } else {
            ResponseBody::Bytes(body_bytes)
        };

        Ok(MtwResponse {
            status,
            headers,
            body,
            timing: ResponseTiming {
                started_at,
                duration,
                dns_time: None,
                connect_time: None,
            },
            metadata: std::collections::HashMap::new(),
            rate_limit: None,
            pagination: None,
            cache_info: None,
        })
    }
}

impl Default for MtwHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for configuring an MtwHttpClient.
pub struct MtwHttpClientBuilder {
    base_url: Option<String>,
    default_headers: HeaderMap,
    auth: Option<AuthStrategy>,
    timeout: Option<Duration>,
    pipeline: ResponsePipeline,
    max_retries: u32,
}

impl MtwHttpClientBuilder {
    pub fn new() -> Self {
        Self {
            base_url: None,
            default_headers: HeaderMap::new(),
            auth: None,
            timeout: None,
            pipeline: ResponsePipeline::new(),
            max_retries: 3,
        }
    }

    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            self.default_headers.insert(name, val);
        }
        self
    }

    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth = Some(AuthStrategy::Bearer(token.into()));
        self
    }

    pub fn auth(mut self, strategy: AuthStrategy) -> Self {
        self.auth = Some(strategy);
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn pipeline_stage(mut self, stage: Arc<dyn PipelineStage>) -> Self {
        self.pipeline.add_stage(stage);
        self
    }

    pub fn build(self) -> MtwHttpClient {
        let mut reqwest_builder = reqwest::Client::builder();
        if let Some(timeout) = self.timeout {
            reqwest_builder = reqwest_builder.timeout(timeout);
        }
        let inner = reqwest_builder.build().unwrap_or_else(|_| reqwest::Client::new());

        MtwHttpClient {
            inner,
            pipeline: self.pipeline,
            base_url: self.base_url,
            default_headers: self.default_headers,
            auth: self.auth,
            max_retries: self.max_retries,
        }
    }
}

impl Default for MtwHttpClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creates_client() {
        let client = MtwHttpClient::builder()
            .base_url("https://api.example.com")
            .header("x-api-version", "v2")
            .bearer_token("tok_123")
            .timeout(Duration::from_secs(30))
            .max_retries(5)
            .build();

        assert_eq!(client.base_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(client.max_retries, 5);
        assert!(client.auth.is_some());
    }

    #[test]
    fn test_default_client() {
        let client = MtwHttpClient::new();
        assert!(client.base_url.is_none());
        assert!(client.auth.is_none());
        assert_eq!(client.max_retries, 3);
        assert!(client.pipeline.is_empty());
    }
}
