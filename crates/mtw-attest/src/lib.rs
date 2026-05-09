//! Cryptographic attestation for mtw tool calls.
//!
//! ## What this crate is for
//!
//! Every tool call that goes through an mtw MCP server can come back with
//! a *receipt*: a small JSON object that records what was called, what
//! came out, and what side-effects the server claims occurred — signed
//! with the server's Ed25519 identity.
//!
//! Why bother:
//!   * **Audit.** Any third party with the server's public key can verify,
//!     after the fact, that a given tool call really happened with the
//!     stated I/O — without trusting the agent that produced the log.
//!   * **Compliance.** Regulators / security teams get a tamper-evident
//!     trail without trusting application code.
//!   * **Liability separation.** When an agent claims "I sent the email",
//!     there's a signed receipt (or there isn't); the conversation moves
//!     from "trust the LLM transcript" to "verify the cryptography".
//!
//! ## Wire format (`Receipt` v1)
//!
//! Stable JSON. Hash and signature are over the *canonical* serialization
//! (sorted keys, no whitespace). Implementations in any language must
//! produce byte-identical bytes for a given Receipt or signatures won't
//! round-trip — see [`Receipt::canonical_bytes`].
//!
//! ```json
//! {
//!   "v": 1,
//!   "tool": "kernel_emails_send",
//!   "input_hash": "sha256:dead...",
//!   "output_hash": "sha256:beef...",
//!   "side_effects": ["email.sent:3"],
//!   "ts_ms": 1715025600000,
//!   "server_id": "ed25519:9aF2...",
//!   "sig": "base64:..."
//! }
//! ```
//!
//! ## Identity model
//!
//! One Ed25519 keypair per server install (the `Identity` struct). The
//! private key never leaves the host; only the public-key fingerprint
//! (`server_id`) goes on the wire. Rotating means generating a new
//! identity — receipts under the old key remain verifiable as long as
//! the public key is preserved somewhere.
//!
//! ## What this crate does NOT do
//!
//! * **Doesn't enforce side-effects.** The server author lists them in
//!   `side_effects`; the tool itself has to do something the OS or a
//!   downstream system can independently witness for the manifest to
//!   mean anything. This crate just makes that list tamper-evident.
//! * **Doesn't ship a transport.** Receipts attach to MCP `_meta` blocks
//!   in the consumer crate (`mtw-mcp`); this crate is pure data + crypto.
//! * **Doesn't handle revocation.** That's a policy problem solved at the
//!   identity-distribution layer, not here.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub mod canonical;

/// Wire-format version of [`Receipt`]. Bump on any breaking change.
pub const RECEIPT_VERSION: u8 = 1;

