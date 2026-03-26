use async_trait::async_trait;
use mtw_core::MtwError;
use std::time::Duration;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Configuration for timeout checking.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Maximum total request duration.
    pub request_timeout: Duration,
    /// Connection timeout (informational -- actual enforcement is at reqwest level).
    pub connect_timeout: Option<Duration>,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            connect_timeout: None,
        }
    }
}

/// Pipeline stage that checks whether the response exceeded the configured timeout.
/// Note: Actual request-level timeout enforcement is done by reqwest. This stage provides
/// post-hoc duration checking and metadata enrichment.
pub struct TimeoutStage {
    config: TimeoutConfig,
}

impl TimeoutStage {
    pub fn new() -> Self {
        Self {
            config: TimeoutConfig::default(),
        }
    }

    pub fn with_config(config: TimeoutConfig) -> Self {
        Self { config }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            config: TimeoutConfig {
                request_timeout: timeout,
                connect_timeout: None,
            },
        }
    }
}

impl Default for TimeoutStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for TimeoutStage {
    fn name(&self) -> &str {
        "timeout"
    }

    fn priority(&self) -> i32 {
        1
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let duration = response.timing.duration;

        // Enrich metadata with timing info
        response.metadata.insert(
            "request_duration_ms".into(),
            serde_json::json!(duration.as_millis() as u64),
        );

        if duration > self.config.request_timeout {
            tracing::warn!(
                duration_ms = duration.as_millis() as u64,
                timeout_ms = self.config.request_timeout.as_millis() as u64,
                "request exceeded timeout threshold"
            );
            response.metadata.insert(
                "timeout_exceeded".into(),
                serde_json::json!(true),
            );
        }

        Ok(PipelineAction::Continue(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;
    use crate::response::ResponseTiming;
    use std::time::Instant;

    fn make_ctx() -> PipelineContext {
        PipelineContext::new(MtwRequest::get("http://example.com"))
    }

    #[tokio::test]
    async fn test_normal_request_passes() {
        let stage = TimeoutStage::with_timeout(Duration::from_secs(30));
        let mut resp = MtwResponse::new(200);
        resp.timing = ResponseTiming {
            started_at: Instant::now(),
            duration: Duration::from_millis(100),
            dns_time: None,
            connect_time: None,
        };
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert_eq!(
                resp.metadata.get("request_duration_ms"),
                Some(&serde_json::json!(100u64))
            );
            assert!(resp.metadata.get("timeout_exceeded").is_none());
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_slow_request_flagged() {
        let stage = TimeoutStage::with_timeout(Duration::from_millis(50));
        let mut resp = MtwResponse::new(200);
        resp.timing = ResponseTiming {
            started_at: Instant::now(),
            duration: Duration::from_millis(100),
            dns_time: None,
            connect_time: None,
        };
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert_eq!(
                resp.metadata.get("timeout_exceeded"),
                Some(&serde_json::json!(true))
            );
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_duration_in_metadata() {
        let stage = TimeoutStage::new();
        let mut resp = MtwResponse::new(200);
        resp.timing = ResponseTiming {
            started_at: Instant::now(),
            duration: Duration::from_millis(250),
            dns_time: None,
            connect_time: None,
        };
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert_eq!(
                resp.metadata.get("request_duration_ms"),
                Some(&serde_json::json!(250u64))
            );
        } else {
            panic!("expected Continue");
        }
    }
}
