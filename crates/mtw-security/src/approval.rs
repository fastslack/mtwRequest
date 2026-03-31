use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Risk level for operations requiring approval
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Approval status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Timeout,
}

/// A gate that requires approval before a tool can execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalGate {
    /// Tool name pattern (supports glob `*` suffix)
    pub tool_pattern: String,
    /// Timeout before auto-deny
    #[serde(with = "duration_secs")]
    pub timeout: Duration,
    /// Risk level
    pub risk_level: RiskLevel,
    /// Human-readable reason for requiring approval
    pub reason: String,
    /// Which channel to send approval request to
    pub channel: String,
}

mod duration_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}

impl ApprovalGate {
    pub fn new(
        tool_pattern: impl Into<String>,
        risk_level: RiskLevel,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tool_pattern: tool_pattern.into(),
            timeout: Duration::from_secs(300),
            risk_level,
            reason: reason.into(),
            channel: "all".to_string(),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = channel.into();
        self
    }

    /// Check if this gate matches a tool name
    pub fn matches(&self, tool_name: &str) -> bool {
        if self.tool_pattern == "*" {
            return true;
        }
        if let Some(prefix) = self.tool_pattern.strip_suffix('*') {
            tool_name.starts_with(prefix)
        } else {
            self.tool_pattern == tool_name
        }
    }
}

/// A pending approval request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub user_id: String,
    pub risk_level: RiskLevel,
    pub reason: String,
    pub status: ApprovalStatus,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
    #[serde(skip)]
    pub expires_at: Option<Instant>,
}

/// Manages approval gates and pending approvals
pub struct ApprovalManager {
    gates: Vec<ApprovalGate>,
    pending: DashMap<String, PendingApproval>,
}

impl ApprovalManager {
    pub fn new() -> Self {
        Self {
            gates: Vec::new(),
            pending: DashMap::new(),
        }
    }

    /// Register an approval gate
    pub fn register_gate(&mut self, gate: ApprovalGate) {
        self.gates.push(gate);
    }

    /// Check if a tool requires approval, returns the matching gate
    pub fn check_requires_approval(&self, tool_name: &str) -> Option<&ApprovalGate> {
        self.gates.iter().find(|g| g.matches(tool_name))
    }

    /// Create a pending approval request
    pub fn request_approval(
        &self,
        tool_name: impl Into<String>,
        args: serde_json::Value,
        user_id: impl Into<String>,
        gate: &ApprovalGate,
    ) -> PendingApproval {
        let now = Instant::now();
        let approval = PendingApproval {
            id: ulid::Ulid::new().to_string(),
            tool_name: tool_name.into(),
            args,
            user_id: user_id.into(),
            risk_level: gate.risk_level,
            reason: gate.reason.clone(),
            status: ApprovalStatus::Pending,
            created_at: format!("{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()),
            resolved_at: None,
            resolved_by: None,
            expires_at: Some(now + gate.timeout),
        };
        self.pending.insert(approval.id.clone(), approval.clone());
        approval
    }

    /// Resolve a pending approval
    pub fn resolve(
        &self,
        id: &str,
        approved: bool,
        resolved_by: impl Into<String>,
    ) -> Result<PendingApproval, MtwError> {
        let mut entry = self
            .pending
            .get_mut(id)
            .ok_or_else(|| MtwError::Internal(format!("approval not found: {}", id)))?;

        if entry.status != ApprovalStatus::Pending {
            return Err(MtwError::Internal(format!(
                "approval {} already resolved",
                id
            )));
        }

        entry.status = if approved {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Denied
        };
        entry.resolved_by = Some(resolved_by.into());
        entry.resolved_at = Some(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));

        Ok(entry.clone())
    }

    /// Get a pending approval by ID
    pub fn get(&self, id: &str) -> Option<PendingApproval> {
        self.pending.get(id).map(|e| e.clone())
    }

    /// List all pending approvals
    pub fn list_pending(&self) -> Vec<PendingApproval> {
        self.pending
            .iter()
            .filter(|e| e.status == ApprovalStatus::Pending)
            .map(|e| e.clone())
            .collect()
    }

