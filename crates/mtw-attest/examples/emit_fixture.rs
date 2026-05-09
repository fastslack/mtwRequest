//! Generate a deterministic receipt fixture for cross-language interop tests.
//! Run: `cargo run -p mtw-attest --example emit_fixture`
//!
//! Writes a canonical JSON object to stdout. Pipe it into the TS test
//! suite to verify byte-for-byte parity between the Rust signer and the
//! TS verifier.

use mtw_attest::{hash_json, Identity, Receipt};
use serde_json::json;

fn main() {
    // Deterministic seed → identity is stable across runs / hosts.
    let id = Identity::from_secret_bytes(&[42u8; 32]);

    let r = Receipt::sign(
        &id,
        "kernel_interop",
        hash_json(&json!({"q": "hi", "n": 7})),
        hash_json(&json!({"answer": 42, "items": [1, 2, 3]})),
        vec!["log.appended:1".into(), "metric.recorded:1".into()],
        1_715_025_600_000,
    );

    println!("{}", serde_json::to_string(&r).unwrap());
}
