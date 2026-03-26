use async_trait::async_trait;
use futures::future::BoxFuture;
use mtw_core::MtwError;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Type alias for the auth refresh function.
pub type RefreshFn = Arc<dyn Fn() -> BoxFuture<'static, Result<String, MtwError>> + Send + Sync>;

/// Configuration for auth token refresh.
pub struct AuthRefreshConfig {
    /// Function that refreshes the auth token and returns the new token.
    pub refresh_fn: RefreshFn,
    /// Header to update with the new token (default: "authorization").
    pub auth_header: String,
    /// Prefix for the token value (default: "Bearer ").
    pub token_prefix: String,
    /// Maximum number of refresh attempts.
    pub max_refresh_attempts: u32,
}

impl AuthRefreshConfig {
    pub fn new(refresh_fn: RefreshFn) -> Self {
        Self {
            refresh_fn,
            auth_header: "authorization".to_string(),
            token_prefix: "Bearer ".to_string(),
            max_refresh_attempts: 1,
        }
    }
}

/// Pipeline stage that automatically refreshes auth tokens on 401 responses.
pub struct AuthRefreshStage {
    config: AuthRefreshConfig,
    refresh_count: Arc<Mutex<u32>>,
}

impl AuthRefreshStage {
    pub fn new(config: AuthRefreshConfig) -> Self {
        Self {
            config,
            refresh_count: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl PipelineStage for AuthRefreshStage {
    fn name(&self) -> &str {
        "auth_refresh"
    }

    fn priority(&self) -> i32 {
        5
    }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        if response.status != 401 {
            // Reset refresh count on non-401
            *self.refresh_count.lock().await = 0;
            return Ok(PipelineAction::Continue(response));
        }

        let mut count = self.refresh_count.lock().await;
        if *count >= self.config.max_refresh_attempts {
            tracing::warn!("auth refresh attempts exceeded");
            return Ok(PipelineAction::Error(MtwError::Auth(
                "token refresh failed after max attempts".into(),
            )));
        }

        *count += 1;
        drop(count);

        tracing::info!("refreshing auth token");

        let new_token = (self.config.refresh_fn)().await?;

        let mut retry_req = context.request.clone();
        retry_req.headers.insert(
            self.config.auth_header.clone(),
            format!("{}{}", self.config.token_prefix, new_token),
        );

        Ok(PipelineAction::Retry(retry_req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;

    fn make_ctx() -> PipelineContext {
        PipelineContext::new(MtwRequest::get("http://example.com"))
    }

    fn make_refresh_fn(token: &str) -> RefreshFn {
        let token = token.to_string();
        Arc::new(move || {
            let token = token.clone();
            Box::pin(async move { Ok(token) })
        })
    }

    #[tokio::test]
    async fn test_non_401_passes_through() {
        let config = AuthRefreshConfig::new(make_refresh_fn("new_token"));
        let stage = AuthRefreshStage::new(config);
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(r) if r.status == 200));
    }

    #[tokio::test]
    async fn test_401_triggers_refresh_and_retry() {
        let config = AuthRefreshConfig::new(make_refresh_fn("fresh_token"));
        let stage = AuthRefreshStage::new(config);
        let resp = MtwResponse::new(401);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Retry(req) = result {
            assert_eq!(
                req.headers.get("authorization").unwrap(),
                "Bearer fresh_token"
            );
        } else {
            panic!("expected Retry");
        }
    }

    #[tokio::test]
    async fn test_max_refresh_attempts_exceeded() {
        let config = AuthRefreshConfig {
            refresh_fn: make_refresh_fn("token"),
            max_refresh_attempts: 1,
            ..AuthRefreshConfig::new(make_refresh_fn("token"))
        };
        let stage = AuthRefreshStage::new(config);
        let resp = MtwResponse::new(401);
        let mut ctx = make_ctx();

        // First attempt succeeds
        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Retry(_)));

        // Second attempt should fail
        let resp2 = MtwResponse::new(401);
        let result2 = stage.process(resp2, &mut ctx).await.unwrap();
        assert!(matches!(result2, PipelineAction::Error(_)));
    }
}
