//! Shared helpers for mtwRequest benchmarks.
//!
//! The benches themselves live under `benches/` and use Criterion.

use mtw_protocol::{MsgType, MtwMessage, Payload};

/// Build a representative [`MtwMessage`] used across benchmarks. Has a channel,
/// a JSON payload, metadata, and a ref_id — roughly what a real-world event
/// message looks like.
pub fn sample_message(size: usize) -> MtwMessage {
    let body = "x".repeat(size);
    MtwMessage::new(
        MsgType::Event,
        Payload::Json(serde_json::json!({
            "kind": "ticker",
            "symbol": "BTCUSD",
            "price": 68432.17,
            "volume": 42.1,
            "body": body,
        })),
    )
    .with_channel("ticker.btcusd")
    .with_metadata("source", serde_json::json!("bench"))
    .with_metadata("seq", serde_json::json!(1u64))
}
