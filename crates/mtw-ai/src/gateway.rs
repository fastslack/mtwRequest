//! One LLM call to one provider endpoint, for a caller that keeps its own
//! routing policy.
//!
//! The caller (Kernl's `llm()` driver) decides which provider and model to
//! try, in what order, how often to retry and how to account for it; this
//! module only performs the exchange: it builds the provider's wire request,
//! sends it through the broker's egress policy (`mtw-net`), and reports the
//! outcome as data, never as a bare string:
//!
//! - success: the text, the model that answered, and every token count the
//!   provider reported (cache reads and writes included);
//! - an HTTP error: the status, the body, and the rate-limit headers, so the
//!   caller can honour `Retry-After` and tell a 429 from a 401 or a 5xx;
//! - a timeout or a network failure, told apart.
//!
//! Endpoint, key and model travel with every request: credentials have one
//! owner (the caller) and nothing here caches them.
//!
//! The text is returned as the provider sent it. Reasoning blocks, quirks
//! of particular models and parameter names (`max_completion_tokens`…) are
//! the caller's: it passes the top-level body fields it wants in `params`.

use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Default per-call timeout when the caller gives none.
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// Response headers a caller needs to work out how long to back off.
pub const RATE_LIMIT_HEADERS: &[&str] = &[
    "retry-after",
    "anthropic-ratelimit-requests-reset",
    "anthropic-ratelimit-tokens-reset",
    "x-ratelimit-reset-requests",
    "x-ratelimit-reset-tokens",
];

/// Which wire format the endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Wire {
    /// `POST …/chat/completions` (OpenAI and every compatible provider).
    Openai,
    /// `POST …/v1/messages` (Anthropic).
    Anthropic,
}

/// An inline image sent with the user turn.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Image {
    pub media_type: String,
    /// Base64, without a `data:` prefix.
    pub data: String,
}

/// One call.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayRequest {
    pub wire: Wire,
    /// The full endpoint URL.
    pub url: String,
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub system: Option<String>,
    pub user: String,
    #[serde(default)]
    pub image: Option<Image>,
    /// Top-level body fields (`max_tokens`, `temperature`,
    /// `response_format`…). `model`, `messages` and `system` are set here
    /// and win over the same keys in `params`.
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Token counts, as far as the provider reported them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GatewayUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The provider answered with an error status.
    Http,
    /// No answer within the timeout.
    Timeout,
    /// Could not reach the provider (DNS, refused, reset, TLS…).
    Network,
    /// A success status whose body is not what the wire format promises.
    InvalidResponse,
}

/// The outcome of one call. On the wire ([`GatewayOutcome::to_json`]) it is
/// `{ok: true, text, model, usage}` or `{ok: false, kind, status?, message,
/// headers?}`.
#[derive(Debug, Clone, PartialEq)]
pub enum GatewayOutcome {
    Success {
        text: String,
        model: String,
        usage: GatewayUsage,
    },
    Failure {
        kind: FailureKind,
        status: Option<u16>,
        message: String,
        /// The rate-limit headers the provider sent (lower-case names).
        headers: Map<String, Value>,
    },
}

impl GatewayOutcome {
    pub fn to_json(&self) -> Value {
        match self {
            GatewayOutcome::Success { text, model, usage } => serde_json::json!({
                "ok": true, "text": text, "model": model, "usage": usage,
            }),
            GatewayOutcome::Failure {
                kind,
                status,
                message,
                headers,
            } => {
                let mut v = serde_json::json!({ "ok": false, "kind": kind, "message": message });
                if let Some(s) = status {
                    v["status"] = (*s).into();
                }
                if !headers.is_empty() {
                    v["headers"] = Value::Object(headers.clone());
                }
                v
            }
        }
    }

    fn failure(kind: FailureKind, message: impl Into<String>) -> Self {
        GatewayOutcome::Failure {
            kind,
            status: None,
            message: message.into(),
            headers: Map::new(),
        }
    }
}

