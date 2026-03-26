use async_trait::async_trait;
use mtw_core::MtwError;
use std::time::Duration;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::{MtwResponse, RateLimitInfo};

/// Pipeline stage that extracts rate limit info and handles 429 responses.
pub struct RateLimitStage;

impl RateLimitStage {
    pub fn new() -> Self {
        Self
    }

    /// Extract rate limit info from response headers.
    /// Supports both X-RateLimit-* and IETF RateLimit-* header formats.
    fn extract_rate_limit(headers: &std::collections::HashMap<String, String>) -> RateLimitInfo {
        let get_u32 = |keys: &[&str]| -> Option<u32> {
            for key in keys {
                if let Some(val) = headers.get(*key) {
                    if let Ok(n) = val.parse() {
                        return Some(n);
                    }
                }
            }
            None
        };

        let get_u64 = |keys: &[&str]| -> Option<u64> {
            for key in keys {
                if let Some(val) = headers.get(*key) {
                    if let Ok(n) = val.parse() {
                        return Some(n);
                    }
                }
            }
            None
        };

        RateLimitInfo {
            limit: get_u32(&["x-ratelimit-limit", "ratelimit-limit"]),
            remaining: get_u32(&["x-ratelimit-remaining", "ratelimit-remaining"]),
            reset_at: get_u64(&["x-ratelimit-reset", "ratelimit-reset"]),
            retry_after: get_u64(&["retry-after"]),
        }
    }
}

impl Default for RateLimitStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for RateLimitStage {
    fn name(&self) -> &str {
        "rate_limit"
    }

    fn priority(&self) -> i32 {
        12
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let rate_limit = Self::extract_rate_limit(&response.headers);

        // Always populate rate limit info if any headers are present
        if rate_limit.limit.is_some()
            || rate_limit.remaining.is_some()
            || rate_limit.reset_at.is_some()
            || rate_limit.retry_after.is_some()
        {
            response.rate_limit = Some(rate_limit.clone());
        }

        // Handle 429 Too Many Requests
        if response.status == 429 && context.attempt <= context.max_retries {
            let wait_secs = rate_limit
                .retry_after
                .or(rate_limit.reset_at.map(|reset| {
                    // reset_at is typically a Unix timestamp
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    reset.saturating_sub(now)
                }))
                .unwrap_or(1);

            tracing::warn!(
                remaining = rate_limit.remaining,
                wait_secs,
                "rate limited, waiting before retry"
            );

            tokio::time::sleep(Duration::from_secs(wait_secs.min(60))).await;

            return Ok(PipelineAction::Retry(context.request.clone()));
        }

        Ok(PipelineAction::Continue(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;

    fn make_ctx() -> PipelineContext {
        PipelineContext::new(MtwRequest::get("http://example.com"))
    }

    #[tokio::test]
    async fn test_extracts_rate_limit_info() {
        let stage = RateLimitStage::new();
        let resp = MtwResponse::new(200)
            .with_header("x-ratelimit-limit", "100")
            .with_header("x-ratelimit-remaining", "42")
            .with_header("x-ratelimit-reset", "1700000000");
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let rl = resp.rate_limit.unwrap();
            assert_eq!(rl.limit, Some(100));
            assert_eq!(rl.remaining, Some(42));
            assert_eq!(rl.reset_at, Some(1700000000));
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_ietf_headers() {
        let stage = RateLimitStage::new();
        let resp = MtwResponse::new(200)
            .with_header("ratelimit-limit", "50")
            .with_header("ratelimit-remaining", "10");
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let rl = resp.rate_limit.unwrap();
            assert_eq!(rl.limit, Some(50));
            assert_eq!(rl.remaining, Some(10));
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_429_triggers_retry() {
        let stage = RateLimitStage::new();
        let resp = MtwResponse::new(429).with_header("retry-after", "1");
        let mut ctx = make_ctx();
        ctx.attempt = 1;

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Retry(_)));
    }

    #[tokio::test]
    async fn test_no_rate_limit_headers() {
        let stage = RateLimitStage::new();
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert!(resp.rate_limit.is_none());
        } else {
            panic!("expected Continue");
        }
    }
}
