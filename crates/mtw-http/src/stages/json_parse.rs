use async_trait::async_trait;
use mtw_core::MtwError;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::{MtwResponse, ResponseBody};

/// Configuration for JSON parsing.
#[derive(Debug, Clone)]
pub struct JsonParseConfig {
    /// If true, error when Content-Type is application/json but body is invalid.
    /// If false, skip silently on parse failure.
    pub strict: bool,
}

impl Default for JsonParseConfig {
    fn default() -> Self {
        Self { strict: false }
    }
}

/// Pipeline stage that auto-parses JSON response bodies.
pub struct JsonParseStage {
    config: JsonParseConfig,
}

impl JsonParseStage {
    pub fn new() -> Self {
        Self {
            config: JsonParseConfig::default(),
        }
    }

    pub fn strict() -> Self {
        Self {
            config: JsonParseConfig { strict: true },
        }
    }

    pub fn with_config(config: JsonParseConfig) -> Self {
        Self { config }
    }

    fn is_json_content_type(headers: &std::collections::HashMap<String, String>) -> bool {
        headers
            .get("content-type")
            .map(|ct| ct.contains("application/json") || ct.contains("+json"))
            .unwrap_or(false)
    }
}

impl Default for JsonParseStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for JsonParseStage {
    fn name(&self) -> &str {
        "json_parse"
    }

    fn priority(&self) -> i32 {
        20
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        // Only parse if content-type indicates JSON
        if !Self::is_json_content_type(&response.headers) {
            return Ok(PipelineAction::Continue(response));
        }

        // Only parse if body is still raw bytes
        let bytes = match &response.body {
            ResponseBody::Bytes(b) => b.clone(),
            _ => return Ok(PipelineAction::Continue(response)),
        };

        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => {
                // Store parsed JSON in metadata and as body
                response
                    .metadata
                    .insert("json_body".into(), value.clone());
                response.body = ResponseBody::Json(value);
                Ok(PipelineAction::Continue(response))
            }
            Err(e) => {
                if self.config.strict {
                    Ok(PipelineAction::Error(MtwError::Codec(format!(
                        "JSON parse error: {}",
                        e
                    ))))
                } else {
                    tracing::warn!("failed to parse JSON body: {}", e);
                    Ok(PipelineAction::Continue(response))
                }
            }
        }
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
    async fn test_parses_json_body() {
        let stage = JsonParseStage::new();
        let resp = MtwResponse::new(200)
            .with_header("content-type", "application/json")
            .with_body(br#"{"key":"value"}"#.to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert!(matches!(resp.body, ResponseBody::Json(_)));
            let val: serde_json::Value = resp.json().unwrap();
            assert_eq!(val["key"], "value");
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_skips_non_json() {
        let stage = JsonParseStage::new();
        let resp = MtwResponse::new(200)
            .with_header("content-type", "text/html")
            .with_body(b"<html></html>".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert!(matches!(resp.body, ResponseBody::Bytes(_)));
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_strict_mode_errors_on_bad_json() {
        let stage = JsonParseStage::strict();
        let resp = MtwResponse::new(200)
            .with_header("content-type", "application/json")
            .with_body(b"not valid json".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Error(_)));
    }

    #[tokio::test]
    async fn test_lenient_mode_continues_on_bad_json() {
        let stage = JsonParseStage::new();
        let resp = MtwResponse::new(200)
            .with_header("content-type", "application/json")
            .with_body(b"not valid json".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
    }

    #[tokio::test]
    async fn test_parses_json_with_charset() {
        let stage = JsonParseStage::new();
        let resp = MtwResponse::new(200)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(br#"[1,2,3]"#.to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let val: Vec<i32> = resp.json().unwrap();
            assert_eq!(val, vec![1, 2, 3]);
        } else {
            panic!("expected Continue");
        }
    }
}
