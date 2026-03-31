use async_trait::async_trait;
use mtw_core::MtwError;
use crate::types::{SkillConfig, SkillMetadata, SkillStatus};
use crate::tool::{SkillToolDef, SkillToolResult};

#[async_trait]
pub trait MtwSkill: Send + Sync {
    fn metadata(&self) -> &SkillMetadata;
    fn tools(&self) -> Vec<SkillToolDef>;
    async fn initialize(&self, config: &SkillConfig) -> Result<(), MtwError>;
    async fn execute_tool(&self, tool_name: &str, args: serde_json::Value) -> Result<SkillToolResult, MtwError>;
    async fn shutdown(&self) -> Result<(), MtwError>;
    fn status(&self) -> SkillStatus;
}
