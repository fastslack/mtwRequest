use async_trait::async_trait;
use mtw_core::MtwError;
use std::collections::HashSet;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Configuration for status code checking.
#[derive(Debug, Clone)]
pub struct StatusCheckConfig {
    /// Status codes that should be treated as errors (default: 400-599).
    pub error_codes: HashSet<u16>,
    /// Status codes that are retryable (default: 429, 500, 502, 503, 504).
    pub retryable_codes: HashSet<u16>,
    /// If true, non-retryable error codes immediately abort the pipeline.
    pub fail_fast: bool,
}

impl Default for StatusCheckConfig {
    fn default() -> Self {
        let error_codes: HashSet<u16> = (400..600).collect();
        let retryable_codes: HashSet<u16> = [429, 500, 502, 503, 504].into_iter().collect();
        Self {
            error_codes,
            retryable_codes,
            fail_fast: true,
        }
    }
}

/// Pipeline stage that checks HTTP status codes and maps them to errors or retries.
pub struct StatusCheckStage {
    config: StatusCheckConfig,
}

impl StatusCheckStage {
    pub fn new() -> Self {
        Self {
            config: StatusCheckConfig::default(),
        }
    }

    pub fn with_config(config: StatusCheckConfig) -> Self {
        Self { config }
    }
}

impl Default for StatusCheckStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for StatusCheckStage {
    fn name(&self) -> &str {
        "status_check"
    }

    fn priority(&self) -> i32 {
        10
    }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let status = response.status;

        if !self.config.error_codes.contains(&status) {
            return Ok(PipelineAction::Continue(response));
        }

        // Check if this is a retryable status code
        if self.config.retryable_codes.contains(&status)
            && context.attempt <= context.max_retries
        {
            tracing::warn!(status, attempt = context.attempt, "retryable status code");
            return Ok(PipelineAction::Retry(context.request.clone()));
        }

        if self.config.fail_fast {
            let error_msg = if response.is_client_error() {
                format!("HTTP client error: {}", status)
            } else {
                format!("HTTP server error: {}", status)
            };
            return Ok(PipelineAction::Error(MtwError::Transport(error_msg)));
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
    async fn test_success_passes_through() {
        let stage = StatusCheckStage::new();
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(r) if r.status == 200));
    }

    #[tokio::test]
    async fn test_client_error_returns_error() {
        let stage = StatusCheckStage::new();
        let resp = MtwResponse::new(404);
        let mut ctx = make_ctx();
        ctx.attempt = 4; // past max retries

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Error(_)));
    }

    #[tokio::test]
    async fn test_retryable_code_triggers_retry() {
        let stage = StatusCheckStage::new();
        let resp = MtwResponse::new(503);
        let mut ctx = make_ctx();
        ctx.attempt = 1;

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Retry(_)));
    }

    #[tokio::test]
    async fn test_429_triggers_retry() {
        let stage = StatusCheckStage::new();
        let resp = MtwResponse::new(429);
        let mut ctx = make_ctx();
        ctx.attempt = 1;

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Retry(_)));
    }

    #[tokio::test]
    async fn test_non_fail_fast_continues() {
        let config = StatusCheckConfig {
            fail_fast: false,
            ..Default::default()
        };
        let stage = StatusCheckStage::with_config(config);
        let resp = MtwResponse::new(404);
        let mut ctx = make_ctx();
        ctx.attempt = 4; // past retries

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(r) if r.status == 404));
    }
}
