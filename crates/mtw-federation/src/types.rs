use serde::{Deserialize, Serialize};

/// A federation instance (self or remote)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationInstance {
    pub instance_id: String,
    pub name: String,
    pub url: String,
    pub created_at: String,
}

/// Status of a federation peer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    Active,
    Paused,
    Unreachable,
}

/// A federation peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationPeer {
    pub id: String,
    pub name: String,
    pub url: String,
    pub api_key: String,
    pub status: PeerStatus,
    pub last_seen_at: Option<String>,
    pub last_sync_version: u64,
    pub sync_errors: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Type of change action
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Insert,
    Update,
    Delete,
}

/// An entry in the change log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeLogEntry {
    pub id: String,
    pub version: u64,
    pub instance_id: String,
    pub table_name: String,
    pub row_id: String,
    pub action: ChangeAction,
    pub data_json: serde_json::Value,
    pub updated_at: String,
    pub created_at: String,
}

/// How a conflict was resolved
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    LocalWins,
    RemoteWins,
    Merged,
    Manual,
}

/// A sync conflict record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    pub id: String,
    pub table_name: String,
    pub row_id: String,
    pub local_data: serde_json::Value,
    pub remote_data: serde_json::Value,
    pub remote_instance_id: String,
    pub resolution: ConflictResolution,
    pub resolved_at: Option<String>,
}

/// Sync status for a peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    pub peer_id: String,
    pub peer_name: String,
    pub peer_url: String,
    pub status: PeerStatus,
    pub last_seen_at: Option<String>,
    pub last_sync_version: u64,
    pub pending_changes: u64,
    pub sync_errors: u32,
}

/// Result of a sync operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub applied: u32,
    pub errors: u32,
    pub conflicts: u32,
}

impl SyncResult {
    pub fn empty() -> Self {
        Self {
            applied: 0,
            errors: 0,
            conflicts: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_action_serialization() {
        let action = ChangeAction::Insert;
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, "\"insert\"");

        let deserialized: ChangeAction = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ChangeAction::Insert);
    }

    #[test]
    fn test_sync_result_empty() {
        let r = SyncResult::empty();
        assert_eq!(r.applied, 0);
        assert_eq!(r.errors, 0);
        assert_eq!(r.conflicts, 0);
    }
}
