use async_trait::async_trait;
use mtw_core::MtwError;
use mtw_protocol::{ConnId, MtwMessage};
use std::sync::Arc;

/// Action returned by middleware to control the processing pipeline
#[derive(Debug)]
pub enum MiddlewareAction {
    /// Continue to the next middleware
    Continue(MtwMessage),
    /// Stop the chain — message is consumed
    Halt,
    /// Replace the message entirely
    Transform(MtwMessage),
    /// Redirect to a different channel
    Redirect { channel: String, msg: MtwMessage },
}

/// Context available to middleware during processing
pub struct MiddlewareContext {
    pub conn_id: ConnId,
    pub channel: Option<String>,
}

/// Middleware trait — intercepts messages in the processing pipeline
#[async_trait]
pub trait MtwMiddleware: Send + Sync {
    /// Middleware name
    fn name(&self) -> &str;

    /// Priority (lower = runs first). Default: 100
    fn priority(&self) -> i32 {
        100
    }

    /// Process an inbound message (client → server)
    async fn on_inbound(
        &self,
        msg: MtwMessage,
        ctx: &MiddlewareContext,
    ) -> Result<MiddlewareAction, MtwError>;

    /// Process an outbound message (server → client)
    async fn on_outbound(
        &self,
        msg: MtwMessage,
        _ctx: &MiddlewareContext,
    ) -> Result<MiddlewareAction, MtwError> {
        Ok(MiddlewareAction::Continue(msg))
    }
}

/// Middleware chain — processes messages through an ordered list of middleware
pub struct MiddlewareChain {
    middlewares: Vec<Arc<dyn MtwMiddleware>>,
}

impl MiddlewareChain {
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }

    /// Add a middleware to the chain (sorted by priority)
    pub fn add(&mut self, middleware: Arc<dyn MtwMiddleware>) {
        self.middlewares.push(middleware);
        self.middlewares.sort_by_key(|m| m.priority());
    }

    /// Process an inbound message through the chain
    pub async fn process_inbound(
        &self,
        mut msg: MtwMessage,
        ctx: &MiddlewareContext,
    ) -> Result<Option<MtwMessage>, MtwError> {
        for mw in &self.middlewares {
            match mw.on_inbound(msg, ctx).await? {
                MiddlewareAction::Continue(m) => msg = m,
                MiddlewareAction::Halt => {
                    tracing::debug!(middleware = %mw.name(), "message halted");
                    return Ok(None);
                }
                MiddlewareAction::Transform(m) => msg = m,
                MiddlewareAction::Redirect { channel, msg: mut m } => {
                    m.channel = Some(channel);
                    msg = m;
                }
            }
        }
        Ok(Some(msg))
    }

    /// Process an outbound message through the chain (reverse order)
    pub async fn process_outbound(
        &self,
        mut msg: MtwMessage,
        ctx: &MiddlewareContext,
    ) -> Result<Option<MtwMessage>, MtwError> {
        for mw in self.middlewares.iter().rev() {
            match mw.on_outbound(msg, ctx).await? {
                MiddlewareAction::Continue(m) => msg = m,
                MiddlewareAction::Halt => return Ok(None),
                MiddlewareAction::Transform(m) => msg = m,
                MiddlewareAction::Redirect { channel, msg: mut m } => {
                    m.channel = Some(channel);
                    msg = m;
                }
            }
        }
        Ok(Some(msg))
    }

    /// Number of middlewares in the chain
    pub fn len(&self) -> usize {
        self.middlewares.len()
    }

    pub fn is_empty(&self) -> bool {
        self.middlewares.is_empty()
    }
}

impl Default for MiddlewareChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_protocol::{MsgType, Payload};

    struct PassthroughMiddleware;

    #[async_trait]
    impl MtwMiddleware for PassthroughMiddleware {
        fn name(&self) -> &str {
            "passthrough"
        }
        fn priority(&self) -> i32 {
            50
        }
        async fn on_inbound(
            &self,
            msg: MtwMessage,
            _ctx: &MiddlewareContext,
        ) -> Result<MiddlewareAction, MtwError> {
            Ok(MiddlewareAction::Continue(msg))
        }
    }

    struct BlockingMiddleware;

    #[async_trait]
    impl MtwMiddleware for BlockingMiddleware {
        fn name(&self) -> &str {
            "blocker"
        }
        fn priority(&self) -> i32 {
            10
        }
        async fn on_inbound(
            &self,
            _msg: MtwMessage,
            _ctx: &MiddlewareContext,
        ) -> Result<MiddlewareAction, MtwError> {
            Ok(MiddlewareAction::Halt)
        }
    }

    struct TransformMiddleware;

    #[async_trait]
    impl MtwMiddleware for TransformMiddleware {
        fn name(&self) -> &str {
            "transformer"
        }
        async fn on_inbound(
            &self,
            mut msg: MtwMessage,
            _ctx: &MiddlewareContext,
        ) -> Result<MiddlewareAction, MtwError> {
            msg.payload = Payload::Text("transformed".into());
            Ok(MiddlewareAction::Continue(msg))
        }
    }

    fn make_ctx() -> MiddlewareContext {
        MiddlewareContext {
            conn_id: "test-conn".to_string(),
            channel: None,
        }
    }

    #[tokio::test]
    async fn test_passthrough() {
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(PassthroughMiddleware));

        let msg = MtwMessage::event("hello");
        let ctx = make_ctx();
        let result = chain.process_inbound(msg, &ctx).await.unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().payload.as_text(), Some("hello"));
    }

    #[tokio::test]
    async fn test_blocking() {
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(BlockingMiddleware));
        chain.add(Arc::new(PassthroughMiddleware));

        let msg = MtwMessage::event("hello");
        let ctx = make_ctx();
        let result = chain.process_inbound(msg, &ctx).await.unwrap();

        assert!(result.is_none()); // blocked
    }

    #[tokio::test]
    async fn test_transform() {
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(TransformMiddleware));

        let msg = MtwMessage::event("original");
        let ctx = make_ctx();
        let result = chain.process_inbound(msg, &ctx).await.unwrap();

        assert_eq!(result.unwrap().payload.as_text(), Some("transformed"));
    }

    #[test]
    fn test_priority_ordering() {
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(PassthroughMiddleware)); // priority 50
        chain.add(Arc::new(BlockingMiddleware)); // priority 10

        // Blocker should be first (lower priority number)
        assert_eq!(chain.middlewares[0].name(), "blocker");
        assert_eq!(chain.middlewares[1].name(), "passthrough");
    }
}
