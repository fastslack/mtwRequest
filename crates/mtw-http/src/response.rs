use bytes::Bytes;
use mtw_core::MtwError;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The body of an HTTP response.
#[derive(Debug, Clone)]
pub enum ResponseBody {
    /// Raw bytes.
    Bytes(Bytes),
    /// Already-parsed JSON (stored by JsonParseStage).
    Json(serde_json::Value),
    /// Empty body.
    Empty,
}

impl ResponseBody {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            ResponseBody::Bytes(b) => b,
            ResponseBody::Json(v) => {
                // This is a fallback; callers should prefer json() for parsed data.
                // We return an empty slice since we can't return a reference to a temporary.
                let _ = v;
                &[]
            }
            ResponseBody::Empty => &[],
        }
    }
}

/// Timing information for the response.
#[derive(Debug, Clone)]
pub struct ResponseTiming {
    pub started_at: Instant,
    pub duration: Duration,
    pub dns_time: Option<Duration>,
    pub connect_time: Option<Duration>,
}

impl Default for ResponseTiming {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            duration: Duration::ZERO,
            dns_time: None,
            connect_time: None,
        }
    }
}

/// Rate limit information extracted from response headers.
#[derive(Debug, Clone, Default)]
pub struct RateLimitInfo {
    pub limit: Option<u32>,
    pub remaining: Option<u32>,
    pub reset_at: Option<u64>,
    pub retry_after: Option<u64>,
}

/// Pagination information extracted from response headers/body.
#[derive(Debug, Clone, Default)]
pub struct PaginationInfo {
    pub next: Option<String>,
    pub prev: Option<String>,
    pub total: Option<u64>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub has_more: bool,
}

/// Cache information extracted from response headers.
#[derive(Debug, Clone, Default)]
pub struct CacheInfo {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub cache_control: Option<String>,
    pub max_age: Option<u64>,
    pub is_cached: bool,
}

/// An enhanced HTTP response with pipeline-extracted metadata.
#[derive(Debug, Clone)]
pub struct MtwResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: ResponseBody,
    pub timing: ResponseTiming,
    pub metadata: HashMap<String, serde_json::Value>,
    pub rate_limit: Option<RateLimitInfo>,
    pub pagination: Option<PaginationInfo>,
    pub cache_info: Option<CacheInfo>,
}

impl MtwResponse {
    /// Deserialize the response body as JSON into a typed value.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, MtwError> {
        match &self.body {
            ResponseBody::Json(v) => {
                serde_json::from_value(v.clone()).map_err(|e| MtwError::Codec(e.to_string()))
            }
            ResponseBody::Bytes(b) => {
                serde_json::from_slice(b).map_err(|e| MtwError::Codec(e.to_string()))
            }
            ResponseBody::Empty => Err(MtwError::Codec("empty response body".into())),
        }
    }

    /// Get the response body as text.
    pub fn text(&self) -> Result<&str, MtwError> {
        match &self.body {
            ResponseBody::Bytes(b) => {
                std::str::from_utf8(b).map_err(|e| MtwError::Codec(e.to_string()))
            }
            ResponseBody::Json(_) => Err(MtwError::Codec(
                "body is parsed JSON, use json() instead".into(),
            )),
            ResponseBody::Empty => Ok(""),
        }
    }

    /// Get the response body as raw bytes.
    pub fn bytes(&self) -> &[u8] {
        self.body.as_bytes()
    }

    /// Whether the status code is 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Whether the status code is 4xx.
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status)
    }

    /// Whether the status code is 5xx.
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status)
    }

    /// Create a new response with default values (useful for testing).
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: ResponseBody::Empty,
            timing: ResponseTiming::default(),
            metadata: HashMap::new(),
            rate_limit: None,
            pagination: None,
            cache_info: None,
        }
    }

    /// Builder-style: set body bytes.
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = ResponseBody::Bytes(body.into());
        self
    }

    /// Builder-style: set JSON body.
    pub fn with_json(mut self, value: serde_json::Value) -> Self {
        self.body = ResponseBody::Json(value);
        self
    }

    /// Builder-style: set a header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Builder-style: set metadata.
    pub fn with_meta(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_checks() {
        assert!(MtwResponse::new(200).is_success());
        assert!(MtwResponse::new(201).is_success());
        assert!(!MtwResponse::new(301).is_success());

        assert!(MtwResponse::new(400).is_client_error());
        assert!(MtwResponse::new(404).is_client_error());
        assert!(!MtwResponse::new(500).is_client_error());

        assert!(MtwResponse::new(500).is_server_error());
        assert!(MtwResponse::new(503).is_server_error());
        assert!(!MtwResponse::new(404).is_server_error());
    }

    #[test]
    fn test_json_deserialization() {
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"name": "Alice", "age": 30}));
        let val: serde_json::Value = resp.json().unwrap();
        assert_eq!(val["name"], "Alice");
        assert_eq!(val["age"], 30);
    }

    #[test]
    fn test_text_body() {
        let resp = MtwResponse::new(200).with_body(Bytes::from("hello world"));
        assert_eq!(resp.text().unwrap(), "hello world");
    }

    #[test]
    fn test_empty_body_text() {
        let resp = MtwResponse::new(204);
        assert_eq!(resp.text().unwrap(), "");
    }

    #[test]
    fn test_json_from_bytes() {
        let body = serde_json::to_vec(&serde_json::json!({"x": 1})).unwrap();
        let resp = MtwResponse::new(200).with_body(Bytes::from(body));
        let val: serde_json::Value = resp.json().unwrap();
        assert_eq!(val["x"], 1);
    }
}
