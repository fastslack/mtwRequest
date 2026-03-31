use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolDef {
    pub name: String, pub description: String,
    pub parameters: serde_json::Value, pub handler_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolResult { pub content: String, pub is_error: bool }

impl SkillToolResult {
    pub fn ok(content: impl Into<String>) -> Self { Self { content: content.into(), is_error: false } }
    pub fn err(content: impl Into<String>) -> Self { Self { content: content.into(), is_error: true } }
}