/// A local endpoint (LM Studio, Ollama…) skips the egress profile: a
/// SOCKS5/Tor proxy installed process-wide must not break localhost.
pub fn is_local(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port)
    };
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "host.docker.internal"
        || host == "::1"
        || host.starts_with("127.")
}

fn client_for(url: &str) -> &'static reqwest::Client {
    static EGRESS: OnceLock<reqwest::Client> = OnceLock::new();
    static LOCAL: OnceLock<reqwest::Client> = OnceLock::new();
    if is_local(url) {
        LOCAL.get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("failed to build local HTTP client")
        })
    } else {
        EGRESS.get_or_init(mtw_net::default_client)
    }
}

/// The provider's request body.
pub fn build_body(req: &GatewayRequest) -> Value {
    let mut body = req.params.clone();
    body.insert("model".into(), Value::String(req.model.clone()));
    let system = req.system.as_deref().filter(|s| !s.is_empty());
    match req.wire {
        Wire::Openai => {
            let user = match &req.image {
                Some(img) => serde_json::json!([
                    { "type": "text", "text": req.user },
                    { "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", img.media_type, img.data) } },
                ]),
                None => Value::String(req.user.clone()),
            };
            let mut messages = Vec::new();
            if let Some(s) = system {
                messages.push(serde_json::json!({ "role": "system", "content": s }));
            }
            messages.push(serde_json::json!({ "role": "user", "content": user }));
            body.insert("messages".into(), Value::Array(messages));
            body.remove("system");
        }
        Wire::Anthropic => {
            let user = match &req.image {
                Some(img) => serde_json::json!([
                    { "type": "image", "source": { "type": "base64", "media_type": img.media_type, "data": img.data } },
                    { "type": "text", "text": req.user },
                ]),
                None => Value::String(req.user.clone()),
            };
            body.insert(
                "messages".into(),
                serde_json::json!([{ "role": "user", "content": user }]),
            );
            match system {
                Some(s) => {
                    body.insert("system".into(), Value::String(s.to_string()));
                }
                None => {
                    body.remove("system");
                }
            }
        }
    }
    Value::Object(body)
}

fn u64_at(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

/// Read a success body.
pub fn parse_success(wire: Wire, requested_model: &str, data: &Value) -> Option<GatewayOutcome> {
    let model = data
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .unwrap_or(requested_model)
        .to_string();
    let usage = data.get("usage").cloned().unwrap_or(Value::Null);
    match wire {
        Wire::Openai => {
            let choices = data.get("choices")?.as_array()?;
            let text = choices
                .first()
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(GatewayOutcome::Success {
                text,
                model,
                usage: GatewayUsage {
                    input_tokens: u64_at(&usage, "prompt_tokens"),
                    output_tokens: u64_at(&usage, "completion_tokens"),
                    cache_read_tokens: usage
                        .get("prompt_tokens_details")
                        .and_then(|d| u64_at(d, "cached_tokens")),
                    cache_write_tokens: None,
                },
            })
        }
        Wire::Anthropic => {
            let blocks = data.get("content")?.as_array()?;
            let text = blocks
                .iter()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|b| b.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(GatewayOutcome::Success {
                text,
                model,
                usage: GatewayUsage {
                    input_tokens: u64_at(&usage, "input_tokens"),
                    output_tokens: u64_at(&usage, "output_tokens"),
                    cache_read_tokens: u64_at(&usage, "cache_read_input_tokens"),
                    cache_write_tokens: u64_at(&usage, "cache_creation_input_tokens"),
                },
            })
        }
    }
}

fn rate_limit_headers(headers: &reqwest::header::HeaderMap) -> Map<String, Value> {
    let mut out = Map::new();
    for name in RATE_LIMIT_HEADERS {
        if let Some(v) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            out.insert((*name).to_string(), Value::String(v.to_string()));
        }
    }
    out
}

