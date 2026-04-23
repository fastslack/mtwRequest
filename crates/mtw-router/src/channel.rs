use arc_swap::ArcSwap;
use dashmap::DashMap;
use mtw_core::MtwError;
use mtw_protocol::{ConnId, ConnTarget, EnvelopeSink, MtwMessage, SharedEnvelope};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

/// Channel subscriber info. `target` is resolved at subscribe time (when a
/// direct sink is installed) and kept through the subscriber's lifetime so
/// broadcast deliveries never pay for a conn-id → sender lookup.
#[derive(Clone)]
pub struct Subscriber {
    pub conn_id: ConnId,
    pub subscribed_at: u64,
    pub target: Option<Arc<dyn ConnTarget>>,
}

impl std::fmt::Debug for Subscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscriber")
            .field("conn_id", &self.conn_id)
            .field("subscribed_at", &self.subscribed_at)
            .field("target", &self.target.as_ref().map(|_| "..."))
            .finish()
    }
}

/// Snapshot entry used by the publish hot path. Carries the pre-bound
/// delivery target so iterating the snapshot is read-only and lock-free.
#[derive(Clone)]
pub struct SubscriberEntry {
    pub conn_id: ConnId,
    pub target: Option<Arc<dyn ConnTarget>>,
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
    /// Active subscribers (keyed lookup for sub/unsub/is_subscribed).
    subscribers: DashMap<ConnId, Subscriber>,
    /// Immutable snapshot of subscriber entries, each carrying a pre-bound
    /// `ConnTarget`. Swapped atomically on every sub/unsub. `publish` reads
    /// it via a single atomic load and calls `target.deliver()` directly —
    /// no DashMap lookups on the broadcast hot path.
    subscriber_list: ArcSwap<Vec<SubscriberEntry>>,
    /// Message history ring buffer
    history: tokio::sync::RwLock<std::collections::VecDeque<MtwMessage>>,
    /// Fallback delivery path: a central mpsc drained by a forwarder task.
    /// Used when `direct_sink` is unset.
    message_tx: mpsc::UnboundedSender<(ConnId, Arc<SharedEnvelope>)>,
    /// Hot-path delivery: if set, `publish` enqueues envelopes directly into
    /// each connection's writer queue via the sink, bypassing `message_tx`
    /// and its forwarder task entirely.
    direct_sink: OnceLock<Arc<dyn EnvelopeSink>>,
}

impl Channel {
    pub fn new(
        name: impl Into<String>,
        auth_required: bool,
        max_members: Option<usize>,
        history_size: usize,
        message_tx: mpsc::UnboundedSender<(ConnId, Arc<SharedEnvelope>)>,
    ) -> Self {
        Self {
            name: name.into(),
            auth_required,
            max_members,
            history_size,
            subscribers: DashMap::new(),
            subscriber_list: ArcSwap::from_pointee(Vec::new()),
            history: tokio::sync::RwLock::new(std::collections::VecDeque::new()),
            message_tx,
            direct_sink: OnceLock::new(),
        }
    }

    /// Install a direct delivery sink. Call once, before any `publish`.
    /// If subscribers already exist, their cached `ConnTarget`s are
    /// back-filled via `sink.resolve` and the snapshot is refreshed.
    pub fn set_direct_sink(&self, sink: Arc<dyn EnvelopeSink>) {
        if self.direct_sink.set(sink.clone()).is_err() {
            return;
        }
        // Back-fill in two phases to avoid holding DashMap write locks
        // while calling into `sink.resolve` (which may take its own locks).
        let unresolved: Vec<ConnId> = self
            .subscribers
            .iter()
            .filter(|e| e.value().target.is_none())
            .map(|e| e.key().clone())
            .collect();

        let mut touched = false;
        for conn_id in unresolved {
            if let Some(target) = sink.resolve(&conn_id) {
                if let Some(mut entry) = self.subscribers.get_mut(&conn_id) {
                    if entry.target.is_none() {
                        entry.target = Some(target);
                        touched = true;
                    }
                }
            }
        }
        if touched {
            self.refresh_subscriber_list();
        }
    }

    /// Rebuild the immutable subscriber snapshot after a membership change.
    /// Entries keep the `ConnTarget` resolved at subscribe time, so publish
    /// deliveries don't pay for any lookup.
    fn refresh_subscriber_list(&self) {
        let list: Vec<SubscriberEntry> = self
            .subscribers
            .iter()
            .map(|e| SubscriberEntry {
                conn_id: e.key().clone(),
                target: e.value().target.clone(),
            })
            .collect();
        self.subscriber_list.store(Arc::new(list));
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

        // Resolve the delivery target once. If no direct sink is installed
        // yet, the fallback publish path will route through the mpsc.
        let target = self
            .direct_sink
            .get()
            .and_then(|sink| sink.resolve(conn_id));

        let subscriber = Subscriber {
            conn_id: conn_id.clone(),
            subscribed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            target,
        };

        self.subscribers.insert(conn_id.clone(), subscriber);
        self.refresh_subscriber_list();
        tracing::debug!(channel = %self.name, conn_id = %conn_id, "subscribed");
        Ok(())
    }

    /// Unsubscribe a connection from this channel
    pub fn unsubscribe(&self, conn_id: &ConnId) -> bool {
        let removed = self.subscribers.remove(conn_id).is_some();
        if removed {
            self.refresh_subscriber_list();
            tracing::debug!(channel = %self.name, conn_id = %conn_id, "unsubscribed");
        }
        removed
    }

    /// Check if a connection is subscribed
    pub fn is_subscribed(&self, conn_id: &ConnId) -> bool {
        self.subscribers.contains_key(conn_id)
    }

