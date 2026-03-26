use bytes::Bytes;
use std::collections::HashMap;
use std::time::Duration;

/// HTTP methods supported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl Method {
    pub fn as_reqwest(&self) -> reqwest::Method {
        match self {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
            Method::Delete => reqwest::Method::DELETE,
            Method::Head => reqwest::Method::HEAD,
            Method::Options => reqwest::Method::OPTIONS,
        }
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Method::Get => write!(f, "GET"),
            Method::Post => write!(f, "POST"),
            Method::Put => write!(f, "PUT"),
            Method::Patch => write!(f, "PATCH"),
            Method::Delete => write!(f, "DELETE"),
            Method::Head => write!(f, "HEAD"),
            Method::Options => write!(f, "OPTIONS"),
        }
    }
}

/// HTTP request body.
#[derive(Debug, Clone)]
pub enum Body {
    Empty,
    Text(String),
    Json(serde_json::Value),
    Bytes(Bytes),
}

impl From<String> for Body {
    fn from(s: String) -> Self {
        Body::Text(s)
    }
}

impl From<&str> for Body {
    fn from(s: &str) -> Self {
        Body::Text(s.to_string())
    }
}

impl From<serde_json::Value> for Body {
    fn from(v: serde_json::Value) -> Self {
        Body::Json(v)
    }
}

impl From<Bytes> for Body {
    fn from(b: Bytes) -> Self {
        Body::Bytes(b)
    }
}

impl From<Vec<u8>> for Body {
    fn from(v: Vec<u8>) -> Self {
        Body::Bytes(Bytes::from(v))
    }
}

/// An HTTP request with metadata.
#[derive(Debug, Clone)]
pub struct MtwRequest {
    pub method: Method,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub query: HashMap<String, String>,
    pub body: Option<Body>,
    pub timeout: Option<Duration>,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl MtwRequest {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: HashMap::new(),
            query: HashMap::new(),
            body: None,
            timeout: None,
            metadata: HashMap::new(),
        }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }

    pub fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }

    pub fn put(url: impl Into<String>) -> Self {
        Self::new(Method::Put, url)
    }

    pub fn patch(url: impl Into<String>) -> Self {
        Self::new(Method::Patch, url)
    }

    pub fn delete(url: impl Into<String>) -> Self {
        Self::new(Method::Delete, url)
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.insert(key.into(), value.into());
        self
    }

    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn json(mut self, value: serde_json::Value) -> Self {
        self.body = Some(Body::Json(value));
        self.headers
            .insert("content-type".into(), "application/json".into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn meta(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_builder() {
        let req = MtwRequest::get("https://api.example.com/users")
            .header("accept", "application/json")
            .query("page", "1")
            .timeout(Duration::from_secs(30));

        assert_eq!(req.method, Method::Get);
        assert_eq!(req.url, "https://api.example.com/users");
        assert_eq!(req.headers.get("accept").unwrap(), "application/json");
        assert_eq!(req.query.get("page").unwrap(), "1");
        assert_eq!(req.timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_body_conversions() {
        let _b: Body = "hello".into();
        let _b: Body = String::from("hello").into();
        let _b: Body = serde_json::json!({"key": "value"}).into();
        let _b: Body = vec![1u8, 2, 3].into();
    }

    #[test]
    fn test_json_body_sets_content_type() {
        let req = MtwRequest::post("https://example.com").json(serde_json::json!({"a": 1}));
        assert_eq!(
            req.headers.get("content-type").unwrap(),
            "application/json"
        );
        assert!(matches!(req.body, Some(Body::Json(_))));
    }

    #[test]
    fn test_method_display() {
        assert_eq!(Method::Get.to_string(), "GET");
        assert_eq!(Method::Post.to_string(), "POST");
        assert_eq!(Method::Delete.to_string(), "DELETE");
    }
}
