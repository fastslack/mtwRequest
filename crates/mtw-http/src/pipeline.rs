use async_trait::async_trait;
use mtw_core::MtwError;
use std::collections::HashMap;
use std::sync::Arc;

use crate::request::MtwRequest;
use crate::response::MtwResponse;

/// Action returned by a pipeline stage.
#[derive(Debug)]
pub enum PipelineAction {
    /// Continue to the next stage with the (possibly modified) response.
    Continue(MtwResponse),
    /// Retry the request (e.g., after auth refresh or rate-limit wait).
    Retry(MtwRequest),
    /// Abort the pipeline with an error.
    Error(MtwError),
    /// Return a cached response, skipping remaining stages.
    Cached(MtwResponse),
}

/// Context passed through the pipeline, carrying request info and metadata.
#[derive(Debug, Clone)]
pub struct PipelineContext {
    pub request: MtwRequest,
    pub attempt: u32,
    pub max_retries: u32,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl PipelineContext {
    pub fn new(request: MtwRequest) -> Self {
        Self {
            request,
            attempt: 0,
            max_retries: 3,
            metadata: HashMap::new(),
        }
    }
}

/// A composable stage in the response pipeline.
#[async_trait]
pub trait PipelineStage: Send + Sync {
    /// The name of this stage (used for logging/debugging).
    fn name(&self) -> &str;

    /// Priority (lower = runs first). Default: 100.
    fn priority(&self) -> i32 {
        100
    }

    /// Process the response. Can modify it, trigger a retry, error out, or return cached data.
    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError>;
}

/// A chain of pipeline stages that process HTTP responses.
pub struct ResponsePipeline {
    stages: Vec<Arc<dyn PipelineStage>>,
}

impl ResponsePipeline {
    pub fn new() -> Self {
        Self {
            stages: Vec::new(),
        }
    }

    /// Add a stage and re-sort by priority.
    pub fn add_stage(&mut self, stage: Arc<dyn PipelineStage>) {
        self.stages.push(stage);
        self.stages.sort_by_key(|s| s.priority());
    }

    /// Execute all stages in order. Returns the final response or an error.
    pub async fn execute(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let mut current = response;

        for stage in &self.stages {
            match stage.process(current, context).await? {
                PipelineAction::Continue(resp) => {
                    current = resp;
                }
                action @ PipelineAction::Retry(_) => {
                    tracing::debug!(stage = %stage.name(), "pipeline stage requested retry");
                    return Ok(action);
                }
                action @ PipelineAction::Error(_) => {
                    tracing::debug!(stage = %stage.name(), "pipeline stage returned error");
                    return Ok(action);
                }
                action @ PipelineAction::Cached(_) => {
                    tracing::debug!(stage = %stage.name(), "pipeline stage returned cached response");
                    return Ok(action);
                }
            }
        }

        Ok(PipelineAction::Continue(current))
    }

    /// Number of stages.
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

impl Default for ResponsePipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ResponsePipeline {
    fn clone(&self) -> Self {
        Self {
            stages: self.stages.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AddMetaStage {
        key: String,
        value: serde_json::Value,
        priority: i32,
    }

    #[async_trait]
    impl PipelineStage for AddMetaStage {
        fn name(&self) -> &str {
            "add_meta"
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        async fn process(
            &self,
            mut response: MtwResponse,
            _context: &mut PipelineContext,
        ) -> Result<PipelineAction, MtwError> {
            response
                .metadata
                .insert(self.key.clone(), self.value.clone());
            Ok(PipelineAction::Continue(response))
        }
    }

    struct ErrorStage;

    #[async_trait]
    impl PipelineStage for ErrorStage {
        fn name(&self) -> &str {
            "error"
        }
        fn priority(&self) -> i32 {
            50
        }
        async fn process(
            &self,
            _response: MtwResponse,
            _context: &mut PipelineContext,
        ) -> Result<PipelineAction, MtwError> {
            Ok(PipelineAction::Error(MtwError::Internal(
                "stage error".into(),
            )))
        }
    }

    #[tokio::test]
    async fn test_pipeline_executes_stages_in_order() {
        let mut pipeline = ResponsePipeline::new();
        pipeline.add_stage(Arc::new(AddMetaStage {
            key: "first".into(),
            value: serde_json::json!(1),
            priority: 10,
        }));
        pipeline.add_stage(Arc::new(AddMetaStage {
            key: "second".into(),
            value: serde_json::json!(2),
            priority: 20,
        }));

        let resp = MtwResponse::new(200);
        let req = MtwRequest::get("http://example.com");
        let mut ctx = PipelineContext::new(req);

        let result = pipeline.execute(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert_eq!(resp.metadata.get("first"), Some(&serde_json::json!(1)));
            assert_eq!(resp.metadata.get("second"), Some(&serde_json::json!(2)));
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_pipeline_error_stops_execution() {
        let mut pipeline = ResponsePipeline::new();
        pipeline.add_stage(Arc::new(ErrorStage));
        pipeline.add_stage(Arc::new(AddMetaStage {
            key: "should_not_exist".into(),
            value: serde_json::json!(true),
            priority: 100,
        }));

        let resp = MtwResponse::new(200);
        let req = MtwRequest::get("http://example.com");
        let mut ctx = PipelineContext::new(req);

        let result = pipeline.execute(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Error(_)));
    }

    #[tokio::test]
    async fn test_pipeline_priority_ordering() {
        let mut pipeline = ResponsePipeline::new();
        // Add in reverse order; pipeline should sort by priority
        pipeline.add_stage(Arc::new(AddMetaStage {
            key: "high_priority".into(),
            value: serde_json::json!("first"),
            priority: 200,
        }));
        pipeline.add_stage(Arc::new(AddMetaStage {
            key: "low_priority".into(),
            value: serde_json::json!("second"),
            priority: 10,
        }));

        assert_eq!(pipeline.stages[0].priority(), 10);
        assert_eq!(pipeline.stages[1].priority(), 200);
    }

    #[test]
    fn test_empty_pipeline() {
        let pipeline = ResponsePipeline::new();
        assert!(pipeline.is_empty());
        assert_eq!(pipeline.len(), 0);
    }
}