/// Perform one call. Never fails: every outcome is reported as data.
pub async fn complete(req: &GatewayRequest) -> GatewayOutcome {
    let timeout_ms = req.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let body = build_body(req);
    let mut http = client_for(&req.url)
        .post(&req.url)
        .timeout(Duration::from_millis(timeout_ms))
        .header("content-type", "application/json")
        .json(&body);
    http = match req.wire {
        Wire::Openai => http.bearer_auth(&req.api_key),
        Wire::Anthropic => http
            .header("x-api-key", &req.api_key)
            .header("anthropic-version", "2023-06-01"),
    };

    let resp = match http.send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return GatewayOutcome::failure(
                FailureKind::Timeout,
                format!("no response in {timeout_ms}ms"),
            )
        }
        Err(e) => return GatewayOutcome::failure(FailureKind::Network, e.to_string()),
    };

    let status = resp.status();
    let headers = rate_limit_headers(resp.headers());
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) if e.is_timeout() => {
            return GatewayOutcome::failure(
                FailureKind::Timeout,
                format!("no response in {timeout_ms}ms"),
            )
        }
        Err(e) => return GatewayOutcome::failure(FailureKind::Network, e.to_string()),
    };

    if !status.is_success() {
        return GatewayOutcome::Failure {
            kind: FailureKind::Http,
            status: Some(status.as_u16()),
            message: text,
            headers,
        };
    }

    let parsed = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|data| parse_success(req.wire, &req.model, &data));
    parsed.unwrap_or_else(|| {
        let preview: String = text.chars().take(300).collect();
        GatewayOutcome::failure(
            FailureKind::InvalidResponse,
            format!("unexpected response body: {preview}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A one-shot HTTP server: answers the first request with `response`
    /// and hands back what it received.
    async fn serve_once(response: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = s.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
            }
            let _ = s.write_all(response.as_bytes()).await;
            let _ = s.shutdown().await;
            String::from_utf8_lossy(&buf).to_string()
        });
        (format!("http://{addr}/v1/x"), handle)
    }

    fn http(status: &str, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn request(wire: Wire, url: String) -> GatewayRequest {
        GatewayRequest {
            wire,
            url,
            api_key: "sk-test".into(),
            model: "m1".into(),
            system: Some("be brief".into()),
            user: "hi".into(),
            image: None,
            params: Map::new(),
            timeout_ms: Some(5_000),
        }
    }

    fn sent_body(raw: &str) -> Value {
        serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    #[tokio::test]
    async fn openai_success_reports_text_model_and_usage() {
        let (url, got) = serve_once(http(
            "200 OK",
            "",
            r#"{"model":"m1-2026","choices":[{"message":{"content":"<think>x</think>hello"}}],"usage":{"prompt_tokens":7,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":5}}}"#,
        ))
        .await;
        let mut req = request(Wire::Openai, url);
        req.params
            .insert("max_completion_tokens".into(), 512.into());
        req.params.insert("model".into(), "ignored".into());
        let out = complete(&req).await;
        assert_eq!(
            out,
            GatewayOutcome::Success {
                // Reasoning blocks are the caller's to strip.
                text: "<think>x</think>hello".into(),
                model: "m1-2026".into(),
                usage: GatewayUsage {
                    input_tokens: Some(7),
                    output_tokens: Some(3),
                    cache_read_tokens: Some(5),
                    cache_write_tokens: None
                },
            }
        );
        let raw = got.await.unwrap();
        assert!(raw.contains("authorization: Bearer sk-test"), "{raw}");
        let body = sent_body(&raw);
        assert_eq!(body["model"], "m1");
        assert_eq!(body["max_completion_tokens"], 512);
        assert_eq!(
            body["messages"][0],
            serde_json::json!({"role":"system","content":"be brief"})
        );
        assert_eq!(
            body["messages"][1],
            serde_json::json!({"role":"user","content":"hi"})
        );
    }

    #[tokio::test]
    async fn anthropic_success_with_cache_tokens_and_an_image() {
        let (url, got) = serve_once(http(
            "200 OK",
            "",
            r#"{"model":"claude-x","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":8,"cache_creation_input_tokens":1}}"#,
        ))
        .await;
        let mut req = request(Wire::Anthropic, url);
        req.image = Some(Image {
            media_type: "image/png".into(),
            data: "AAAA".into(),
        });
        req.params.insert("max_tokens".into(), 100.into());
        let out = complete(&req).await;
        assert_eq!(
            out,
            GatewayOutcome::Success {
                text: "ok".into(),
                model: "claude-x".into(),
                usage: GatewayUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(2),
                    cache_read_tokens: Some(8),
                    cache_write_tokens: Some(1)
                },
            }
        );
        let raw = got.await.unwrap();
        assert!(raw.contains("x-api-key: sk-test"), "{raw}");
        assert!(raw.contains("anthropic-version: 2023-06-01"), "{raw}");
        let body = sent_body(&raw);
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["messages"][0]["content"][0]["source"]["data"], "AAAA");
        assert_eq!(body["messages"][0]["content"][1]["text"], "hi");
    }

    #[tokio::test]
    async fn an_error_status_comes_back_with_its_body_and_rate_limit_headers() {
        let (url, _) = serve_once(http(
            "429 Too Many Requests",
            "Retry-After: 7\r\nx-ratelimit-reset-tokens: 1m30s\r\nX-Other: no\r\n",
            r#"{"error":"slow down"}"#,
        ))
        .await;
        match complete(&request(Wire::Openai, url)).await {
            GatewayOutcome::Failure {
                kind,
                status,
                message,
                headers,
            } => {
                assert_eq!(kind, FailureKind::Http);
                assert_eq!(status, Some(429));
                assert_eq!(message, r#"{"error":"slow down"}"#);
                assert_eq!(headers["retry-after"], "7");
                assert_eq!(headers["x-ratelimit-reset-tokens"], "1m30s");
                assert!(!headers.contains_key("x-other"));
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeouts_and_unreachable_endpoints_are_told_apart() {
        // Accepts and never answers.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_s, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let mut req = request(Wire::Openai, format!("http://{addr}/v1/x"));
        req.timeout_ms = Some(200);
        assert!(matches!(
            complete(&req).await,
            GatewayOutcome::Failure {
                kind: FailureKind::Timeout,
                ..
            }
        ));

        // Nothing listens there.
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = closed.local_addr().unwrap();
        drop(closed);
        let req = request(Wire::Openai, format!("http://{dead}/v1/x"));
        assert!(matches!(
            complete(&req).await,
            GatewayOutcome::Failure {
                kind: FailureKind::Network,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_success_status_with_a_foreign_body_is_reported() {
        let (url, _) = serve_once(http("200 OK", "", "<html>proxy</html>")).await;
        assert!(matches!(
            complete(&request(Wire::Anthropic, url)).await,
            GatewayOutcome::Failure {
                kind: FailureKind::InvalidResponse,
                ..
            }
        ));
    }

    #[test]
    fn outcomes_serialize_with_an_ok_flag() {
        let ok = GatewayOutcome::Success {
            text: "t".into(),
            model: "m".into(),
            usage: GatewayUsage {
                input_tokens: Some(1),
                ..Default::default()
            },
        }
        .to_json();
        assert_eq!(
            ok,
            serde_json::json!({"ok": true, "text": "t", "model": "m", "usage": {"input_tokens": 1}})
        );
        let failure = GatewayOutcome::failure(FailureKind::Timeout, "x").to_json();
        assert_eq!(
            failure,
            serde_json::json!({"ok": false, "kind": "timeout", "message": "x"})
        );
    }

    #[test]
    fn local_endpoints_are_recognised() {
        for url in [
            "http://localhost:1234/v1/chat/completions",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:8080/x",
            "http://host.docker.internal:1234/v1",
            "http://lm.localhost/v1",
        ] {
            assert!(is_local(url), "{url}");
        }
        for url in [
            "https://api.openai.com/v1/chat/completions",
            "https://localhost.evil.com/v1",
            "https://api.anthropic.com/v1/messages",
        ] {
            assert!(!is_local(url), "{url}");
        }
    }
}
