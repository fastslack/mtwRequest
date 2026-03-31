use serde::{Deserialize, Serialize};
use std::sync::RwLock;

use crate::provider::NotificationPriority;

/// A persisted dashboard notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardNotification {
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub source: Option<String>,
    pub priority: NotificationPriority,
    pub read: bool,
    pub created_at: String,
}

impl DashboardNotification {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            title: title.into(),
            body: None,
            source: None,
            priority: NotificationPriority::Normal,
            read: false,
            created_at: chrono_now(),
        }
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_priority(mut self, priority: NotificationPriority) -> Self {
        self.priority = priority;
        self
    }
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

/// In-memory store for dashboard notifications
pub struct DashboardStore {
    notifications: RwLock<Vec<DashboardNotification>>,
}

impl DashboardStore {
    pub fn new() -> Self {
        Self {
            notifications: RwLock::new(Vec::new()),
        }
    }

    /// Add a notification
    pub fn add(&self, notification: DashboardNotification) {
        self.notifications.write().unwrap().push(notification);
    }

    /// Get all notifications
    pub fn get_all(&self) -> Vec<DashboardNotification> {
        self.notifications.read().unwrap().clone()
    }

    /// Get unread count
    pub fn get_unread_count(&self) -> usize {
        self.notifications
            .read()
            .unwrap()
            .iter()
            .filter(|n| !n.read)
            .count()
    }

    /// Mark a notification as read
    pub fn mark_read(&self, id: &str) -> bool {
        let mut notifications = self.notifications.write().unwrap();
        if let Some(n) = notifications.iter_mut().find(|n| n.id == id) {
            n.read = true;
            return true;
        }
        false
    }

    /// Mark all as read
    pub fn mark_all_read(&self) {
        let mut notifications = self.notifications.write().unwrap();
        for n in notifications.iter_mut() {
            n.read = true;
        }
    }

    /// Delete a notification by ID
    pub fn delete(&self, id: &str) -> bool {
        let mut notifications = self.notifications.write().unwrap();
        let len_before = notifications.len();
        notifications.retain(|n| n.id != id);
        notifications.len() < len_before
    }

    /// Remove notifications older than the given timestamp
    pub fn purge_older_than(&self, timestamp: &str) -> usize {
        let mut notifications = self.notifications.write().unwrap();
        let len_before = notifications.len();
        notifications.retain(|n| n.created_at.as_str() >= timestamp);
        len_before - notifications.len()
    }
}

impl Default for DashboardStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_store() {
        let store = DashboardStore::new();
        store.add(DashboardNotification::new("Alert 1"));
        store.add(DashboardNotification::new("Alert 2"));

        assert_eq!(store.get_all().len(), 2);
        assert_eq!(store.get_unread_count(), 2);
    }

    #[test]
    fn test_mark_read() {
        let store = DashboardStore::new();
        let n = DashboardNotification::new("Test");
        let id = n.id.clone();
        store.add(n);

        assert_eq!(store.get_unread_count(), 1);
        assert!(store.mark_read(&id));
        assert_eq!(store.get_unread_count(), 0);
    }

    #[test]
    fn test_mark_all_read() {
        let store = DashboardStore::new();
        store.add(DashboardNotification::new("A"));
        store.add(DashboardNotification::new("B"));
        store.mark_all_read();
        assert_eq!(store.get_unread_count(), 0);
    }

    #[test]
    fn test_delete() {
        let store = DashboardStore::new();
        let n = DashboardNotification::new("Delete me");
        let id = n.id.clone();
        store.add(n);

        assert!(store.delete(&id));
        assert_eq!(store.get_all().len(), 0);
        assert!(!store.delete("nonexistent"));
    }
}
