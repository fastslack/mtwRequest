use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use crate::skill::MtwSkill;
use crate::tool::{SkillToolDef, SkillToolResult};
use crate::types::{SkillConfig, SkillMetadata, SkillPermission, SkillStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRegistryConfig {
    pub skills_path: String, pub data_path: String,
    pub auto_enable: bool, pub default_permissions: Vec<SkillPermission>,
    pub blocked_skills: Vec<String>,
}

impl Default for SkillRegistryConfig {
    fn default() -> Self {
        Self { skills_path: "./skills".into(), data_path: "./data/skills".into(),
            auto_enable: false, default_permissions: Vec::new(), blocked_skills: Vec::new() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolStats { pub calls: u64, pub successes: u64, pub failures: u64, pub avg_ms: f64 }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillUsageStats {
    pub total_calls: u64, pub success_count: u64, pub failure_count: u64,
    pub last_call_at: Option<String>, pub avg_response_ms: f64,
    pub tool_stats: HashMap<String, ToolStats>,
}

struct SkillEntry {
    skill: Arc<dyn MtwSkill>,
    config: SkillConfig,
    stats: SkillUsageStats,
}

pub struct SkillRegistry {
    skills: DashMap<String, SkillEntry>,
    config: SkillRegistryConfig,
}

impl SkillRegistry {
    pub fn new(config: SkillRegistryConfig) -> Self { Self { skills: DashMap::new(), config } }

    pub fn register(&self, skill: Arc<dyn MtwSkill>) -> Result<(), MtwError> {
        let id = skill.metadata().id.clone();
        if self.config.blocked_skills.contains(&id) {
            return Err(MtwError::Internal(format!("skill is blocked: {}", id)));
        }
        let config = SkillConfig { enabled: self.config.auto_enable, granted_permissions: self.config.default_permissions.clone(), ..Default::default() };
        self.skills.insert(id, SkillEntry { skill, config, stats: Default::default() });
        Ok(())
    }

    pub fn unregister(&self, id: &str) -> bool { self.skills.remove(id).is_some() }

    pub fn list(&self) -> Vec<SkillMetadata> {
        self.skills.iter().map(|e| e.skill.metadata().clone()).collect()
    }

    pub fn list_active(&self) -> Vec<SkillMetadata> {
        self.skills.iter().filter(|e| e.config.enabled && e.skill.status() == SkillStatus::Active)
            .map(|e| e.skill.metadata().clone()).collect()
    }

    pub fn enable(&self, id: &str) -> Result<(), MtwError> {
        let mut e = self.skills.get_mut(id).ok_or_else(|| MtwError::Internal(format!("skill not found: {}", id)))?;
        e.config.enabled = true; Ok(())
    }

    pub fn disable(&self, id: &str) -> Result<(), MtwError> {
        let mut e = self.skills.get_mut(id).ok_or_else(|| MtwError::Internal(format!("skill not found: {}", id)))?;
        e.config.enabled = false; Ok(())
    }

    pub fn get_config(&self, id: &str) -> Option<SkillConfig> { self.skills.get(id).map(|e| e.config.clone()) }

    pub fn set_config(&self, id: &str, config: SkillConfig) -> Result<(), MtwError> {
        let mut e = self.skills.get_mut(id).ok_or_else(|| MtwError::Internal(format!("skill not found: {}", id)))?;
        e.config = config; Ok(())
    }

    pub fn check_permissions(&self, id: &str, required: &[SkillPermission]) -> bool {
        self.skills.get(id).map(|e| required.iter().all(|p| e.config.granted_permissions.contains(p))).unwrap_or(false)
    }

    pub fn get_stats(&self, id: &str) -> Option<SkillUsageStats> { self.skills.get(id).map(|e| e.stats.clone()) }

    pub fn record_usage(&self, id: &str, tool: &str, success: bool, duration_ms: f64) {
        if let Some(mut e) = self.skills.get_mut(id) {
            e.stats.total_calls += 1;
            if success { e.stats.success_count += 1; } else { e.stats.failure_count += 1; }
            let ts = e.stats.tool_stats.entry(tool.to_string()).or_default();
            ts.calls += 1;
            if success { ts.successes += 1; } else { ts.failures += 1; }
            ts.avg_ms = (ts.avg_ms * (ts.calls - 1) as f64 + duration_ms) / ts.calls as f64;
        }
    }

    pub fn get_all_tools(&self) -> Vec<(String, SkillToolDef)> {
        self.skills.iter().filter(|e| e.config.enabled)
            .flat_map(|e| { let id = e.skill.metadata().id.clone(); e.skill.tools().into_iter().map(move |t| (id.clone(), t)) })
            .collect()
    }

    pub async fn execute_tool(&self, skill_id: &str, tool_name: &str, args: serde_json::Value) -> Result<SkillToolResult, MtwError> {
        let entry = self.skills.get(skill_id).ok_or_else(|| MtwError::Internal(format!("skill not found: {}", skill_id)))?;
        if !entry.config.enabled { return Err(MtwError::Internal("skill is disabled".into())); }
        let result = entry.skill.execute_tool(tool_name, args).await;
        drop(entry);
        let success = result.as_ref().map(|r| !r.is_error).unwrap_or(false);
        self.record_usage(skill_id, tool_name, success, 0.0);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SkillCategory;
    use async_trait::async_trait;

    struct MockSkill { meta: SkillMetadata }

    #[async_trait]
    impl MtwSkill for MockSkill {
        fn metadata(&self) -> &SkillMetadata { &self.meta }
        fn tools(&self) -> Vec<SkillToolDef> { vec![SkillToolDef { name: "test_tool".into(), description: "test".into(), parameters: serde_json::json!({}), handler_id: "h".into() }] }
        async fn initialize(&self, _: &SkillConfig) -> Result<(), MtwError> { Ok(()) }
        async fn execute_tool(&self, _: &str, _: serde_json::Value) -> Result<SkillToolResult, MtwError> { Ok(SkillToolResult::ok("done")) }
        async fn shutdown(&self) -> Result<(), MtwError> { Ok(()) }
        fn status(&self) -> SkillStatus { SkillStatus::Active }
    }

    fn mock_skill(id: &str) -> Arc<dyn MtwSkill> {
        Arc::new(MockSkill { meta: SkillMetadata {
            id: id.into(), name: id.into(), description: "".into(), version: "0.1.0".into(),
            author: "test".into(), homepage: None, icon: None, category: Some(SkillCategory::Utility),
            permissions: vec![], tags: vec![],
        }})
    }

    #[test]
    fn test_register_and_list() {
        let reg = SkillRegistry::new(SkillRegistryConfig::default());
        reg.register(mock_skill("s1")).unwrap();
        reg.register(mock_skill("s2")).unwrap();
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn test_blocked_skill() {
        let config = SkillRegistryConfig { blocked_skills: vec!["bad".into()], ..Default::default() };
        let reg = SkillRegistry::new(config);
        assert!(reg.register(mock_skill("bad")).is_err());
    }

    #[test]
    fn test_enable_disable() {
        let reg = SkillRegistry::new(SkillRegistryConfig { auto_enable: true, ..Default::default() });
        reg.register(mock_skill("s1")).unwrap();
        assert!(reg.get_config("s1").unwrap().enabled);
        reg.disable("s1").unwrap();
        assert!(!reg.get_config("s1").unwrap().enabled);
        reg.enable("s1").unwrap();
        assert!(reg.get_config("s1").unwrap().enabled);
    }

    #[test]
    fn test_usage_stats() {
        let reg = SkillRegistry::new(SkillRegistryConfig::default());
        reg.register(mock_skill("s1")).unwrap();
        reg.record_usage("s1", "tool1", true, 10.0);
        reg.record_usage("s1", "tool1", false, 20.0);
        let stats = reg.get_stats("s1").unwrap();
        assert_eq!(stats.total_calls, 2);
        assert_eq!(stats.success_count, 1);
    }
}