/// Errors surfaced by this crate. Kept narrow on purpose — every variant
/// maps to a distinct cause a caller might want to handle differently
/// (bad signature vs. malformed JSON vs. unknown identity).
#[derive(Debug, thiserror::Error)]
pub enum AttestError {
    #[error("invalid base64: {0}")]
    Base64(String),
    #[error("invalid hex: {0}")]
    Hex(String),
    #[error("invalid Ed25519 key or signature: {0}")]
    Crypto(String),
    #[error("signature verification failed")]
    BadSignature,
    #[error("unsupported receipt version: {0}")]
    UnsupportedVersion(u8),
    #[error("malformed server_id: expected `ed25519:<hex32>`, got: {0}")]
    BadServerId(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

// ── Identity ──────────────────────────────────────────────────────────────

/// Long-lived per-install Ed25519 keypair.
///
/// One of these per running mtw server. Persisted to disk on first use and
/// reloaded on every restart — rotation is a manual operation (delete the
/// file, restart, accept the new fingerprint downstream).
#[derive(Clone)]
pub struct Identity {
    signing: SigningKey,
}

impl Identity {
    /// Generate a fresh identity. Use only for tests / first-boot.
    pub fn generate() -> Self {
        let mut rng = OsRng;
        Self { signing: SigningKey::generate(&mut rng) }
    }

    /// Load an identity from raw 32-byte secret material. Most callers
    /// will use [`Identity::from_pem_bytes`] or the persistence helpers.
    pub fn from_secret_bytes(secret: &[u8; 32]) -> Self {
        Self { signing: SigningKey::from_bytes(secret) }
    }

    /// Wire-format server ID (`"ed25519:<hex>"`). Stable across restarts
    /// for the lifetime of the keypair.
    pub fn server_id(&self) -> String {
        format!("ed25519:{}", hex::encode(self.signing.verifying_key().as_bytes()))
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    /// Sign canonical bytes. Caller is responsible for canonicalising —
    /// see [`Receipt::sign`] for the typical path.
    pub fn sign_bytes(&self, msg: &[u8]) -> Signature {
        self.signing.sign(msg)
    }

    /// Persist the secret to a file. Format: 32 raw bytes (no encoding,
    /// no header) — keep on a disk the OS protects (mode 0600).
    pub fn save_to_path(&self, path: impl AsRef<std::path::Path>) -> Result<(), AttestError> {
        std::fs::write(path, self.secret_bytes())?;
        Ok(())
    }

    /// Load an existing identity from a file written by [`save_to_path`].
    pub fn load_from_path(path: impl AsRef<std::path::Path>) -> Result<Self, AttestError> {
        let bytes = std::fs::read(path)?;
        if bytes.len() != 32 {
            return Err(AttestError::Crypto(format!(
                "expected 32 secret bytes, got {}",
                bytes.len()
            )));
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&bytes);
        Ok(Self::from_secret_bytes(&buf))
    }

    /// Load if the file exists, otherwise generate + save. This is the
    /// pattern `mtw-mcp::main` uses at startup.
    pub fn load_or_create(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, AttestError> {
        let path = path.as_ref();
        if path.exists() {
            return Self::load_from_path(path);
        }
        let id = Self::generate();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        id.save_to_path(path)?;
        Ok(id)
    }
}

impl fmt::Debug for Identity {
    /// Never prints the secret. The `Debug` impl shows only the public
    /// fingerprint — pasting an `Identity` into a log file is safe.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity").field("server_id", &self.server_id()).finish()
    }
}

// ── Public-key parsing ────────────────────────────────────────────────────

/// Parse a wire `server_id` (`"ed25519:<hex32>"`) back into a verifying key.
pub fn parse_server_id(id: &str) -> Result<VerifyingKey, AttestError> {
    let suffix = id
        .strip_prefix("ed25519:")
        .ok_or_else(|| AttestError::BadServerId(id.to_string()))?;
    let bytes = hex::decode(suffix).map_err(|e| AttestError::Hex(e.to_string()))?;
    if bytes.len() != 32 {
        return Err(AttestError::BadServerId(id.to_string()));
    }
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&bytes);
    VerifyingKey::from_bytes(&buf).map_err(|e| AttestError::Crypto(e.to_string()))
}

// ── Receipt ───────────────────────────────────────────────────────────────

/// One signed attestation record for a single tool call.
///
/// Field order in this struct doesn't matter for the wire format — what
/// matters is the canonical serialization produced by `canonical_bytes`,
/// which sorts keys and elides defaults. Two implementations in different
/// languages can produce byte-identical canonical bytes as long as they
/// follow the same rules (see [`canonical`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    /// Wire-format version — currently `RECEIPT_VERSION` (= 1).
    pub v: u8,
    /// Name of the tool that ran (e.g. `"kernel_emails_send"`).
    pub tool: String,
    /// `sha256:<hex>` of the canonicalised input arguments.
    pub input_hash: String,
    /// `sha256:<hex>` of the canonicalised output payload.
    pub output_hash: String,
    /// Free-form side-effect manifest declared by the tool. Convention:
    /// `"<resource>.<verb>:<count>"`, e.g. `"email.sent:3"`,
    /// `"calendar.event.created:1"`. Empty array for read-only calls.
    #[serde(default)]
    pub side_effects: Vec<String>,
    /// Unix epoch milliseconds when the call completed.
    pub ts_ms: u64,
    /// Public-key fingerprint of the signer — `"ed25519:<hex32>"`.
    pub server_id: String,
    /// Base64-encoded Ed25519 signature over [`canonical_bytes`].
    pub sig: String,
}

/// Body of a receipt before it's signed. `Receipt` adds `server_id` and
/// `sig` on top of these fields. Kept separate so canonicalisation can
/// hash *just* this body — including the signature in its own input
/// would make signing impossible.
#[derive(Debug, Clone, Serialize)]
struct ReceiptBody<'a> {
    v: u8,
    tool: &'a str,
    input_hash: &'a str,
    output_hash: &'a str,
    side_effects: &'a [String],
    ts_ms: u64,
    server_id: &'a str,
}

impl Receipt {
    /// Build + sign a receipt. Hashes are computed by the caller with
    /// [`hash_json`] so this fn doesn't have to know how the tool args
    /// and result were laid out.
    pub fn sign(
        identity: &Identity,
        tool: impl Into<String>,
        input_hash: impl Into<String>,
        output_hash: impl Into<String>,
        side_effects: Vec<String>,
        ts_ms: u64,
    ) -> Self {
        let tool = tool.into();
        let input_hash = input_hash.into();
        let output_hash = output_hash.into();
        let server_id = identity.server_id();

        let body = ReceiptBody {
            v: RECEIPT_VERSION,
            tool: &tool,
            input_hash: &input_hash,
            output_hash: &output_hash,
            side_effects: &side_effects,
            ts_ms,
            server_id: &server_id,
        };

        // Canonical serialization of the body is what gets signed.
        let bytes = canonical::canonicalize(&serde_json::to_value(&body).unwrap());
        let sig = identity.sign_bytes(&bytes);

        Self {
            v: RECEIPT_VERSION,
            tool,
            input_hash,
            output_hash,
            side_effects,
            ts_ms,
            server_id,
            sig: B64.encode(sig.to_bytes()),
        }
    }

