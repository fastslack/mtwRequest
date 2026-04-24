//! Shared, lazily-encoded envelopes for pub/sub fanout.
//!
//! The hot path for `Channel::publish` serializes the same `MtwMessage` to
//! every subscriber. Naïvely that means N serializations per fanout. A
//! `SharedEnvelope` encodes the message exactly once per wire format on
//! first use, caches the result, and hands out cheap `Bytes`/`String`
//! clones (refcount bumps) to each subscriber.
//!
//! Two wire forms are cached independently:
//!   - `text()`   — JSON string (UTF-8 guaranteed by `serde_json`)
//!   - `binary()` — MTW binary frame bytes (`mtw_protocol::Frame`)
//!
//! Encoding is on-demand: a channel whose subscribers all speak JSON never
//! pays the binary-framing cost, and vice versa.

use crate::{ConnId, Frame, MtwMessage};
use bytes::Bytes;
use std::sync::{Arc, OnceLock};

/// A message wrapped for broadcast, with lazily-cached wire encodings.
///
/// All three wire forms are backed by `bytes::Bytes` so every subscriber's
/// delivery gets a refcounted clone — O(1), no allocation, no memcpy.
/// Each cache is independent: a channel whose subscribers all speak JSON
/// never pays for MsgPack framing, and vice versa.
pub struct SharedEnvelope {
    pub message: MtwMessage,
    /// Cached JSON bytes (UTF-8 guaranteed by `serde_json`).
    text: OnceLock<Bytes>,
    /// Cached MTW binary frame bytes.
    binary: OnceLock<Bytes>,
    /// Cached MsgPack bytes (via `rmp-serde`). ~40-50 % smaller than JSON
    /// and significantly faster to encode/decode — the wire format used
    /// when a client negotiates `Sec-WebSocket-Protocol: mtw.msgpack.v1`.
    msgpack: OnceLock<Bytes>,
}

impl SharedEnvelope {
    pub fn new(message: MtwMessage) -> Self {
        Self {
            message,
            text: OnceLock::new(),
            binary: OnceLock::new(),
            msgpack: OnceLock::new(),
        }
    }

    /// JSON-serialized bytes. First caller pays the `serde_json` cost; every
    /// subsequent caller gets a refcounted `Bytes` clone.
    ///
    /// The contents are always valid UTF-8 (invariant of `serde_json::to_vec`).
    /// Transports that need `tungstenite::Utf8Bytes` can use the unchecked
    /// constructor safely since this guarantee holds.
    pub fn text_bytes(&self) -> Bytes {
        self.text
            .get_or_init(|| Bytes::from(serde_json::to_vec(&self.message).unwrap_or_default()))
            .clone()
    }

    /// JSON-serialized form as a string slice, for callers that prefer `&str`.
    pub fn text(&self) -> &str {
        let bytes = self
            .text
            .get_or_init(|| Bytes::from(serde_json::to_vec(&self.message).unwrap_or_default()));
        // SAFETY: `serde_json::to_vec` always emits valid UTF-8.
        unsafe { std::str::from_utf8_unchecked(bytes) }
    }

    /// MTW binary frame form. First caller pays the framing cost; every
    /// subsequent caller gets a cheap `Bytes` clone (refcount bump).
    pub fn binary(&self) -> Bytes {
        self.binary
            .get_or_init(|| Frame::encode_message(&self.message).unwrap_or_default())
            .clone()
    }

    /// MsgPack-encoded bytes. About 40-50 % smaller than JSON and
    /// ~3× faster to encode thanks to native-typed integers and strings.
    /// Sent on the wire as a WebSocket binary frame.
    pub fn msgpack(&self) -> Bytes {
        self.msgpack
            .get_or_init(|| {
                rmp_serde::to_vec_named(&self.message)
                    .map(Bytes::from)
                    .unwrap_or_default()
            })
            .clone()
    }
}

/// Delivery sink for channel broadcast. Implemented by transports so
/// `Channel::publish` can enqueue directly into each connection's writer
/// queue without going through a central forwarder mpsc.
///
/// Must be cheap and non-blocking: delivery into the per-connection
/// writer is expected to be a single `mpsc::send`, not network IO.
///
/// `resolve()` is the fast path: called once per subscriber at subscribe
/// time, it returns a handle that captures everything the transport needs
/// (typically the per-connection `Sender` and wire-format flag). The
/// channel stores this handle in its subscriber snapshot and calls
/// `ConnTarget::deliver` on every broadcast — zero DashMap lookups per
/// delivery.
///
/// `deliver()` remains as the fallback path for the rare case where a
/// subscribe happened before the sink was installed, or when the target
/// cannot be cheaply cached (e.g. transports with non-stable addressing).
pub trait EnvelopeSink: Send + Sync {
    fn deliver(&self, conn_id: &ConnId, envelope: &Arc<SharedEnvelope>);

    /// Resolve a connection to a pre-bound `ConnTarget` once, so subsequent
    /// deliveries can skip the conn-id → sender lookup entirely. Default
    /// impl returns `None`, forcing the slow `deliver()` path.
    fn resolve(&self, _conn_id: &ConnId) -> Option<Arc<dyn ConnTarget>> {
        None
    }
}

/// Pre-bound delivery handle for a single connection. Issued by
/// `EnvelopeSink::resolve` at subscribe time and invoked on every
/// broadcast.
///
/// Cloning a `Arc<dyn ConnTarget>` is a refcount bump; the same target is
/// shared by every `Channel` this connection is subscribed to.
///
/// The envelope is passed by reference so the broadcast loop doesn't pay
/// an `Arc::clone` per subscriber. Implementations typically only need
/// the envelope's cached wire bytes (`text_bytes()` / `binary()`), each
/// of which is itself a cheap refcount bump on `Bytes`.
pub trait ConnTarget: Send + Sync {
    /// Deliver an envelope to this specific connection. Must be
    /// non-blocking (typically a single `mpsc::send`).
    fn deliver(&self, envelope: &Arc<SharedEnvelope>);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MsgType, Payload};

    #[test]
    fn text_is_cached() {
        let env = SharedEnvelope::new(MtwMessage::new(MsgType::Event, Payload::Text("hi".into())));
        let a = env.text_bytes();
        let b = env.text_bytes();
        assert_eq!(a.as_ptr(), b.as_ptr(), "text_bytes() should share the same Bytes arc");
    }

    #[test]
    fn text_slice_matches_bytes() {
        let env = SharedEnvelope::new(MtwMessage::new(MsgType::Event, Payload::Text("hi".into())));
        assert_eq!(env.text().as_bytes(), &env.text_bytes()[..]);
    }

    #[test]
    fn binary_is_cached() {
        let env = SharedEnvelope::new(MtwMessage::new(MsgType::Event, Payload::Text("hi".into())));
        let a = env.binary();
        let b = env.binary();
        assert_eq!(a.as_ptr(), b.as_ptr(), "binary() should hand out the same Bytes arc");
    }
}
