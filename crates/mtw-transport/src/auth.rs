//! Handshake authentication for the WebSocket broker.
//!
//! Every connection is authorized *before* the WebSocket upgrade completes:
//! the transport reads the HTTP upgrade request, extracts the token
//! (`?token=` query parameter — a browser cannot set headers on a
//! WebSocket — or `Authorization: Bearer`), and asks an
//! [`MtwHandshakeAuth`] implementation for a decision. Anything other than
//! [`AuthDecision::Allow`] is answered with a plain HTTP error status and
//! the socket is closed; no `101 Switching Protocols` is ever sent.
//!
//! [`KernelBrokerAuth`] validates the token against the Kernl kernel's
//! `GET /api/auth/broker` endpoint (204 = super admin, 403 = user token,
//! 401 = anything else). It fails closed: if the kernel cannot be reached
//! or answers anything unexpected, the connection is rejected.

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Outcome of a handshake authorization check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    /// Credential accepted; the upgrade proceeds.
    Allow,
    /// No credential, or one the authority does not recognise (HTTP 401).
    Unauthorized,
    /// Valid credential that may not open the broker (HTTP 403).
    Forbidden,
    /// The authority could not decide (unreachable, timeout, unexpected
    /// answer). Rejected — fail closed (HTTP 503).
    Unavailable,
}

impl AuthDecision {
    /// HTTP status line used to reject the handshake.
    pub(crate) fn status_line(&self) -> &'static str {
        match self {
            AuthDecision::Allow => "200 OK",
            AuthDecision::Unauthorized => "401 Unauthorized",
            AuthDecision::Forbidden => "403 Forbidden",
            AuthDecision::Unavailable => "503 Service Unavailable",
        }
    }
}

/// Decides whether a WebSocket handshake may complete.
#[async_trait]
pub trait MtwHandshakeAuth: Send + Sync {
    /// Authorize a connection presenting `token` (already extracted from
    /// the query string or the `Authorization` header; `None` if absent).
    async fn authorize(&self, token: Option<&str>) -> AuthDecision;
}

/// Rejects every connection. Used when the server has no auth authority
/// configured and the operator did not explicitly disable auth.
pub struct DenyAll;

#[async_trait]
impl MtwHandshakeAuth for DenyAll {
    async fn authorize(&self, _token: Option<&str>) -> AuthDecision {
        AuthDecision::Unavailable
    }
}

/// Maximum number of cached valid tokens. Only the super-admin token is
/// ever valid, so this is a guard against unbounded growth, not a tuning
/// knob.
const MAX_CACHE_ENTRIES: usize = 64;

/// Validates broker tokens against Kernl's `GET /api/auth/broker`.
pub struct KernelBrokerAuth {
    endpoint: String,
    client: reqwest::Client,
    cache_ttl: Duration,
    /// token -> time it was validated. Only `Allow` results are cached.
    cache: DashMap<String, Instant>,
}

impl KernelBrokerAuth {
    /// `kernel_url` is the kernel's base URL (e.g. `http://kernel:3087`).
    /// `cache_ttl` is how long a successful validation is reused;
    /// `Duration::ZERO` disables the cache.
    pub fn new(kernel_url: &str, cache_ttl: Duration) -> Result<Self, String> {
        let base = kernel_url.trim_end_matches('/');
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            return Err(format!("kernel auth URL must be http(s): {kernel_url}"));
        }
        // Internal call to a sibling service: no proxy, short timeouts.
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("cannot build auth HTTP client: {e}"))?;
        Ok(Self {
            endpoint: format!("{base}/api/auth/broker"),
            client,
            cache_ttl,
            cache: DashMap::new(),
        })
    }

    /// Full URL of the validation endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn cached(&self, token: &str) -> bool {
        if self.cache_ttl.is_zero() {
            return false;
        }
        match self.cache.get(token) {
            Some(at) if at.elapsed() < self.cache_ttl => true,
            Some(_) => {
                drop(self.cache.remove(token));
                false
            }
            None => false,
        }
    }

    fn remember(&self, token: &str) {
        if self.cache_ttl.is_zero() {
            return;
        }
        if self.cache.len() >= MAX_CACHE_ENTRIES {
            self.cache.clear();
        }
        self.cache.insert(token.to_string(), Instant::now());
    }
}