    /// Verify the receipt. Returns `Ok(())` iff the signature is valid
    /// against the embedded `server_id`. Callers concerned with identity
    /// (is *this* server one I trust?) compare `server_id` themselves.
    pub fn verify(&self) -> Result<(), AttestError> {
        if self.v != RECEIPT_VERSION {
            return Err(AttestError::UnsupportedVersion(self.v));
        }

        let vk = parse_server_id(&self.server_id)?;

        let body = ReceiptBody {
            v: self.v,
            tool: &self.tool,
            input_hash: &self.input_hash,
            output_hash: &self.output_hash,
            side_effects: &self.side_effects,
            ts_ms: self.ts_ms,
            server_id: &self.server_id,
        };
        let bytes = canonical::canonicalize(&serde_json::to_value(&body)?);

        let sig_bytes = B64
            .decode(&self.sig)
            .map_err(|e| AttestError::Base64(e.to_string()))?;
        let sig: Signature = Signature::from_slice(&sig_bytes)
            .map_err(|e| AttestError::Crypto(e.to_string()))?;
        vk.verify(&bytes, &sig).map_err(|_| AttestError::BadSignature)
    }

    /// Canonical bytes of the *full* receipt (including signature) — used
    /// when chaining receipts into a Merkle root for run-level audit.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical::canonicalize(&serde_json::to_value(self).unwrap())
    }
}

// ── Hashing helpers ───────────────────────────────────────────────────────

/// `sha256:<hex>` of a JSON value, canonicalised. The canonical form is
/// what guarantees byte-identical hashes across language implementations
/// — see [`canonical`].
pub fn hash_json(value: &serde_json::Value) -> String {
    let bytes = canonical::canonicalize(value);
    let mut h = Sha256::new();
    h.update(&bytes);
    format!("sha256:{}", hex::encode(h.finalize()))
}

/// Convenience: hash an arbitrary Serialize-able value the same way.
pub fn hash<T: Serialize>(value: &T) -> Result<String, AttestError> {
    Ok(hash_json(&serde_json::to_value(value)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_sign_and_verify() {
        let id = Identity::generate();
        let r = Receipt::sign(
            &id,
            "kernel_demo",
            hash_json(&json!({"q": "hi"})),
            hash_json(&json!({"answer": 42})),
            vec!["log.appended:1".into()],
            1715025600000,
        );
        r.verify().expect("must verify");
    }

    #[test]
    fn tampered_output_hash_fails() {
        let id = Identity::generate();
        let mut r = Receipt::sign(
            &id,
            "kernel_demo",
            hash_json(&json!({})),
            hash_json(&json!({"a": 1})),
            vec![],
            42,
        );
        r.output_hash = hash_json(&json!({"a": 2}));
        assert!(matches!(r.verify(), Err(AttestError::BadSignature)));
    }

    #[test]
    fn tampered_side_effects_fails() {
        let id = Identity::generate();
        let mut r = Receipt::sign(
            &id,
            "kernel_demo",
            hash_json(&json!({})),
            hash_json(&json!({})),
            vec!["safe.action:1".into()],
            42,
        );
        r.side_effects.push("scary.action:9000".into());
        assert!(matches!(r.verify(), Err(AttestError::BadSignature)));
    }

    #[test]
    fn unknown_version_rejected() {
        let id = Identity::generate();
        let mut r = Receipt::sign(&id, "x", "sha256:00", "sha256:00", vec![], 0);
        r.v = 99;
        assert!(matches!(r.verify(), Err(AttestError::UnsupportedVersion(99))));
    }

    #[test]
    fn cross_identity_signature_does_not_validate() {
        let alice = Identity::generate();
        let mallory = Identity::generate();
        // Sign as Alice, swap server_id to Mallory's — should fail.
        let mut r = Receipt::sign(&alice, "x", "sha256:00", "sha256:00", vec![], 0);
        r.server_id = mallory.server_id();
        assert!(matches!(r.verify(), Err(AttestError::BadSignature)));
    }

    #[test]
    fn server_id_format_stable() {
        let id = Identity::from_secret_bytes(&[7u8; 32]);
        assert!(id.server_id().starts_with("ed25519:"));
        let suffix = &id.server_id()["ed25519:".len()..];
        assert_eq!(suffix.len(), 64); // 32 bytes hex = 64 chars
    }

    #[test]
    fn debug_does_not_leak_secret() {
        let id = Identity::from_secret_bytes(&[7u8; 32]);
        let dbg = format!("{:?}", id);
        let secret_hex = hex::encode([7u8; 32]);
        assert!(!dbg.contains(&secret_hex), "Debug must not print the secret");
        assert!(dbg.contains("ed25519:"));
    }

    #[test]
    fn hash_json_is_canonical_order_independent() {
        let a = json!({"z": 1, "a": 2});
        let b = json!({"a": 2, "z": 1});
        assert_eq!(hash_json(&a), hash_json(&b));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mtw-attest-test-{}.key", std::process::id()));
        let id = Identity::generate();
        id.save_to_path(&path).unwrap();
        let loaded = Identity::load_from_path(&path).unwrap();
        assert_eq!(id.server_id(), loaded.server_id());
        let _ = std::fs::remove_file(&path);
    }
}
