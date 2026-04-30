//! Cryptographic identity for mtwRequest peers and messages.
//!
//! Provides Ed25519 keypair management, signing of `MtwMessage` instances,
//! and verification of signed messages received from federation peers.
//!
//! The signing (private) key never leaves [`MtwIdentity`]. Use
//! [`MtwIdentity::pubkey`] to share the public part, and
//! [`MtwIdentity::sign_message`] / [`verify_message`] for the message-level API.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use mtw_protocol::MtwMessage;
use rand::rngs::OsRng;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("invalid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("invalid key length: expected {expected}, got {got}")]
    KeyLength { expected: usize, got: usize },
    #[error("signature verification failed")]
    BadSignature,
    #[error("missing pubkey on message")]
    MissingPubkey,
    #[error("missing signature on message")]
    MissingSignature,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("ed25519 error: {0}")]
    Ed25519(String),
}

impl From<ed25519_dalek::SignatureError> for IdentityError {
    fn from(e: ed25519_dalek::SignatureError) -> Self {
        IdentityError::Ed25519(e.to_string())
    }
}

/// A peer/user identity backed by an Ed25519 keypair.
pub struct MtwIdentity {
    signing_key: SigningKey,
}

impl MtwIdentity {
    /// Generate a fresh random identity using the OS RNG.
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::generate(&mut OsRng),
        }
    }

    /// Create an identity from a 32-byte secret seed.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    /// Parse an identity from a hex-encoded 32-byte seed.
    pub fn from_hex(hex_str: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(hex_str)?;
        if bytes.len() != 32 {
            return Err(IdentityError::KeyLength {
                expected: 32,
                got: bytes.len(),
            });
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes);
        Ok(Self::from_seed(seed))
    }

    /// Hex-encoded 32-byte secret seed. Treat as private credential material.
    pub fn to_hex(&self) -> String {
        hex::encode(self.signing_key.to_bytes())
    }

    /// Raw 32-byte secret seed. Treat as private credential material.
    /// Useful for handing the same key to other crypto consumers (e.g. iroh's
    /// `SecretKey::from_bytes`).
    pub fn to_seed_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Public key (32 bytes).
    pub fn pubkey(&self) -> MtwPubkey {
        MtwPubkey(self.signing_key.verifying_key())
    }

    /// Sign arbitrary bytes.
    pub fn sign(&self, bytes: &[u8]) -> MtwSignature {
        MtwSignature(self.signing_key.sign(bytes))
    }

    /// Sign a [`MtwMessage`] in place. Sets `pubkey` and `sig` to canonical
    /// hex-encoded values; the signed bytes are produced by
    /// [`MtwMessage::signing_bytes`].
    pub fn sign_message(&self, msg: &mut MtwMessage) -> Result<(), IdentityError> {
        msg.pubkey = None;
        msg.sig = None;
        let bytes = msg.signing_bytes()?;
        let sig = self.sign(&bytes);
        msg.pubkey = Some(self.pubkey().to_hex());
        msg.sig = Some(sig.to_hex());
        Ok(())
    }

    /// Load an identity from a file (raw hex of the 32-byte seed). Creates the
    /// file with a fresh identity if it does not exist; on Unix the file is
    /// chmod'd to 0600.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let path = path.as_ref();
        if path.exists() {
            let hex_str = std::fs::read_to_string(path)?;
            Self::from_hex(hex_str.trim())
        } else {
            let identity = Self::generate();
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            std::fs::write(path, identity.to_hex())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(path)?.permissions();
                perms.set_mode(0o600);
                std::fs::set_permissions(path, perms)?;
            }
            Ok(identity)
        }
    }
}

/// 32-byte Ed25519 public key — the on-network identity of a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MtwPubkey(VerifyingKey);

impl MtwPubkey {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0.to_bytes())
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(hex_str)?;
        if bytes.len() != 32 {
            return Err(IdentityError::KeyLength {
                expected: 32,
                got: bytes.len(),
            });
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&bytes);
        let vk =
            VerifyingKey::from_bytes(&buf).map_err(|e| IdentityError::Ed25519(e.to_string()))?;
        Ok(Self(vk))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn verify(&self, bytes: &[u8], sig: &MtwSignature) -> Result<(), IdentityError> {
        self.0
            .verify(bytes, &sig.0)
            .map_err(|_| IdentityError::BadSignature)
    }
}

/// 64-byte Ed25519 signature.
#[derive(Debug, Clone, Copy)]
pub struct MtwSignature(Signature);

