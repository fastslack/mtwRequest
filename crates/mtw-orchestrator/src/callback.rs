use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use crate::types::{MessageContext, OrchestratorResponse};

pub type CallbackHandlerFn = Arc<dyn Fn(CallbackData, MessageContext) -> Pin<Box<dyn Future<Output = Result<OrchestratorResponse, MtwError>> + Send>> + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackData {
    pub action: String, pub entity_type: Option<String>,
    pub entity_id: Option<String>, pub extra: HashMap<String, String>,
}

impl CallbackData {
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.is_empty() { return Err("empty callback data".into()); }
        Ok(Self {
            action: parts[0].to_string(),
            entity_type: parts.get(1).map(|s| s.to_string()),
            entity_id: parts.get(2).map(|s| s.to_string()),
            extra: HashMap::new(),
        })
    }
}

pub struct CallbackRegistry { handlers: HashMap<String, CallbackHandlerFn> }

impl CallbackRegistry {
    pub fn new() -> Self { Self { handlers: HashMap::new() } }
    pub fn register(&mut self, action: impl Into<String>, handler: CallbackHandlerFn) { self.handlers.insert(action.into(), handler); }

    pub async fn handle(&self, data: &str, ctx: MessageContext) -> Result<OrchestratorResponse, MtwError> {
        let parsed = CallbackData::parse(data).map_err(|e| MtwError::Internal(e))?;
        let handler = self.handlers.get(&parsed.action)
            .ok_or_else(|| MtwError::Internal(format!("no handler for action: {}", parsed.action)))?;
        (handler)(parsed, ctx).await
    }
}

impl Default for CallbackRegistry { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_callback() {
        let d = CallbackData::parse("acknowledge:task:abc123").unwrap();
        assert_eq!(d.action, "acknowledge");
        assert_eq!(d.entity_type, Some("task".into()));
        assert_eq!(d.entity_id, Some("abc123".into()));
    }
    #[test]
    fn test_parse_simple() {
        let d = CallbackData::parse("ok").unwrap();
        assert_eq!(d.action, "ok");
        assert!(d.entity_type.is_none());
    }
}