    /// Publish a message to all subscribers.
    ///
    /// Hot path: the message is wrapped in a `SharedEnvelope` (lazy text +
    /// binary encoding) and, if a `direct_sink` is installed, delivered
    /// straight into each connection's writer queue with no intermediate
    /// mpsc hop. The subscriber list is read via a single atomic load
    /// (`ArcSwap`), so publish holds zero locks.
    ///
    /// Fallback path (no sink installed): the envelope is pushed into the
    /// central `message_tx` mpsc and a forwarder task relays it.
    pub async fn publish(&self, msg: MtwMessage, exclude: Option<&ConnId>) -> Result<usize, MtwError> {
        // Store in history (history still wants the decoded MtwMessage).
        if self.history_size > 0 {
            let mut history = self.history.write().await;
            history.push_back(msg.clone());
            while history.len() > self.history_size {
                history.pop_front();
            }
        }

        // Single atomic load: no DashMap shard locks on the hot path.
        let snapshot = self.subscriber_list.load();
        if snapshot.is_empty() {
            return Ok(0);
        }

        let envelope = Arc::new(SharedEnvelope::new(msg));
        let direct_sink = self.direct_sink.get();

        let mut sent = 0;
        for entry in snapshot.iter() {
            if let Some(excluded) = exclude {
                if &entry.conn_id == excluded {
                    continue;
                }
            }
            // Fast path: target was resolved at subscribe time, so
            // delivery is one virtual call + one `mpsc::send`. Pass the
            // envelope by reference — the target only needs the cached
            // wire bytes, not a new Arc refcount bump.
            if let Some(target) = &entry.target {
                target.deliver(&envelope);
                sent += 1;
                continue;
            }
            // Fallback: sink was installed but target was never resolved
            // (e.g. the conn went away between subscribe and now).
            if let Some(sink) = direct_sink {
                sink.deliver(&entry.conn_id, &envelope);
                sent += 1;
                continue;
            }
            // Last resort: no direct sink at all, route through the
            // central mpsc forwarder (Arc clone needed here — the queue
            // must own an Arc for the forwarder to consume later).
            if self
                .message_tx
                .send((entry.conn_id.clone(), Arc::clone(&envelope)))
                .is_ok()
            {
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
            None => history.iter().cloned().collect(),
        }
    }

    /// Get all subscriber connection IDs
    pub fn subscribers(&self) -> Vec<ConnId> {
        self.subscribers.iter().map(|e| e.key().clone()).collect()
    }

    /// Remove a connection from all tracking (called on disconnect)
    pub fn remove_connection(&self, conn_id: &ConnId) {
        if self.subscribers.remove(conn_id).is_some() {
            self.refresh_subscriber_list();
        }
    }
}

/// Channel manager — handles multiple channels with glob pattern matching
pub struct ChannelManager {
    channels: DashMap<String, Arc<Channel>>,
    /// Reverse index: conn_id → set of channel names this connection is subscribed to.
    /// Enables O(subscriptions) disconnect instead of O(all_channels).
    conn_channels: DashMap<ConnId, std::collections::HashSet<String>>,
    message_tx: mpsc::UnboundedSender<(ConnId, Arc<SharedEnvelope>)>,
    message_rx: Option<mpsc::UnboundedReceiver<(ConnId, Arc<SharedEnvelope>)>>,
    /// Direct delivery sink, propagated to every channel created after it's
    /// installed. Channels created *before* the sink is installed are
    /// back-filled at install time.
    direct_sink: OnceLock<Arc<dyn EnvelopeSink>>,
}

impl ChannelManager {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            channels: DashMap::new(),
            conn_channels: DashMap::new(),
            message_tx: tx,
            message_rx: Some(rx),
            direct_sink: OnceLock::new(),
        }
    }

    /// Install the direct delivery sink. Once set, every `publish` across
    /// every channel (existing or future) delivers straight into the
    /// transport's per-connection writer queue.
    pub fn set_direct_sink(&self, sink: Arc<dyn EnvelopeSink>) {
        // First set wins; ignore subsequent calls.
        if self.direct_sink.set(sink.clone()).is_err() {
            return;
        }
        for entry in self.channels.iter() {
            entry.value().set_direct_sink(sink.clone());
        }
    }

    /// Take the message receiver (for the transport layer to consume)
    pub fn take_message_receiver(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<(ConnId, Arc<SharedEnvelope>)>> {
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
        if let Some(sink) = self.direct_sink.get() {
            channel.set_direct_sink(sink.clone());
        }
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
        channel.subscribe(conn_id)?;
        self.conn_channels
            .entry(conn_id.clone())
            .or_default()
            .insert(channel_name.to_string());
        Ok(())
    }

    /// Unsubscribe a connection from a channel
    pub fn unsubscribe(&self, channel_name: &str, conn_id: &ConnId) -> bool {
        if let Some(channel) = self.get(channel_name) {
            let removed = channel.unsubscribe(conn_id);
            if removed {
                if let Some(mut set) = self.conn_channels.get_mut(conn_id) {
                    set.remove(channel_name);
                }
            }
            removed
        } else {
            false
        }
    }

    /// Remove a connection from all channels (on disconnect).
    /// Uses reverse index for O(subscriptions) instead of O(all_channels).
    pub fn remove_connection(&self, conn_id: &ConnId) {
        if let Some((_, channel_names)) = self.conn_channels.remove(conn_id) {
            for name in &channel_names {
                if let Some(channel) = self.channels.get(name) {
                    channel.value().remove_connection(conn_id);
                }
            }
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
