//! Skills-over-MCP: expose `mtw-skills` as MCP resources & prompts.
//!
//! ## Why this matters
//! David's talk telegraphs "skills over MCP" as the next experimental
//! extension: server authors ship the *playbook* alongside the tools so
//! the model knows how to use them. We already have `mtw-skills` as a
//! first-class primitive; this module surfaces it through the standard
//! MCP `resources/*` and `prompts/*` methods so any compliant client gets
//! it for free, no plugin mechanism required.
//!
//! ## URI scheme
//! Each skill is two resources:
//!   * `skill://{skill_id}/manifest`  — the [`SkillMetadata`] as JSON.
//!   * `skill://{skill_id}/tools`     — the skill's tool definitions.
//!
//! Plus, every skill exposes its `name` as a prompt that returns its
//! description as the system message — handy for "load skill X" flows.
//!
//! Because both providers are optional, the `initialize` handshake only
//! advertises `resources` / `prompts` capabilities when a `SkillRegistry`
//! has actually been wired in.

use crate::protocol::{PromptProvider, ResourceProvider};
use async_trait::async_trait;
use mtw_skills::registry::SkillRegistry;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct SkillResources {
    registry: Arc<SkillRegistry>,
}

impl SkillResources {
    pub fn new(registry: Arc<SkillRegistry>) -> Self { Self { registry } }
}

#[async_trait]
impl ResourceProvider for SkillResources {
    async fn list(&self, _cursor: Option<&str>) -> Result<Value, String> {
        let mut resources: Vec<Value> = Vec::new();
        for meta in self.registry.list() {
            resources.push(json!({
                "uri": format!("skill://{}/manifest", meta.id),
                "name": format!("{} — manifest", meta.name),
                "description": meta.description,
                "mimeType": "application/json",
            }));
            resources.push(json!({
                "uri": format!("skill://{}/tools", meta.id),
                "name": format!("{} — tools", meta.name),
                "description": format!("Tool definitions exposed by skill `{}`.", meta.id),
                "mimeType": "application/json",
            }));
        }
        Ok(json!({ "resources": resources }))
    }

    async fn read(&self, uri: &str) -> Result<Value, String> {
        let stripped = uri
            .strip_prefix("skill://")
            .ok_or_else(|| format!("unsupported uri scheme: {}", uri))?;
        let mut parts = stripped.splitn(2, '/');
        let skill_id = parts.next().unwrap_or("");
        let kind = parts.next().unwrap_or("");

        let meta = self
            .registry
            .list()
            .into_iter()
            .find(|m| m.id == skill_id)
            .ok_or_else(|| format!("skill not found: {}", skill_id))?;

        let payload = match kind {
            "manifest" => serde_json::to_value(&meta).unwrap_or(Value::Null),
            "tools" => {
                // The registry exposes (skill_id, tool) pairs; filter by id.
                let tools: Vec<Value> = self
                    .registry
                    .get_all_tools()
                    .into_iter()
                    .filter(|(id, _)| id == skill_id)
                    .map(|(_, t)| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        })
                    })
                    .collect();
                json!({ "tools": tools })
            }
            other => return Err(format!("unknown resource kind: {}", other)),
        };

        Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": payload.to_string(),
            }]
        }))
    }
}

pub struct SkillPrompts {
    registry: Arc<SkillRegistry>,
}

impl SkillPrompts {
    pub fn new(registry: Arc<SkillRegistry>) -> Self { Self { registry } }
}

#[async_trait]
impl PromptProvider for SkillPrompts {
    async fn list(&self, _cursor: Option<&str>) -> Result<Value, String> {
        let prompts: Vec<Value> = self
            .registry
            .list()
            .iter()
            .map(|m| {
                json!({
                    "name": format!("skill::{}", m.id),
                    "description": m.description,
                    "arguments": []
                })
            })
            .collect();
        Ok(json!({ "prompts": prompts }))
    }

    async fn get(&self, name: &str, _arguments: &Value) -> Result<Value, String> {
        let id = name
            .strip_prefix("skill::")
            .ok_or_else(|| format!("unknown prompt: {}", name))?;
        let meta = self
            .registry
            .list()
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("skill not found: {}", id))?;

        let body = format!(
            "You have access to the `{}` skill ({} v{} by {}).\n\n{}\n\nTags: {}",
            meta.name,
            meta.id,
            meta.version,
            meta.author,
            meta.description,
            meta.tags.join(", "),
        );

        Ok(json!({
            "description": meta.description,
            "messages": [{
                "role": "user",
                "content": { "type": "text", "text": body }
            }]
        }))
    }
}
