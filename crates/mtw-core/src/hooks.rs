use crate::error::MtwError;
use async_trait::async_trait;
use mtw_protocol::{ConnId, ConnMetadata, DisconnectReason, MtwMessage};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Lifecycle hooks that modules and users can implement
#[async_trait]
pub trait LifecycleHooks: Send + Sync {
    /// Called when a new connection is established
    async fn on_connect(&self, _conn_id: &ConnId, _meta: &ConnMetadata) -> Result<(), MtwError> {
        Ok(())
    }

    /// Called when a connection is closed
    async fn on_disconnect(
        &self,
        _conn_id: &ConnId,
        _reason: &DisconnectReason,
    ) -> Result<(), MtwError> {
        Ok(())
    }

    /// Called before a message is processed (can modify or reject)
    async fn before_message(
        &self,
        _conn_id: &ConnId,
        msg: MtwMessage,
    ) -> Result<Option<MtwMessage>, MtwError> {
        Ok(Some(msg))
    }

    /// Called after a message has been processed
    async fn after_message(
        &self,
        _conn_id: &ConnId,
        _msg: &MtwMessage,
    ) -> Result<(), MtwError> {
        Ok(())
    }

    /// Called when an error occurs
    async fn on_error(&self, _conn_id: Option<&ConnId>, _error: &MtwError) {
        // Default: do nothing
    }
}

/// Hook registry that chains multiple hook implementations
pub struct HookRegistry {
    hooks: RwLock<Vec<Arc<dyn LifecycleHooks>>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: RwLock::new(Vec::new()),
        }
    }

    pub async fn register(&self, hook: Arc<dyn LifecycleHooks>) {
        self.hooks.write().await.push(hook);
    }

    pub async fn on_connect(&self, conn_id: &ConnId, meta: &ConnMetadata) -> Result<(), MtwError> {
        let hooks = self.hooks.read().await;
        for hook in hooks.iter() {
            hook.on_connect(conn_id, meta).await?;
        }
        Ok(())
    }

    pub async fn on_disconnect(
        &self,
        conn_id: &ConnId,
        reason: &DisconnectReason,
    ) -> Result<(), MtwError> {
        let hooks = self.hooks.read().await;
        for hook in hooks.iter() {
            hook.on_disconnect(conn_id, reason).await?;
        }
        Ok(())
    }

    /// Process message through all hooks. Returns None if any hook rejects the message.
    pub async fn before_message(
        &self,
        conn_id: &ConnId,
        mut msg: MtwMessage,
    ) -> Result<Option<MtwMessage>, MtwError> {
        let hooks = self.hooks.read().await;
        for hook in hooks.iter() {
            match hook.before_message(conn_id, msg).await? {
                Some(transformed) => msg = transformed,
                None => return Ok(None),
            }
        }
        Ok(Some(msg))
    }

    pub async fn after_message(
        &self,
        conn_id: &ConnId,
        msg: &MtwMessage,
    ) -> Result<(), MtwError> {
        let hooks = self.hooks.read().await;
        for hook in hooks.iter() {
            hook.after_message(conn_id, msg).await?;
        }
        Ok(())
    }

    pub async fn on_error(&self, conn_id: Option<&ConnId>, error: &MtwError) {
        let hooks = self.hooks.read().await;
        for hook in hooks.iter() {
            hook.on_error(conn_id, error).await;
        }
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}
