//! Wire protocol for bridge communication (MessagePack framing)
//!
//! Frame format:
//! ```text
//! [4 bytes: payload length (BE u32)] [N bytes: MessagePack payload]
//! ```

use serde::{Deserialize, Serialize};

/// Request sent from Rust → TypeScript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRequest {
    /// Unique request ID for correlation
    pub id: String,

    /// Tool name to invoke (e.g., "my_app.users.create")
    pub tool: String,

    /// Tool arguments as JSON
    pub args: serde_json::Value,
}

/// Response sent from TypeScript → Rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeResponse {
    /// Correlation ID (matches request.id)
    pub id: String,

    /// Result data (None if error)
    pub result: Option<serde_json::Value>,

    /// Error message (None if success)
    pub error: Option<String>,
}

impl BridgeRequest {
    pub fn new(tool: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            tool: tool.into(),
            args,
        }
    }

    /// Encode to MessagePack bytes with length prefix
    pub fn encode(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        let payload = rmp_serde::to_vec_named(self)?;
        let len = (payload.len() as u32).to_be_bytes();
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&len);
        frame.extend_from_slice(&payload);
        Ok(frame)
    }
}

impl BridgeResponse {
    /// Decode from MessagePack bytes (without length prefix)
    pub fn decode(data: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(data)
    }

    /// Check if response is an error
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Read exactly `len` bytes from a length-prefixed frame
pub fn read_frame_length(header: &[u8; 4]) -> usize {
    u32::from_be_bytes(*header) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_encode_decode() {
        let req = BridgeRequest::new("users.create", serde_json::json!({"name": "Test"}));
        let encoded = req.encode().unwrap();

        // First 4 bytes are length
        let len = read_frame_length(&[encoded[0], encoded[1], encoded[2], encoded[3]]);
        assert_eq!(len + 4, encoded.len());

        // Decode payload (skip length prefix)
        let decoded: BridgeRequest = rmp_serde::from_slice(&encoded[4..]).unwrap();
        assert_eq!(decoded.id, req.id);
        assert_eq!(decoded.tool, "users.create");
    }

    #[test]
    fn test_response_decode() {
        let resp = BridgeResponse {
            id: "test-123".into(),
            result: Some(serde_json::json!({"id": "t1", "title": "Test"})),
            error: None,
        };

        let bytes = rmp_serde::to_vec_named(&resp).unwrap();
        let decoded = BridgeResponse::decode(&bytes).unwrap();

        assert_eq!(decoded.id, "test-123");
        assert!(!decoded.is_error());
        assert_eq!(decoded.result.unwrap()["title"], "Test");
    }

    #[test]
    fn test_error_response() {
        let resp = BridgeResponse {
            id: "test-456".into(),
            result: None,
            error: Some("tool not found".into()),
        };

        let bytes = rmp_serde::to_vec_named(&resp).unwrap();
        let decoded = BridgeResponse::decode(&bytes).unwrap();

        assert!(decoded.is_error());
        assert_eq!(decoded.error.unwrap(), "tool not found");
    }
}
