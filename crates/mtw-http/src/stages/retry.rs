use async_trait::async_trait;
use mtw_core::MtwError;
use std::collections::HashSet;
use std::time::Duration;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Configuration for the retry stage.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries.
    pub max_retries: u32,
    /// Initial delay before first retry.
    pub initial_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Backoff multiplier (default: 2.0).
    pub backoff_factor: f64,
    /// Status codes that should trigger a retry.
    pub retryable_codes: HashSet<u16>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            backoff_factor: 2.0,
            retryable_codes: [429, 500, 502, 503, 504].into_iter().collect(),
        }
    }
}

/// Pipeline stage that implements retry with exponential backoff.
pub struct RetryStage {
    config: RetryConfig,
}

impl RetryStage {
    pub fn new() -> Self {
        Self {
            config: RetryConfig::default(),
        }
    }

    pub fn with_config(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Calculate the delay for a given attempt using exponential backoff with jitter.
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let base = self.config.initial_delay.as_millis() as f64
            * self.config.backoff_factor.powi(attempt.saturating_sub(1) as i32);
        let capped = base.min(self.config.max_delay.as_millis() as f64);
        // Simple jitter: 75%-100% of calculated delay
        let jitter_factor = 0.75 + (attempt as f64 * 0.07 % 0.25);
        Duration::from_millis((capped * jitter_factor) as u64)
    }

    /// Parse the Retry-After header value (seconds or HTTP-date).
    fn parse_retry_after(headers: &std::collections::HashMap<String, String>) -> Option<Duration> {
        let value = headers.get("retry-after")?;
        // Try parsing as seconds first
        if let Ok(seconds) = value.parse::<u64>() {
            return Some(Duration::from_secs(seconds));
        }
        // Could parse HTTP-date here, but for simplicity return None
        None
    }
}

impl Default for RetryStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for RetryStage {
    fn name(&self) -> &str {
        "retry"
    }

    fn priority(&self) -> i32 {
        15
    }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        if !self.config.retryable_codes.contains(&response.status) {
            return Ok(PipelineAction::Continue(response));
        }

        if context.attempt > self.config.max_retries {
            tracing::warn!(
                status = response.status,
                attempts = context.attempt,
                "max retries exceeded"
            );
            return Ok(PipelineAction::Continue(response));
        }

        // Determine delay
        let delay = Self::parse_retry_after(&response.headers)
            .unwrap_or_else(|| self.calculate_delay(context.attempt));

        tracing::info!(
            status = response.status,
            attempt = context.attempt,
            delay_ms = delay.as_millis() as u64,
            "retrying request"
        );

        tokio::time::sleep(delay).await;

        Ok(PipelineAction::Retry(context.request.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;

    fn make_ctx() -> PipelineContext {
        let mut ctx = PipelineContext::new(MtwRequest::get("http://example.com"));
        ctx.max_retries = 3;
        ctx
    }

    #[tokio::test]
    async fn test_non_retryable_passes_through() {
        let stage = RetryStage::new();
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(r) if r.status == 200));
    }

    #[tokio::test]
    async fn test_retryable_triggers_retry() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(1), // minimal delay for tests
            max_delay: Duration::from_millis(10),
            ..Default::default()
        };
        let stage = RetryStage::with_config(config);
        let resp = MtwResponse::new(503);
        let mut ctx = make_ctx();
        ctx.attempt = 1;

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Retry(_)));
    }

    #[tokio::test]
    async fn test_max_retries_exceeded_passes_through() {
        let stage = RetryStage::new();
        let resp = MtwResponse::new(503);
        let mut ctx = make_ctx();
        ctx.attempt = 4; // exceed max_retries of 3

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(r) if r.status == 503));
    }

    #[tokio::test]
    async fn test_retry_after_header_respected() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            ..Default::default()
        };
        let stage = RetryStage::with_config(config);
        let resp = MtwResponse::new(429).with_header("retry-after", "1");
        let mut ctx = make_ctx();
        ctx.attempt = 1;

        let start = std::time::Instant::now();
        let result = stage.process(resp, &mut ctx).await.unwrap();
        let elapsed = start.elapsed();

        assert!(matches!(result, PipelineAction::Retry(_)));
        // Should have waited ~1 second for Retry-After header
        assert!(elapsed >= Duration::from_millis(900));
    }

    #[test]
    fn test_backoff_calculation() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_factor: 2.0,
            ..Default::default()
        };
        let stage = RetryStage::with_config(config);

        let d1 = stage.calculate_delay(1);
        let d2 = stage.calculate_delay(2);
        let d3 = stage.calculate_delay(3);

        // Each delay should be larger than the previous (with jitter)
        assert!(d2 > d1);
        assert!(d3 > d2);
    }
}