impl MtwSignature {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0.to_bytes())
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(hex_str)?;
        if bytes.len() != 64 {
            return Err(IdentityError::KeyLength {
                expected: 64,
                got: bytes.len(),
            });
        }
        let mut buf = [0u8; 64];
        buf.copy_from_slice(&bytes);
        Ok(Self(Signature::from_bytes(&buf)))
    }

    pub fn to_bytes(&self) -> [u8; 64] {
        self.0.to_bytes()
    }
}

/// Verify the signature on a [`MtwMessage`].
///
/// * `Ok(true)` — both `pubkey` and `sig` are present and the signature is valid.
/// * `Ok(false)` — both are absent (unsigned message; caller decides policy).
/// * `Err(_)` — malformed inputs, partial signature/pubkey, or bad signature.
pub fn verify_message(msg: &MtwMessage) -> Result<bool, IdentityError> {
    let (pubkey_hex, sig_hex) = match (msg.pubkey.as_ref(), msg.sig.as_ref()) {
        (Some(p), Some(s)) => (p, s),
        (None, None) => return Ok(false),
        (None, Some(_)) => return Err(IdentityError::MissingPubkey),
        (Some(_), None) => return Err(IdentityError::MissingSignature),
    };
    let pubkey = MtwPubkey::from_hex(pubkey_hex)?;
    let sig = MtwSignature::from_hex(sig_hex)?;

    let mut clone = msg.clone();
    clone.pubkey = None;
    clone.sig = None;
    let bytes = clone.signing_bytes()?;

    pubkey.verify(&bytes, &sig)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_protocol::{MtwMessage, Payload};
    use tempfile::tempdir;

    #[test]
    fn generate_and_roundtrip_seed() {
        let id = MtwIdentity::generate();
        let hex = id.to_hex();
        let id2 = MtwIdentity::from_hex(&hex).unwrap();
        assert_eq!(id.pubkey(), id2.pubkey());
    }

    #[test]
    fn sign_and_verify_bytes() {
        let id = MtwIdentity::generate();
        let sig = id.sign(b"hello world");
        id.pubkey().verify(b"hello world", &sig).unwrap();
        assert!(id.pubkey().verify(b"tampered", &sig).is_err());
    }

    #[test]
    fn sign_and_verify_message() {
        let id = MtwIdentity::generate();
        let mut msg = MtwMessage::event("hola");
        id.sign_message(&mut msg).unwrap();

        assert!(msg.pubkey.is_some());
        assert!(msg.sig.is_some());
        assert!(verify_message(&msg).unwrap());
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let id = MtwIdentity::generate();
        let mut msg = MtwMessage::event("hola");
        id.sign_message(&mut msg).unwrap();

        msg.payload = Payload::Text("adios".into());
        assert!(verify_message(&msg).is_err());
    }

    #[test]
    fn unsigned_message_returns_false() {
        let msg = MtwMessage::event("hola");
        assert!(!verify_message(&msg).unwrap());
    }

    #[test]
    fn load_or_create_persists_identity() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("identity.key");

        let id1 = MtwIdentity::load_or_create(&path).unwrap();
        let id2 = MtwIdentity::load_or_create(&path).unwrap();
        assert_eq!(id1.pubkey(), id2.pubkey());
    }

    #[test]
    fn pubkey_substitution_fails() {
        let alice = MtwIdentity::generate();
        let bob = MtwIdentity::generate();
        let mut msg = MtwMessage::event("from alice");
        alice.sign_message(&mut msg).unwrap();

        msg.pubkey = Some(bob.pubkey().to_hex());
        assert!(verify_message(&msg).is_err());
    }

    #[test]
    fn metadata_order_independent() {
        // Two messages with same metadata in different insertion order
        // must produce the same signing bytes (HashMap iter order is unstable).
        let id = MtwIdentity::generate();
        let mut a = MtwMessage::event("test")
            .with_metadata("a", serde_json::json!(1))
            .with_metadata("b", serde_json::json!(2))
            .with_metadata("c", serde_json::json!(3));
        let mut b = MtwMessage::event("test")
            .with_metadata("c", serde_json::json!(3))
            .with_metadata("b", serde_json::json!(2))
            .with_metadata("a", serde_json::json!(1));
        b.id = a.id.clone();
        b.timestamp = a.timestamp;

        assert_eq!(a.signing_bytes().unwrap(), b.signing_bytes().unwrap());

        id.sign_message(&mut a).unwrap();
        // Copy sig/pubkey to b (they target the same canonical bytes)
        b.pubkey = a.pubkey.clone();
        b.sig = a.sig.clone();
        assert!(verify_message(&b).unwrap());
    }
}
