//! Middleware that injects faults into the message pipeline.
//!
//! Useful for two things:
//! * simulating realistic production failures (drop/delay/mutate) so agents
//!   can be trained against a noisy environment;
//! * reusing the mtwRouter middleware chain as a fault-injection *reusable
//!   component* rather than a one-off tool.
//!
//! Rules are matched in order and the first match wins.

use async_trait::async_trait;
use mtw_core::MtwError;
use mtw_protocol::{MtwMessage, Payload};
use mtw_router::middleware::{MiddlewareAction, MiddlewareContext, MtwMiddleware};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A single fault rule.
///
/// `channel` is matched against [`MtwMessage::channel`] — `None` means "match
/// any channel".
#[derive(Debug, Clone)]
pub struct FaultRule {
    pub channel: Option<String>,
    pub action: FaultAction,
}

#[derive(Debug, Clone)]
pub enum FaultAction {
    /// Drop the message entirely.
    Drop,
    /// Delay the message by the given duration before continuing.
    Delay(Duration),
    /// Replace the text payload with `replacement`. Non-text payloads pass
    /// through untouched.
    MutateText { replacement: String },
}

pub struct FaultInjectorMiddleware {
    rules: Vec<FaultRule>,
    /// Counter of how many messages matched a rule — handy for test asserts.
    hits: AtomicU64,
}

impl FaultInjectorMiddleware {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            hits: AtomicU64::new(0),
        }
    }

    pub fn with_rule(mut self, rule: FaultRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    fn match_rule(&self, msg: &MtwMessage) -> Option<&FaultRule> {
        self.rules.iter().find(|r| match &r.channel {
            Some(ch) => msg.channel.as_deref() == Some(ch.as_str()),
            None => true,
        })
    }
}

impl Default for FaultInjectorMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MtwMiddleware for FaultInjectorMiddleware {
    fn name(&self) -> &str {
        "fault_injector"
    }

    fn priority(&self) -> i32 {
        5 // run early so downstream middleware sees the injected state
    }

    async fn on_inbound(
        &self,
        msg: MtwMessage,
        _ctx: &MiddlewareContext,
    ) -> Result<MiddlewareAction, MtwError> {
        let Some(rule) = self.match_rule(&msg) else {
            return Ok(MiddlewareAction::Continue(msg));
        };
        self.hits.fetch_add(1, Ordering::Relaxed);
        match &rule.action {
            FaultAction::Drop => Ok(MiddlewareAction::Halt),
            FaultAction::Delay(dur) => {
                tokio::time::sleep(*dur).await;
                Ok(MiddlewareAction::Continue(msg))
            }
            FaultAction::MutateText { replacement } => {
                let mut msg = msg;
                if matches!(msg.payload, Payload::Text(_)) {
                    msg.payload = Payload::Text(replacement.clone());
                }
                Ok(MiddlewareAction::Transform(msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_router::middleware::MiddlewareChain;
    use std::sync::Arc;

    fn ctx() -> MiddlewareContext {
        MiddlewareContext {
            conn_id: "test".into(),
            channel: None,
        }
    }

    #[tokio::test]
    async fn drop_halts_chain() {
        let mw = FaultInjectorMiddleware::new().with_rule(FaultRule {
            channel: None,
            action: FaultAction::Drop,
        });
        let mw = Arc::new(mw);
        let mut chain = MiddlewareChain::new();
        chain.add(mw.clone());
        let out = chain
            .process_inbound(MtwMessage::event("x"), &ctx())
            .await
            .unwrap();
        assert!(out.is_none());
        assert_eq!(mw.hits(), 1);
    }

    #[tokio::test]
    async fn mutate_replaces_text() {
        let mw = FaultInjectorMiddleware::new().with_rule(FaultRule {
            channel: None,
            action: FaultAction::MutateText {
                replacement: "INJECTED".into(),
            },
        });
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(mw));
        let out = chain
            .process_inbound(MtwMessage::event("original"), &ctx())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(out.payload.as_text(), Some("INJECTED"));
    }

    #[tokio::test]
    async fn channel_filter_is_respected() {
        let mw = FaultInjectorMiddleware::new().with_rule(FaultRule {
            channel: Some("evidence.db".into()),
            action: FaultAction::Drop,
        });
        let mw = Arc::new(mw);
        let mut chain = MiddlewareChain::new();
        chain.add(mw.clone());

        let unmatched = MtwMessage::event("x").with_channel("evidence.net");
        assert!(chain
            .process_inbound(unmatched, &ctx())
            .await
            .unwrap()
            .is_some());
        assert_eq!(mw.hits(), 0);

        let matched = MtwMessage::event("x").with_channel("evidence.db");
        assert!(chain
            .process_inbound(matched, &ctx())
            .await
            .unwrap()
            .is_none());
        assert_eq!(mw.hits(), 1);
    }

    #[tokio::test]
    async fn delay_is_honoured() {
        let mw = FaultInjectorMiddleware::new().with_rule(FaultRule {
            channel: None,
            action: FaultAction::Delay(Duration::from_millis(20)),
        });
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(mw));
        let t0 = std::time::Instant::now();
        let _ = chain
            .process_inbound(MtwMessage::event("x"), &ctx())
            .await
            .unwrap();
        assert!(t0.elapsed() >= Duration::from_millis(15));
    }
}
