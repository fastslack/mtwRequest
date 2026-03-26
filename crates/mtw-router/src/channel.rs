use dashmap::DashMap;
use mtw_core::MtwError;
use mtw_protocol::{ConnId, MtwMessage};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Channel subscriber info
#[derive(Debug, Clone)]
pub struct Subscriber {
    pub conn_id: ConnId,
    pub subscribed_at: u64,
}

/// A pub/sub channel
pub struct Channel {
    /// Channel name (supports glob patterns like "chat.*")
    name: String,
    /// Whether authentication is required
    auth_required: bool,
    /// Maximum number of members (None = unlimited)
    max_members: Option<usize>,
    /// Message history size to keep
    history_size: usize,
    /// Active subscribers
    subscribers: DashMap<ConnId, Subscriber>,
    /// Message history ring buffer
    history: tokio::sync::RwLock<Vec<MtwMessage>>,
    /// Channel for outgoing messages to transport
    message_tx: mpsc::UnboundedSender<(ConnId, MtwMessage)>,
}

impl Channel {
    pub fn new(
        name: impl Into<String>,
        auth_required: bool,
        max_members: Option<usize>,
        history_size: usize,
        message_tx: mpsc::UnboundedSender<(ConnId, MtwMessage)>,
    ) -> Self {
        Self {
            name: name.into(),
            auth_required,
            max_members,
            history_size,
            subscribers: DashMap::new(),
            history: tokio::sync::RwLock::new(Vec::new()),
            message_tx,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn auth_required(&self) -> bool {
        self.auth_required
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    /// Subscribe a connection to this channel
    pub fn subscribe(&self, conn_id: &ConnId) -> Result<(), MtwError> {
        if let Some(max) = self.max_members {
            if self.subscribers.len() >= max {
                return Err(MtwError::Router(format!(
                    "channel '{}' is full (max: {})",
                    self.name, max
                )));
            }
        }

        let subscriber = Subscriber {
            conn_id: conn_id.clone(),
            subscribed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        self.subscribers.insert(conn_id.clone(), subscriber);
        tracing::debug!(channel = %self.name, conn_id = %conn_id, "subscribed");
        Ok(())
    }

    /// Unsubscribe a connection from this channel
    pub fn unsubscribe(&self, conn_id: &ConnId) -> bool {
        let removed = self.subscribers.remove(conn_id).is_some();
        if removed {
            tracing::debug!(channel = %self.name, conn_id = %conn_id, "unsubscribed");
        }
        removed
    }

    /// Check if a connection is subscribed
    pub fn is_subscribed(&self, conn_id: &ConnId) -> bool {
        self.subscribers.contains_key(conn_id)
    }

    /// Publish a message to all subscribers
    pub async fn publish(&self, msg: MtwMessage, exclude: Option<&ConnId>) -> Result<usize, MtwError> {
        // Store in history
        if self.history_size > 0 {
            let mut history = self.history.write().await;
            history.push(msg.clone());
            if history.len() > self.history_size {
                history.remove(0);
            }
        }

        let mut sent = 0;
        for entry in self.subscribers.iter() {
            if let Some(excluded) = exclude {
                if entry.key() == excluded {
                    continue;
                }
            }

            if self.message_tx.send((entry.key().clone(), msg.clone())).is_ok() {
                sent += 1;
            }
        }

        Ok(sent)
    }

    /// Get message history
    pub async fn get_history(&self, limit: Option<usize>) -> Vec<MtwMessage> {
        let history = self.history.read().await;
        match limit {
            Some(n) => history.iter().rev().take(n).cloned().collect(),
            None => history.clone(),
        }
    }

    /// Get all subscriber connection IDs
    pub fn subscribers(&self) -> Vec<ConnId> {
        self.subscribers.iter().map(|e| e.key().clone()).collect()
    }

    /// Remove a connection from all tracking (called on disconnect)
    pub fn remove_connection(&self, conn_id: &ConnId) {
        self.subscribers.remove(conn_id);
    }
}

/// Channel manager — handles multiple channels with glob pattern matching
pub struct ChannelManager {
    channels: DashMap<String, Arc<Channel>>,
    message_tx: mpsc::UnboundedSender<(ConnId, MtwMessage)>,
    message_rx: Option<mpsc::UnboundedReceiver<(ConnId, MtwMessage)>>,
}

impl ChannelManager {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            channels: DashMap::new(),
            message_tx: tx,
            message_rx: Some(rx),
        }
    }

    /// Take the message receiver (for the transport layer to consume)
    pub fn take_message_receiver(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<(ConnId, MtwMessage)>> {
        self.message_rx.take()
    }

    /// Create a new channel
    pub fn create_channel(
        &self,
        name: impl Into<String>,
        auth_required: bool,
        max_members: Option<usize>,
        history_size: usize,
    ) -> Arc<Channel> {
        let name = name.into();
        let channel = Arc::new(Channel::new(
            name.clone(),
            auth_required,
            max_members,
            history_size,
            self.message_tx.clone(),
        ));
        self.channels.insert(name, channel.clone());
        channel
    }

    /// Get or create a channel
    pub fn get_or_create(&self, name: &str) -> Arc<Channel> {
        if let Some(channel) = self.channels.get(name) {
            channel.value().clone()
        } else {
            self.create_channel(name, false, None, 50)
        }
    }

    /// Get a channel by name
    pub fn get(&self, name: &str) -> Option<Arc<Channel>> {
        self.channels.get(name).map(|e| e.value().clone())
    }

    /// Find channels matching a glob pattern (e.g., "chat.*")
    pub fn find_matching(&self, pattern: &str) -> Vec<Arc<Channel>> {
        if !pattern.contains('*') {
            return self.get(pattern).into_iter().collect();
        }

        self.channels
            .iter()
            .filter(|entry| {
                // Simple glob matching
                Self::glob_match(pattern, entry.key())
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Simple glob pattern matching
    fn glob_match(pattern: &str, text: &str) -> bool {
        let pattern_parts: Vec<&str> = pattern.split('.').collect();
        let text_parts: Vec<&str> = text.split('.').collect();

        if pattern_parts.len() != text_parts.len() {
            return false;
        }

        pattern_parts
            .iter()
            .zip(text_parts.iter())
            .all(|(p, t)| *p == "*" || p == t)
    }

    /// Subscribe a connection to a channel
    pub fn subscribe(&self, channel_name: &str, conn_id: &ConnId) -> Result<(), MtwError> {
        let channel = self.get_or_create(channel_name);
        channel.subscribe(conn_id)
    }

    /// Unsubscribe a connection from a channel
    pub fn unsubscribe(&self, channel_name: &str, conn_id: &ConnId) -> bool {
        if let Some(channel) = self.get(channel_name) {
            channel.unsubscribe(conn_id)
        } else {
            false
        }
    }

    /// Remove a connection from all channels (on disconnect)
    pub fn remove_connection(&self, conn_id: &ConnId) {
        for entry in self.channels.iter() {
            entry.value().remove_connection(conn_id);
        }
    }

    /// List all channel names
    pub fn list_channels(&self) -> Vec<String> {
        self.channels.iter().map(|e| e.key().clone()).collect()
    }

    /// Delete a channel
    pub fn delete_channel(&self, name: &str) -> bool {
        self.channels.remove(name).is_some()
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_protocol::{MsgType, Payload};

    fn make_manager() -> ChannelManager {
        ChannelManager::new()
    }

    #[test]
    fn test_create_channel() {
        let mgr = make_manager();
        let ch = mgr.create_channel("chat.general", false, Some(100), 50);
        assert_eq!(ch.name(), "chat.general");
        assert_eq!(ch.subscriber_count(), 0);
    }

    #[test]
    fn test_subscribe_unsubscribe() {
        let mgr = make_manager();
        mgr.create_channel("test", false, None, 0);

        mgr.subscribe("test", &"conn1".to_string()).unwrap();
        assert!(mgr.get("test").unwrap().is_subscribed(&"conn1".to_string()));

        mgr.unsubscribe("test", &"conn1".to_string());
        assert!(!mgr.get("test").unwrap().is_subscribed(&"conn1".to_string()));
    }

    #[test]
    fn test_max_members() {
        let mgr = make_manager();
        mgr.create_channel("small", false, Some(2), 0);

        mgr.subscribe("small", &"conn1".to_string()).unwrap();
        mgr.subscribe("small", &"conn2".to_string()).unwrap();

        let result = mgr.subscribe("small", &"conn3".to_string());
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_publish() {
        let mut mgr = make_manager();
        let _rx = mgr.take_message_receiver();

        mgr.create_channel("test", false, None, 10);
        mgr.subscribe("test", &"conn1".to_string()).unwrap();
        mgr.subscribe("test", &"conn2".to_string()).unwrap();

        let msg = MtwMessage::new(MsgType::Publish, Payload::Text("hello".into()))
            .with_channel("test");

        let ch = mgr.get("test").unwrap();
        let sent = ch.publish(msg, None).await.unwrap();
        assert_eq!(sent, 2);
    }

    #[tokio::test]
    async fn test_history() {
        let mgr = make_manager();
        let ch = mgr.create_channel("test", false, None, 3);

        for i in 0..5 {
            let msg = MtwMessage::event(format!("msg-{}", i));
            ch.publish(msg, None).await.unwrap();
        }

        let history = ch.get_history(None).await;
        assert_eq!(history.len(), 3); // only last 3 kept
    }

    #[test]
    fn test_glob_matching() {
        assert!(ChannelManager::glob_match("chat.*", "chat.general"));
        assert!(ChannelManager::glob_match("chat.*", "chat.random"));
        assert!(!ChannelManager::glob_match("chat.*", "chat.sub.deep"));
        assert!(!ChannelManager::glob_match("chat.*", "other.channel"));
        assert!(ChannelManager::glob_match("*.*", "any.thing"));
    }

    #[test]
    fn test_remove_connection() {
        let mgr = make_manager();
        mgr.create_channel("ch1", false, None, 0);
        mgr.create_channel("ch2", false, None, 0);

        let conn = "conn1".to_string();
        mgr.subscribe("ch1", &conn).unwrap();
        mgr.subscribe("ch2", &conn).unwrap();

        mgr.remove_connection(&conn);

        assert!(!mgr.get("ch1").unwrap().is_subscribed(&conn));
        assert!(!mgr.get("ch2").unwrap().is_subscribed(&conn));
    }
}