    /// Clean up expired approvals (mark as timeout)
    pub fn cleanup_expired(&self) -> usize {
        let now = Instant::now();
        let mut count = 0;
        for mut entry in self.pending.iter_mut() {
            if entry.status == ApprovalStatus::Pending {
                if let Some(expires) = entry.expires_at {
                    if now >= expires {
                        entry.status = ApprovalStatus::Timeout;
                        count += 1;
                    }
                }
            }
        }
        count
    }
}

impl Default for ApprovalManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create default approval gates for common dangerous operations
pub fn default_gates() -> Vec<ApprovalGate> {
    vec![
        ApprovalGate::new(
            "kernel_comms_send",
            RiskLevel::Medium,
            "Sending messages requires approval",
        ),
        ApprovalGate::new(
            "kernel_events_cancel",
            RiskLevel::High,
            "Cancelling events is destructive",
        ),
        ApprovalGate::new(
            "kernel_crm_delete_*",
            RiskLevel::High,
            "Deleting CRM data is destructive",
        ),
        ApprovalGate::new(
            "kernel_trading_execute_*",
            RiskLevel::Critical,
            "Executing trades requires explicit approval",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_matching() {
        let gate = ApprovalGate::new("kernel_comms_*", RiskLevel::Medium, "test");
        assert!(gate.matches("kernel_comms_send"));
        assert!(gate.matches("kernel_comms_draft"));
        assert!(!gate.matches("kernel_tasks_create"));
    }

    #[test]
    fn test_exact_match() {
        let gate = ApprovalGate::new("kernel_comms_send", RiskLevel::Medium, "test");
        assert!(gate.matches("kernel_comms_send"));
        assert!(!gate.matches("kernel_comms_draft"));
    }

    #[test]
    fn test_approval_flow() {
        let mut manager = ApprovalManager::new();
        let gate = ApprovalGate::new("dangerous_tool", RiskLevel::High, "requires approval");
        manager.register_gate(gate.clone());

        // Check if tool requires approval
        assert!(manager.check_requires_approval("dangerous_tool").is_some());
        assert!(manager.check_requires_approval("safe_tool").is_none());

        // Request approval
        let approval = manager.request_approval(
            "dangerous_tool",
            serde_json::json!({"action": "delete"}),
            "user1",
            &gate,
        );
        assert_eq!(approval.status, ApprovalStatus::Pending);

        // List pending
        assert_eq!(manager.list_pending().len(), 1);

        // Resolve
        let resolved = manager.resolve(&approval.id, true, "admin").unwrap();
        assert_eq!(resolved.status, ApprovalStatus::Approved);
        assert_eq!(resolved.resolved_by, Some("admin".to_string()));

        // No more pending
        assert_eq!(manager.list_pending().len(), 0);
    }

    #[test]
    fn test_deny_approval() {
        let manager = ApprovalManager::new();
        let gate = ApprovalGate::new("tool", RiskLevel::Low, "test");

        let approval = manager.request_approval("tool", serde_json::json!({}), "user", &gate);
        let resolved = manager.resolve(&approval.id, false, "admin").unwrap();
        assert_eq!(resolved.status, ApprovalStatus::Denied);
    }

    #[test]
    fn test_double_resolve_fails() {
        let manager = ApprovalManager::new();
        let gate = ApprovalGate::new("tool", RiskLevel::Low, "test");

        let approval = manager.request_approval("tool", serde_json::json!({}), "user", &gate);
        manager.resolve(&approval.id, true, "admin").unwrap();
        assert!(manager.resolve(&approval.id, false, "admin").is_err());
    }

    #[test]
    fn test_default_gates() {
        let gates = default_gates();
        assert!(gates.len() >= 4);
        assert!(gates.iter().any(|g| g.matches("kernel_comms_send")));
        assert!(gates.iter().any(|g| g.matches("kernel_trading_execute_buy")));
    }
}
