use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::ExchangeCredentials;

type HmacSha256 = Hmac<Sha256>;

/// Generate Bitvavo authentication headers.
///
/// Returns (timestamp_ms_string, hex_signature, api_key).
///
/// Bitvavo authentication requires:
/// - `Bitvavo-Access-Key`: API key
/// - `Bitvavo-Access-Signature`: HMAC-SHA256(secret, timestamp + method + url + body)
/// - `Bitvavo-Access-Timestamp`: Current timestamp in milliseconds
pub fn sign(
    credentials: &ExchangeCredentials,
    method: &str,
    url_path: &str,
    body: &str,
) -> (String, String, String) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string();

    let message = format!("{}{}{}{}", timestamp, method, url_path, body);

    let mut mac = HmacSha256::new_from_slice(credentials.api_secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(message.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    (timestamp, signature, credentials.api_key.clone())
}

/// Generate WebSocket authentication payload.
///
/// Returns a JSON string for the WebSocket authenticate action.
pub fn ws_auth_payload(credentials: &ExchangeCredentials) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let message = format!("{}GET/v2/websocket", timestamp);

    let mut mac = HmacSha256::new_from_slice(credentials.api_secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(message.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    serde_json::json!({
        "action": "authenticate",
        "key": credentials.api_key,
        "signature": signature,
        "timestamp": timestamp,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_produces_consistent_format() {
        let creds = ExchangeCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
            passphrase: None,
        };
        let (timestamp, signature, key) = sign(&creds, "GET", "/v2/ticker24h", "");
        assert_eq!(key, "test-key");
        assert!(!timestamp.is_empty());
        // HMAC-SHA256 produces 64 hex chars
        assert_eq!(signature.len(), 64);
    }

    #[test]
    fn test_ws_auth_payload_is_valid_json() {
        let creds = ExchangeCredentials {
            api_key: "test-key".into(),
            api_secret: "test-secret".into(),
            passphrase: None,
        };
        let payload = ws_auth_payload(&creds);
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["action"], "authenticate");
        assert_eq!(parsed["key"], "test-key");
        assert!(parsed["signature"].is_string());
        assert!(parsed["timestamp"].is_number());
    }
}
