use async_trait::async_trait;
use mtw_core::MtwError;
use std::sync::Arc;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::{MtwResponse, ResponseBody};

/// A single validation rule.
#[derive(Clone)]
pub struct ValidationRule {
    /// Human-readable name for this rule.
    pub name: String,
    /// The rule kind.
    pub kind: ValidationRuleKind,
}

/// Kinds of validation rules.
#[derive(Clone)]
pub enum ValidationRuleKind {
    /// A field that must be present at the given JSON path.
    RequiredField(String),
    /// A field that must be a specific JSON type.
    FieldType {
        field: String,
        expected_type: JsonType,
    },
    /// A custom validator function.
    Custom(Arc<dyn Fn(&serde_json::Value) -> Result<(), String> + Send + Sync>),
}

/// Expected JSON types for field type validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonType {
    String,
    Number,
    Boolean,
    Array,
    Object,
    Null,
}

impl JsonType {
    fn matches(&self, value: &serde_json::Value) -> bool {
        match self {
            JsonType::String => value.is_string(),
            JsonType::Number => value.is_number(),
            JsonType::Boolean => value.is_boolean(),
            JsonType::Array => value.is_array(),
            JsonType::Object => value.is_object(),
            JsonType::Null => value.is_null(),
        }
    }
}

impl std::fmt::Display for JsonType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonType::String => write!(f, "string"),
            JsonType::Number => write!(f, "number"),
            JsonType::Boolean => write!(f, "boolean"),
            JsonType::Array => write!(f, "array"),
            JsonType::Object => write!(f, "object"),
            JsonType::Null => write!(f, "null"),
        }
    }
}

/// Configuration for the validation stage.
pub struct ValidationConfig {
    pub rules: Vec<ValidationRule>,
    /// If true, collect all errors. If false, fail on first error.
    pub collect_all: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            collect_all: false,
        }
    }
}

/// Pipeline stage that validates response bodies against rules.
pub struct ValidationStage {
    config: ValidationConfig,
}

impl ValidationStage {
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }

    /// Create a stage requiring specific fields to exist.
    pub fn require_fields(fields: Vec<&str>) -> Self {
        let rules = fields
            .into_iter()
            .map(|f| ValidationRule {
                name: format!("required:{}", f),
                kind: ValidationRuleKind::RequiredField(f.to_string()),
            })
            .collect();
        Self {
            config: ValidationConfig {
                rules,
                collect_all: false,
            },
        }
    }

    fn validate_value(
        &self,
        value: &serde_json::Value,
    ) -> Vec<String> {
        let mut errors = Vec::new();

        for rule in &self.config.rules {
            match &rule.kind {
                ValidationRuleKind::RequiredField(field) => {
                    if value.get(field).is_none() {
                        errors.push(format!("required field '{}' is missing", field));
                        if !self.config.collect_all {
                            return errors;
                        }
                    }
                }
                ValidationRuleKind::FieldType {
                    field,
                    expected_type,
                } => {
                    if let Some(val) = value.get(field) {
                        if !expected_type.matches(val) {
                            errors.push(format!(
                                "field '{}' expected type {}, got different type",
                                field, expected_type
                            ));
                            if !self.config.collect_all {
                                return errors;
                            }
                        }
                    }
                }
                ValidationRuleKind::Custom(validator) => {
                    if let Err(msg) = validator(value) {
                        errors.push(msg);
                        if !self.config.collect_all {
                            return errors;
                        }
                    }
                }
            }
        }

        errors
    }
}

#[async_trait]
impl PipelineStage for ValidationStage {
    fn name(&self) -> &str {
        "validation"
    }

    fn priority(&self) -> i32 {
        60
    }

    async fn process(
        &self,
        response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let value = match &response.body {
            ResponseBody::Json(v) => v.clone(),
            ResponseBody::Bytes(b) => match serde_json::from_slice::<serde_json::Value>(b) {
                Ok(v) => v,
                Err(_) => return Ok(PipelineAction::Continue(response)),
            },
            ResponseBody::Empty => return Ok(PipelineAction::Continue(response)),
        };

        let errors = self.validate_value(&value);
        if errors.is_empty() {
            Ok(PipelineAction::Continue(response))
        } else {
            Ok(PipelineAction::Error(MtwError::Codec(format!(
                "validation failed: {}",
                errors.join("; ")
            ))))
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
    async fn test_required_fields_present() {
        let stage = ValidationStage::require_fields(vec!["id", "name"]);
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"id": 1, "name": "Alice"}));
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
    }

    #[tokio::test]
    async fn test_required_field_missing() {
        let stage = ValidationStage::require_fields(vec!["id", "name"]);
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"id": 1}));
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Error(_)));
    }

    #[tokio::test]
    async fn test_field_type_check() {
        let config = ValidationConfig {
            rules: vec![ValidationRule {
                name: "age_is_number".into(),
                kind: ValidationRuleKind::FieldType {
                    field: "age".into(),
                    expected_type: JsonType::Number,
                },
            }],
            collect_all: false,
        };
        let stage = ValidationStage::new(config);
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"age": "not a number"}));
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Error(_)));
    }

    #[tokio::test]
    async fn test_custom_validator() {
        let config = ValidationConfig {
            rules: vec![ValidationRule {
                name: "positive_count".into(),
                kind: ValidationRuleKind::Custom(Arc::new(|val| {
                    let count = val
                        .get("count")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if count > 0 {
                        Ok(())
                    } else {
                        Err("count must be positive".into())
                    }
                })),
            }],
            collect_all: false,
        };
        let stage = ValidationStage::new(config);

        // Should pass
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"count": 5}));
        let mut ctx = make_ctx();
        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));

        // Should fail
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"count": 0}));
        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Error(_)));
    }

    #[tokio::test]
    async fn test_empty_body_passes() {
        let stage = ValidationStage::require_fields(vec!["id"]);
        let resp = MtwResponse::new(204);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
    }
}
