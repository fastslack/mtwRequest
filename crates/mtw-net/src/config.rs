use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::profile::OutboundProfile;

/// Minimum TLS version the factory will accept. `Tls13` is recommended;
/// `Tls12` exists for legacy endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsVersionFloor {
    #[serde(alias = "1.2", alias = "TLSv1.2")]
    Tls12,
    #[serde(alias = "1.3", alias = "TLSv1.3")]
    Tls13,
}

impl Default for TlsVersionFloor {
    fn default() -> Self {
        // 1.2 by default — many public APIs still negotiate it. Operators
        // can raise to 1.3 from config.
        Self::Tls12
    }
}

/// Top-level network configuration. Lives under `[net]` in `mtw.toml`.
///
/// ```toml
/// [net]
/// default_profile = "clear"
/// https_only = false
/// min_tls_version = "1.2"
/// user_agent = "mtwRequest/0.3"
///
/// [net.profiles.clear]
/// type = "direct"
///
/// [net.profiles.tor-trackers]
/// type = "proxy"
/// url = "socks5h://127.0.0.1:9050"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetConfig {
    /// Profile to use when a caller does not specify one.
    #[serde(default = "default_profile_name")]
    pub default_profile: String,

    /// Reject `http://` URLs at request time. Default `false` because
    /// many local integrations (Ollama, LM Studio, mock servers) speak
    /// plain HTTP. Set true in hardened deployments.
    #[serde(default)]
    pub https_only: bool,

    /// Minimum TLS version the client will negotiate.
    #[serde(default)]
    pub min_tls_version: TlsVersionFloor,

    /// User-Agent header applied to every request.
    #[serde(default = "default_user_agent")]
    pub user_agent: String,

    /// Total request timeout in seconds. `None` = use reqwest defaults.
    #[serde(default)]
    pub timeout_secs: Option<u64>,

    /// TCP connect timeout in seconds.
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,

    /// Named profiles. Always contains an entry for `clear` (auto-injected
    /// if missing) so callers can rely on it.
    #[serde(default)]
    pub profiles: HashMap<String, OutboundProfile>,
}

fn default_profile_name() -> String {
    "clear".into()
}

fn default_user_agent() -> String {
    format!("mtwRequest/{}", env!("CARGO_PKG_VERSION"))
}

fn default_connect_timeout() -> u64 {
    10
}

impl Default for NetConfig {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert("clear".into(), OutboundProfile::Direct);
        Self {
            default_profile: default_profile_name(),
            https_only: false,
            min_tls_version: TlsVersionFloor::default(),
            user_agent: default_user_agent(),
            timeout_secs: None,
            connect_timeout_secs: default_connect_timeout(),
            profiles,
        }
    }
}

impl NetConfig {
    /// Inject the `clear` profile if the operator forgot to declare it,
    /// so callers can always resolve `"clear"`.
    pub fn ensure_clear_profile(&mut self) {
        self.profiles
            .entry("clear".into())
            .or_insert(OutboundProfile::Direct);
    }

    /// Returns the profile under `name`, or the configured default if
    /// `name` is empty / "default".
    pub fn lookup(&self, name: &str) -> Option<&OutboundProfile> {
        let key = if name.is_empty() || name == "default" {
            self.default_profile.as_str()
        } else {
            name
        };
        self.profiles.get(key)
    }

    /// Returns the list of profile names available in this build (i.e.
    /// implemented + the cargo features they depend on are enabled).
    pub fn available_profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .profiles
            .iter()
            .filter(|(_, p)| p.is_available())
            .map(|(k, _)| k.clone())
            .collect();
        names.sort();
        names
    }

    /// Parse the `[net]` section out of a full mtw.toml content. Returns
    /// the default config when the section is absent so callers don't
    /// have to special-case "no `[net]`".
    ///
    /// We parse twice (once here, once in `MtwConfig::from_str`) on
    /// purpose: it keeps mtw-core decoupled from mtw-net. Toml parsing
    /// is fast (~tens of µs) and only runs at boot.
    pub fn from_mtw_toml(content: &str) -> Result<Self, mtw_core::MtwError> {
        #[derive(serde::Deserialize)]
        struct Wrap {
            #[serde(default)]
            net: Option<NetConfig>,
        }
        // We can't share env-expansion with MtwConfig's loader (lives
        // in mtw-core); callers that need `${ENV}` expansion should
        // pre-expand and pass the result here.
        let wrap: Wrap = toml::from_str(content)
            .map_err(|e| mtw_core::MtwError::Config(format!("[net] parse: {}", e)))?;
        let mut cfg = wrap.net.unwrap_or_default();
        cfg.ensure_clear_profile();
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let t = r#"
default_profile = "clear"
https_only = true
min_tls_version = "1.3"
user_agent = "mtwRequest/test"
timeout_secs = 30
connect_timeout_secs = 5

[profiles.clear]
type = "direct"

[profiles.tor-trackers]
type = "proxy"
url = "socks5h://127.0.0.1:9050"
"#;
        let cfg: NetConfig = toml::from_str(t).unwrap();
        assert_eq!(cfg.default_profile, "clear");
        assert!(cfg.https_only);
        assert_eq!(cfg.min_tls_version, TlsVersionFloor::Tls13);
        assert_eq!(cfg.timeout_secs, Some(30));
        assert_eq!(cfg.profiles.len(), 2);
    }

    #[test]
    fn ensure_clear_idempotent() {
        let mut cfg = NetConfig {
            profiles: HashMap::new(),
            ..NetConfig::default()
        };
        cfg.ensure_clear_profile();
        cfg.ensure_clear_profile();
        assert_eq!(cfg.profiles.len(), 1);
        assert!(matches!(
            cfg.profiles.get("clear"),
            Some(OutboundProfile::Direct)
        ));
    }

    #[test]
    fn lookup_default_alias() {
        let cfg = NetConfig::default();
        assert!(cfg.lookup("default").is_some());
        assert!(cfg.lookup("").is_some());
        assert!(cfg.lookup("nope").is_none());
    }

    #[test]
    fn tls_version_aliases() {
        let v: TlsVersionFloor = serde_json::from_str(r#""1.3""#).unwrap();
        assert_eq!(v, TlsVersionFloor::Tls13);
        let v: TlsVersionFloor = serde_json::from_str(r#""tls12""#).unwrap();
        assert_eq!(v, TlsVersionFloor::Tls12);
        let v: TlsVersionFloor = serde_json::from_str(r#""TLSv1.3""#).unwrap();
        assert_eq!(v, TlsVersionFloor::Tls13);
    }
}
