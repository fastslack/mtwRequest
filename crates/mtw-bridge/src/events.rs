//! Server-side event bus. Lets bridge tool handlers (and any other
//! Rust-side module that holds a reference) push frames to every
//! currently-connected client over the same Unix socket.
//!
//! ## Wire compatibility
//!
//! Events are framed identically to [`BridgeResponse`] (4-byte BE
//! length prefix + msgpack payload). They are distinguished by the
//! `type: "event"` field. Clients that only know about responses can
//! safely skip frames whose decoded shape lacks an `id`.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::broadcast;

use crate::protocol::BridgeEventFrame;

/// In-memory broadcast channel for `BridgeEventFrame`s. Each connected
/// client subscribes; emissions go to all subscribers. The channel is
/// bounded — slow consumers see `RecvError::Lagged` and skip ahead, they
/// never block emitters.
#[derive(Clone)]
pub struct BridgeEventBus {
    inner: Arc<BridgeEventBusInner>,
}

struct BridgeEventBusInner {
    sender: broadcast::Sender<BridgeEventFrame>,
}

impl BridgeEventBus {
    /// Create a new bus with the given channel capacity (per subscriber).
    /// Defaults to 256 — enough to cover progress bursts during torrent
    /// metadata fetch / first peer handshake.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            inner: Arc::new(BridgeEventBusInner { sender }),
        }
    }

    /// Emit an event under `topic`. Returns the number of subscribers
    /// that received it. Emissions when no client is connected are a
    /// no-op; the caller doesn't have to special-case that.
    pub fn emit(&self, topic: impl Into<String>, data: Value) -> usize {
        let frame = BridgeEventFrame::new(topic, data);
        self.inner.sender.send(frame).unwrap_or(0)
    }

    /// Emit a pre-built frame (lets callers pass through a shared
    /// payload without re-cloning).
    pub fn emit_frame(&self, frame: BridgeEventFrame) -> usize {
        self.inner.sender.send(frame).unwrap_or(0)
    }

    /// Subscribe to the bus. Each call returns an independent receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<BridgeEventFrame> {
        self.inner.sender.subscribe()
    }

    /// Number of currently-attached subscribers (live receivers).
    pub fn subscriber_count(&self) -> usize {
        self.inner.sender.receiver_count()
    }
}

impl Default for BridgeEventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_with_no_subscribers_returns_zero() {
        let bus = BridgeEventBus::default();
        let n = bus.emit("topic.a", serde_json::json!({"x": 1}));
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn subscribers_receive_events() {
        let bus = BridgeEventBus::default();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let n = bus.emit("torrent.progress", serde_json::json!({"p": 0.5}));
        assert_eq!(n, 2);

        let f1 = rx1.recv().await.unwrap();
        let f2 = rx2.recv().await.unwrap();
        assert_eq!(f1.topic, "torrent.progress");
        assert_eq!(f2.topic, "torrent.progress");
        assert_eq!(f1.kind, "event");
        assert_eq!(f1.data["p"], 0.5);
    }

    #[tokio::test]
    async fn dropped_subscriber_decrements_count() {
        let bus = BridgeEventBus::default();
        let rx = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        drop(rx);
        // broadcast::Sender::receiver_count is updated lazily on next op,
        // but at least emit returns 0.
        let n = bus.emit("x", serde_json::Value::Null);
        assert_eq!(n, 0);
    }
}
