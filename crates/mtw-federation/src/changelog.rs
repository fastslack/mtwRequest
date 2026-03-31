use std::sync::RwLock;

use crate::types::{ChangeAction, ChangeLogEntry};

/// In-memory change log with auto-incrementing version counter
pub struct ChangeLog {
    instance_id: String,
    entries: RwLock<Vec<ChangeLogEntry>>,
    version: RwLock<u64>,
}

impl ChangeLog {
    pub fn new(instance_id: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.into(),
            entries: RwLock::new(Vec::new()),
            version: RwLock::new(0),
        }
    }

    /// Record a change and return the entry
    pub fn record(
        &self,
        table: impl Into<String>,
        row_id: impl Into<String>,
        action: ChangeAction,
        data: serde_json::Value,
    ) -> ChangeLogEntry {
        let mut version = self.version.write().unwrap();
        *version += 1;
        let v = *version;

        let now = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        let entry = ChangeLogEntry {
            id: ulid::Ulid::new().to_string(),
            version: v,
            instance_id: self.instance_id.clone(),
            table_name: table.into(),
            row_id: row_id.into(),
            action,
            data_json: data,
            updated_at: now.clone(),
            created_at: now,
        };

        self.entries.write().unwrap().push(entry.clone());
        entry
    }

    /// Get changes since a version (exclusive), up to limit
    pub fn get_changes_since(&self, version: u64, limit: usize) -> Vec<ChangeLogEntry> {
        self.entries
            .read()
            .unwrap()
            .iter()
            .filter(|e| e.version > version)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Get the latest version number
    pub fn get_latest_version(&self) -> u64 {
        *self.version.read().unwrap()
    }

    /// Count changes since a version
    pub fn count_since(&self, version: u64) -> u64 {
        self.entries
            .read()
            .unwrap()
            .iter()
            .filter(|e| e.version > version)
            .count() as u64
    }

    /// Prune entries with version <= the given version
    pub fn prune_before(&self, version: u64) {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| e.version > version);
    }

    /// Apply remote changes (from a peer)
    pub fn apply_remote(&self, entries: Vec<ChangeLogEntry>) -> u32 {
        let mut local = self.entries.write().unwrap();
        let mut applied = 0;
        for entry in entries {
            // Skip if we already have this version from this instance
            let exists = local
                .iter()
                .any(|e| e.instance_id == entry.instance_id && e.version == entry.version);
            if !exists {
                local.push(entry);
                applied += 1;
            }
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_get() {
        let log = ChangeLog::new("instance-1");

        log.record("tasks", "task-1", ChangeAction::Insert, serde_json::json!({"title": "Test"}));
        log.record("tasks", "task-2", ChangeAction::Insert, serde_json::json!({"title": "Test 2"}));

        assert_eq!(log.get_latest_version(), 2);
        assert_eq!(log.get_changes_since(0, 100).len(), 2);
        assert_eq!(log.get_changes_since(1, 100).len(), 1);
    }

    #[test]
    fn test_count_since() {
        let log = ChangeLog::new("instance-1");
        log.record("t", "1", ChangeAction::Insert, serde_json::json!({}));
        log.record("t", "2", ChangeAction::Update, serde_json::json!({}));
        log.record("t", "3", ChangeAction::Delete, serde_json::json!({}));

        assert_eq!(log.count_since(0), 3);
        assert_eq!(log.count_since(2), 1);
        assert_eq!(log.count_since(3), 0);
    }

    #[test]
    fn test_prune() {
        let log = ChangeLog::new("instance-1");
        log.record("t", "1", ChangeAction::Insert, serde_json::json!({}));
        log.record("t", "2", ChangeAction::Insert, serde_json::json!({}));
        log.record("t", "3", ChangeAction::Insert, serde_json::json!({}));

        log.prune_before(2);
        assert_eq!(log.get_changes_since(0, 100).len(), 1);
    }

    #[test]
    fn test_limit() {
        let log = ChangeLog::new("instance-1");
        for i in 0..10 {
            log.record("t", &i.to_string(), ChangeAction::Insert, serde_json::json!({}));
        }
        assert_eq!(log.get_changes_since(0, 5).len(), 5);
    }
}
