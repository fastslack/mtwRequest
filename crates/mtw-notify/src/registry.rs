use dashmap::DashMap;
use mtw_core::MtwError;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::provider::{MtwNotifyProvider, Notification, NotifyStatus};

/// Registry managing notification providers with channel-based routing
pub struct NotifyRegistry {
    providers: DashMap<String, Arc<dyn MtwNotifyProvider>>,
    channel_routes: DashMap<String, String>,
}

impl NotifyRegistry {
    pub fn new() -> Self {
        Self {
            providers: DashMap::new(),
            channel_routes: DashMap::new(),
        }
    }

    /// Register a notification provider
    pub fn register(&self, provider: Arc<dyn MtwNotifyProvider>) {
        let id = provider.id().to_string();
        info!(provider = %id, "registered notification provider");
        self.providers.insert(id, provider);
    }

    /// Unregister a provider by ID
    pub fn unregister(&self, id: &str) -> Option<Arc<dyn MtwNotifyProvider>> {
        self.providers.remove(id).map(|(_, p)| p)
    }

    /// Get a provider by ID
    pub fn get(&self, id: &str) -> Option<Arc<dyn MtwNotifyProvider>> {
        self.providers.get(id).map(|p| p.value().clone())
    }

    /// Map a channel name to a provider ID
    pub fn route_channel(&self, channel: impl Into<String>, provider_id: impl Into<String>) {
        self.channel_routes.insert(channel.into(), provider_id.into());
    }

    /// Send a notification, routing by channel if specified
    pub async fn send(&self, notification: &Notification) -> Result<Vec<String>, MtwError> {
        if let Some(ref channel) = notification.channel {
            if channel == "all" {
                return self.broadcast(notification).await;
            }
            if let Some(provider_id) = self.channel_routes.get(channel) {
                if let Some(provider) = self.providers.get(provider_id.value()) {
                    if provider.is_ready() {
                        let msg_id = provider.send(notification).await?;
                        return Ok(vec![msg_id]);
                    }
                    return Err(MtwError::Internal(format!(
                        "provider {} not ready",
                        provider_id.value()
                    )));
                }
            }
            // Try channel as provider ID directly
            if let Some(provider) = self.providers.get(channel.as_str()) {
                if provider.is_ready() {
                    let msg_id = provider.send(notification).await?;
                    return Ok(vec![msg_id]);
                }
            }
            return Err(MtwError::Internal(format!(
                "no provider found for channel: {}",
                channel
            )));
        }
        // No channel specified: send to first ready provider
        for entry in self.providers.iter() {
            if entry.value().is_ready() {
                let msg_id = entry.value().send(notification).await?;
                return Ok(vec![msg_id]);
            }
        }
        Err(MtwError::Internal("no ready providers".into()))
    }

    /// Broadcast to all ready providers, collecting results
    pub async fn broadcast(&self, notification: &Notification) -> Result<Vec<String>, MtwError> {
        let mut message_ids = Vec::new();
        let mut errors = Vec::new();

        for entry in self.providers.iter() {
            let provider = entry.value();
            if !provider.is_ready() {
                warn!(provider = %provider.id(), "skipping non-ready provider");
                continue;
            }
            match provider.send(notification).await {
                Ok(msg_id) => message_ids.push(msg_id),
                Err(e) => {
                    error!(provider = %provider.id(), error = %e, "broadcast send failed");
                    errors.push(e);
                }
            }
        }

        if message_ids.is_empty() && !errors.is_empty() {
            return Err(MtwError::Internal(format!(
                "all providers failed: {}",
                errors
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        Ok(message_ids)
    }

    /// List all registered provider IDs
    pub fn list_providers(&self) -> Vec<String> {
        self.providers.iter().map(|e| e.key().clone()).collect()
    }

    /// Get status of all providers
    pub fn provider_statuses(&self) -> Vec<NotifyStatus> {
        self.providers.iter().map(|e| e.value().status()).collect()
    }
}

impl Default for NotifyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{NotifyCapabilities, NotifyStatus};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    struct MockProvider {
        provider_id: String,
        ready: AtomicBool,
        send_count: AtomicU32,
    }

    impl MockProvider {
        fn new(id: &str) -> Self {
            Self {
                provider_id: id.to_string(),
                ready: AtomicBool::new(true),
                send_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl MtwNotifyProvider for MockProvider {
        fn id(&self) -> &str {
            &self.provider_id
        }
        fn name(&self) -> &str {
            &self.provider_id
        }
        fn capabilities(&self) -> NotifyCapabilities {
            NotifyCapabilities::default()
        }
        async fn send(&self, _notification: &Notification) -> Result<String, MtwError> {
            self.send_count.fetch_add(1, Ordering::Relaxed);
            Ok(format!("msg-{}", self.send_count.load(Ordering::Relaxed)))
        }
        async fn start(&self) -> Result<(), MtwError> {
            Ok(())
        }
        async fn stop(&self) -> Result<(), MtwError> {
            Ok(())
        }
        fn is_ready(&self) -> bool {
            self.ready.load(Ordering::Relaxed)
        }
        fn status(&self) -> NotifyStatus {
            NotifyStatus {
                provider_id: self.provider_id.clone(),
                connected: true,
                authenticated: true,
                error: None,
                info: Default::default(),
            }
        }
    }

    #[tokio::test]
    async fn test_send_to_channel() {
        let registry = NotifyRegistry::new();
        registry.register(Arc::new(MockProvider::new("telegram")));
        registry.route_channel("alerts", "telegram");

        let n = Notification::new("Test").with_channel("alerts");
        let ids = registry.send(&n).await.unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[tokio::test]
    async fn test_broadcast() {
        let registry = NotifyRegistry::new();
        registry.register(Arc::new(MockProvider::new("telegram")));
        registry.register(Arc::new(MockProvider::new("slack")));

        let n = Notification::new("Broadcast").with_channel("all");
        let ids = registry.send(&n).await.unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn test_direct_provider_id() {
        let registry = NotifyRegistry::new();
        registry.register(Arc::new(MockProvider::new("slack")));

        let n = Notification::new("Direct").with_channel("slack");
        let ids = registry.send(&n).await.unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_list_providers() {
        let registry = NotifyRegistry::new();
        registry.register(Arc::new(MockProvider::new("a")));
        registry.register(Arc::new(MockProvider::new("b")));
        assert_eq!(registry.list_providers().len(), 2);
    }
}
