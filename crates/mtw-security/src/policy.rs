use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Security policy mode
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    Allowlist,
    Denylist,
}

impl Default for PolicyMode {
    fn default() -> Self {
        Self::Denylist
    }
}

/// Security policy defining tool access rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPolicy {
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub mode: PolicyMode,
    pub rate_limit: u32,
    pub max_tokens: u32,
    pub require_pairing: bool,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            mode: PolicyMode::Denylist,
            rate_limit: 1000,
            max_tokens: 100_000,
            require_pairing: false,
        }
    }
}

/// Global security configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub default_policy: SecurityPolicy,
    pub pairing_code_expiration_mins: u32,
    pub rate_limit_window_secs: u32,
    pub log_events: bool,
    pub trusted_platforms: Vec<String>,
    pub trusted_users: HashMap<String, Vec<String>>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            default_policy: SecurityPolicy::default(),
            pairing_code_expiration_mins: 5,
            rate_limit_window_secs: 60,
            log_events: true,
            trusted_platforms: Vec::new(),
            trusted_users: HashMap::new(),
        }
    }
}

/// Policy engine that evaluates tool access
pub struct PolicyEngine {
    config: SecurityConfig,
    per_user_policies: HashMap<String, SecurityPolicy>,
}

impl PolicyEngine {
    pub fn new(config: SecurityConfig) -> Self {
        Self {
            config,
            per_user_policies: HashMap::new(),
        }
    }

    /// Set a custom policy for a specific user
    pub fn set_user_policy(&mut self, user_id: impl Into<String>, policy: SecurityPolicy) {
        self.per_user_policies.insert(user_id.into(), policy);
    }

    /// Check if a tool is allowed for a given user and platform
    pub fn is_tool_allowed(&self, tool_name: &str, user_id: &str, platform: &str) -> bool {
        // Trusted platforms bypass all checks
        if self.config.trusted_platforms.contains(&platform.to_string()) {
            return true;
        }

        // Trusted users bypass all checks
        if let Some(users) = self.config.trusted_users.get(platform) {
            if users.contains(&user_id.to_string()) {
                return true;
            }
        }

        let policy = self
            .per_user_policies
            .get(user_id)
            .unwrap_or(&self.config.default_policy);

        match policy.mode {
            PolicyMode::Allowlist => {
                policy.allowed_tools.iter().any(|p| pattern_matches(p, tool_name))
            }
            PolicyMode::Denylist => {
                !policy.denied_tools.iter().any(|p| pattern_matches(p, tool_name))
            }
        }
    }

    /// Get the rate limit for a user
    pub fn rate_limit_for(&self, user_id: &str) -> u32 {
        self.per_user_policies
            .get(user_id)
            .map(|p| p.rate_limit)
            .unwrap_or(self.config.default_policy.rate_limit)
    }

    /// Check if pairing is required for a user
    pub fn requires_pairing(&self, user_id: &str) -> bool {
        self.per_user_policies
            .get(user_id)
            .map(|p| p.require_pairing)
            .unwrap_or(self.config.default_policy.require_pairing)
    }
}

/// Simple pattern matching supporting glob-style `*` suffix and exact match
fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        value.starts_with(prefix)
    } else {
        pattern == value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_denylist_mode() {
        let config = SecurityConfig {
            default_policy: SecurityPolicy {
                denied_tools: vec!["kernel_comms_send".into(), "kernel_trading_*".into()],
                mode: PolicyMode::Denylist,
                ..Default::default()
            },
            ..Default::default()
        };
        let engine = PolicyEngine::new(config);

        assert!(!engine.is_tool_allowed("kernel_comms_send", "user1", "api"));
        assert!(!engine.is_tool_allowed("kernel_trading_execute", "user1", "api"));
        assert!(engine.is_tool_allowed("kernel_tasks_create", "user1", "api"));
    }

    #[test]
    fn test_allowlist_mode() {
        let config = SecurityConfig {
            default_policy: SecurityPolicy {
                allowed_tools: vec!["kernel_tasks_*".into(), "kernel_reminders_*".into()],
                mode: PolicyMode::Allowlist,
                ..Default::default()
            },
            ..Default::default()
        };
        let engine = PolicyEngine::new(config);

        assert!(engine.is_tool_allowed("kernel_tasks_create", "user1", "api"));
        assert!(!engine.is_tool_allowed("kernel_comms_send", "user1", "api"));
    }

    #[test]
    fn test_trusted_platform() {
        let config = SecurityConfig {
            trusted_platforms: vec!["api".into()],
            ..Default::default()
        };
        let engine = PolicyEngine::new(config);
        assert!(engine.is_tool_allowed("anything", "anyone", "api"));
    }

    #[test]
    fn test_trusted_user() {
        let mut config = SecurityConfig::default();
        config
            .trusted_users
            .insert("telegram".into(), vec!["admin".into()]);

        let engine = PolicyEngine::new(config);
        assert!(engine.is_tool_allowed("anything", "admin", "telegram"));
        assert!(engine.is_tool_allowed("anything", "other", "telegram")); // denylist empty = all allowed
    }

    #[test]
    fn test_per_user_policy() {
        let config = SecurityConfig::default();
        let mut engine = PolicyEngine::new(config);

        engine.set_user_policy(
            "restricted_user",
            SecurityPolicy {
                allowed_tools: vec!["kernel_tasks_*".into()],
                mode: PolicyMode::Allowlist,
                rate_limit: 10,
                ..Default::default()
            },
        );

        assert!(engine.is_tool_allowed("kernel_tasks_list", "restricted_user", "api"));
        assert!(!engine.is_tool_allowed("kernel_comms_send", "restricted_user", "api"));
        assert_eq!(engine.rate_limit_for("restricted_user"), 10);
    }

    #[test]
    fn test_pattern_matches() {
        assert!(pattern_matches("*", "anything"));
        assert!(pattern_matches("kernel_*", "kernel_tasks"));
        assert!(!pattern_matches("kernel_*", "other_tasks"));
        assert!(pattern_matches("exact", "exact"));
        assert!(!pattern_matches("exact", "other"));
    }
}
