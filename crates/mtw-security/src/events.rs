use serde::{Deserialize, Serialize};
use std::fmt;

use crate::approval::RiskLevel;

/// Security events emitted by the security subsystem
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecurityEvent {
    ToolBlocked {
        tool: String,
        user_id: String,
        platform: String,
        reason: String,
    },
    RateLimited {
        user_id: String,
        platform: String,
        count: u32,
        limit: u32,
    },
    PairingRequested {
        code: String,
        user_id: String,
        platform: String,
    },
    PairingApproved {
        user_id: String,
        platform: String,
    },
    PairingDenied {
        user_id: String,
        platform: String,
    },
    Unauthorized {
        user_id: String,
        platform: String,
        reason: String,
    },
    ApprovalRequested {
        approval_id: String,
        tool: String,
        user_id: String,
        risk_level: RiskLevel,
    },
    ApprovalGranted {
        approval_id: String,
        tool: String,
        resolved_by: String,
    },
    ApprovalDenied {
        approval_id: String,
        tool: String,
        resolved_by: String,
    },
}

impl fmt::Display for SecurityEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolBlocked {
                tool,
                user_id,
                reason,
                ..
            } => write!(f, "tool blocked: {} for {} ({})", tool, user_id, reason),
            Self::RateLimited {
                user_id,
                count,
                limit,
                ..
            } => write!(f, "rate limited: {} ({}/{})", user_id, count, limit),
            Self::PairingRequested {
                code,
                user_id,
                platform,
            } => write!(f, "pairing requested: {} on {} (code: {})", user_id, platform, code),
            Self::PairingApproved {
                user_id, platform, ..
            } => write!(f, "pairing approved: {} on {}", user_id, platform),
            Self::PairingDenied {
                user_id, platform, ..
            } => write!(f, "pairing denied: {} on {}", user_id, platform),
            Self::Unauthorized {
                user_id, reason, ..
            } => write!(f, "unauthorized: {} ({})", user_id, reason),
            Self::ApprovalRequested {
                tool,
                user_id,
                risk_level,
                ..
            } => write!(
                f,
                "approval requested: {} by {} (risk: {:?})",
                tool, user_id, risk_level
            ),
            Self::ApprovalGranted {
                tool, resolved_by, ..
            } => write!(f, "approval granted: {} by {}", tool, resolved_by),
            Self::ApprovalDenied {
                tool, resolved_by, ..
            } => write!(f, "approval denied: {} by {}", tool, resolved_by),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_display() {
        let event = SecurityEvent::ToolBlocked {
            tool: "kernel_delete".into(),
            user_id: "user1".into(),
            platform: "api".into(),
            reason: "not allowed".into(),
        };
        let s = event.to_string();
        assert!(s.contains("kernel_delete"));
        assert!(s.contains("user1"));
    }

    #[test]
    fn test_event_serialization() {
        let event = SecurityEvent::RateLimited {
            user_id: "user1".into(),
            platform: "telegram".into(),
            count: 1001,
            limit: 1000,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("rate_limited"));
        assert!(json.contains("1001"));
    }
}