#[async_trait]
impl MtwHandshakeAuth for KernelBrokerAuth {
    async fn authorize(&self, token: Option<&str>) -> AuthDecision {
        // A missing token is still sent to the kernel (without credentials):
        // the kernel is the single source of truth, and one running with
        // auth disabled (no KERNEL_AUTH_TOKEN) answers 204 to everyone,
        // while one with auth enabled answers 401.
        let token = token.unwrap_or("");
        if self.cached(token) {
            return AuthDecision::Allow;
        }
        let mut req = self.client.get(&self.endpoint);
        if !token.is_empty() {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await;
        let decision = match resp {
            Ok(r) => match r.status().as_u16() {
                200 | 204 => AuthDecision::Allow,
                401 => AuthDecision::Unauthorized,
                403 => AuthDecision::Forbidden,
                other => {
                    tracing::warn!(
                        status = other,
                        "broker auth: unexpected kernel status — rejecting"
                    );
                    AuthDecision::Unavailable
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "broker auth: kernel unreachable — rejecting");
                AuthDecision::Unavailable
            }
        };
        if decision == AuthDecision::Allow {
            self.remember(token);
        }
        decision
    }
}

/// Wrap any authorizer in an `Arc` trait object.
pub fn shared<A: MtwHandshakeAuth + 'static>(a: A) -> Arc<dyn MtwHandshakeAuth> {
    Arc::new(a)
}

/// Extract the broker token from a parsed upgrade request: `?token=` in
/// the request target wins, then `Authorization: Bearer`.
pub(crate) fn extract_token(target: &str, headers: &[(String, String)]) -> Option<String> {
    if let Some(q) = target.split_once('?').map(|(_, q)| q) {
        for pair in q.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if k == "token" && !v.is_empty() {
                return Some(percent_decode(v));
            }
        }
    }
    headers.iter().find_map(|(name, value)| {
        if !name.eq_ignore_ascii_case("authorization") {
            return None;
        }
        let (scheme, rest) = value.trim().split_once(' ')?;
        if scheme.eq_ignore_ascii_case("bearer") {
            let t = rest.trim();
            (!t.is_empty()).then(|| t.to_string())
        } else {
            None
        }
    })
}

/// Minimal `application/x-www-form-urlencoded` value decoding.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    #[test]
    fn token_from_query() {
        assert_eq!(extract_token("/ws?token=abc", &[]).as_deref(), Some("abc"));
        assert_eq!(
            extract_token("/ws?x=1&token=a%2Bb%3D", &[]).as_deref(),
            Some("a+b=")
        );
    }

    #[test]
    fn token_from_bearer() {
        let hs = vec![h("Authorization", "Bearer xyz")];
        assert_eq!(extract_token("/ws", &hs).as_deref(), Some("xyz"));
        let hs = vec![h("authorization", "Basic xyz")];
        assert_eq!(extract_token("/ws", &hs), None);
    }

    #[test]
    fn no_token() {
        assert_eq!(extract_token("/ws", &[]), None);
        assert_eq!(extract_token("/ws?token=", &[]), None);
    }

    #[test]
    fn rejects_non_http_url() {
        assert!(KernelBrokerAuth::new("kernel:3087", Duration::ZERO).is_err());
        let a = KernelBrokerAuth::new("http://kernel:3087/", Duration::ZERO).unwrap();
        assert_eq!(a.endpoint(), "http://kernel:3087/api/auth/broker");
    }

    #[tokio::test]
    async fn deny_all_rejects() {
        assert_eq!(
            DenyAll.authorize(Some("x")).await,
            AuthDecision::Unavailable
        );
    }
}
