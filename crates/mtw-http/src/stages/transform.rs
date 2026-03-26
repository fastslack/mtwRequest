use async_trait::async_trait;
use mtw_core::MtwError;
use std::sync::Arc;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::{MtwResponse, ResponseBody};

/// Type alias for transform functions.
pub type TransformFn =
    Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value, MtwError> + Send + Sync>;

/// Pipeline stage that applies a transformation function to JSON response bodies.
/// Useful for unwrapping nested data, renaming fields, filtering, etc.
pub struct TransformStage {
    name_str: String,
    transform_fn: TransformFn,
}

impl TransformStage {
    pub fn new(name: impl Into<String>, transform_fn: TransformFn) -> Self {
        Self {
            name_str: name.into(),
            transform_fn,
        }
    }

    /// Create a stage that unwraps a nested field from the response JSON.
    /// For example, `unwrap_field("data")` transforms `{"data": {...}, "meta": {...}}` into `{...}`.
    pub fn unwrap_field(field: impl Into<String>) -> Self {
        let field = field.into();
        let field_clone = field.clone();
        Self {
            name_str: format!("transform_unwrap_{}", field),
            transform_fn: Arc::new(move |value: serde_json::Value| {
                value.get(&field_clone).cloned().ok_or_else(|| {
                    MtwError::Codec(format!("field '{}' not found in response", field_clone))
                })
            }),
        }
    }
}

#[async_trait]
impl PipelineStage for TransformStage {
    fn name(&self) -> &str {
        &self.name_str
    }

    fn priority(&self) -> i32 {
        50
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let json_value = match &response.body {
            ResponseBody::Json(v) => v.clone(),
            ResponseBody::Bytes(b) => {
                match serde_json::from_slice::<serde_json::Value>(b) {
                    Ok(v) => v,
                    Err(_) => return Ok(PipelineAction::Continue(response)),
                }
            }
            ResponseBody::Empty => return Ok(PipelineAction::Continue(response)),
        };

        let transformed = (self.transform_fn)(json_value)?;
        response.body = ResponseBody::Json(transformed);
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
    async fn test_transform_unwrap_field() {
        let stage = TransformStage::unwrap_field("data");
        let resp = MtwResponse::new(200).with_json(serde_json::json!({
            "data": {"name": "Alice"},
            "meta": {"page": 1}
        }));
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let val: serde_json::Value = resp.json().unwrap();
            assert_eq!(val["name"], "Alice");
            assert!(val.get("meta").is_none());
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_custom_transform() {
        let stage = TransformStage::new(
            "double_values",
            Arc::new(|value: serde_json::Value| {
                if let Some(arr) = value.as_array() {
                    let doubled: Vec<serde_json::Value> = arr
                        .iter()
                        .filter_map(|v| v.as_i64())
                        .map(|n| serde_json::json!(n * 2))
                        .collect();
                    Ok(serde_json::Value::Array(doubled))
                } else {
                    Ok(value)
                }
            }),
        );

        let resp = MtwResponse::new(200).with_json(serde_json::json!([1, 2, 3]));
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let val: Vec<i64> = resp.json().unwrap();
            assert_eq!(val, vec![2, 4, 6]);
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_transform_skips_non_json() {
        let stage = TransformStage::unwrap_field("data");
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
    }

    #[tokio::test]
    async fn test_unwrap_missing_field_errors() {
        let stage = TransformStage::unwrap_field("missing");
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"other": 1}));
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await;
        assert!(result.is_err());
    }
}
