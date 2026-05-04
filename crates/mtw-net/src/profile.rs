use serde::{Deserialize, Serialize};

/// An outbound traffic profile. Each profile defines *how* a connection
/// leaves the host — directly, through a proxy, or (future) through a
/// userspace VPN / Tor / I2P stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboundProfile {
    /// Direct outbound. The OS routing table and kernel TLS stack handle
    /// everything. Used for `clear` traffic.
    Direct,

    /// Tunnel HTTP traffic through an HTTP, HTTPS, or SOCKS5 proxy. The
    /// `url` scheme picks the variant: `http://`, `https://`, `socks5://`,
    /// `socks5h://` (DNS resolved by the proxy).
    Proxy {
        url: String,
        #[serde(default)]
        no_proxy: Vec<String>,
    },

    /// Reserved. Userspace WireGuard via `boringtun`. Requires the
    /// `wireguard` cargo feature on `mtw-net`.
    #[serde(rename = "wireguard")]
    Wireguard {
        private_key: String,
        peer_public_key: String,
        endpoint: String,
        #[serde(default = "default_allowed_ips")]
        allowed_ips: Vec<String>,
        #[serde(default)]
        dns: Vec<String>,
        #[serde(default = "default_mtu")]
        mtu: u16,
    },

    /// Reserved. Native Tor via `arti`. Requires the `arti-tor` cargo
    /// feature on `mtw-net`.
    #[serde(rename = "tor")]
    Tor {
        /// Optional override for the bundled arti config. When unset, the
        /// embedded arti instance is used.
        #[serde(default)]
        config_path: Option<String>,
    },
}

fn default_allowed_ips() -> Vec<String> {
    vec!["0.0.0.0/0".into(), "::/0".into()]
}

fn default_mtu() -> u16 {
    1420
}

/// Recognised proxy schemes. Used for diagnostics; reqwest itself accepts
/// the `url` string as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyScheme {
    Http,
    Https,
    Socks5,
    /// SOCKS5 with remote DNS — recommended for Tor (`socks5h://`).
    Socks5h,
}

impl ProxyScheme {
    pub fn detect(url: &str) -> Option<Self> {
        let lower = url.to_ascii_lowercase();
        if lower.starts_with("http://") {
            Some(Self::Http)
        } else if lower.starts_with("https://") {
            Some(Self::Https)
        } else if lower.starts_with("socks5h://") {
            Some(Self::Socks5h)
        } else if lower.starts_with("socks5://") {
            Some(Self::Socks5)
        } else {
            None
        }
    }
}

impl OutboundProfile {
    /// Whether this profile is implemented in the current build. Profiles
    /// gated by feature flags or unimplemented variants return false; the
    /// factory will refuse to materialize them with a clear error.
    pub fn is_available(&self) -> bool {
        match self {
            Self::Direct => true,
            Self::Proxy { url, .. } => ProxyScheme::detect(url).is_some(),
            Self::Wireguard { .. } => cfg!(feature = "wireguard"),
            Self::Tor { .. } => cfg!(feature = "arti-tor"),
        }
    }

    /// A short, human-readable label used by `torrent.encryption.profiles`
    /// and similar discovery endpoints.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Proxy { .. } => "proxy",
            Self::Wireguard { .. } => "wireguard",
            Self::Tor { .. } => "tor",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_scheme_detection() {
        assert_eq!(
            ProxyScheme::detect("socks5h://127.0.0.1:9050"),
            Some(ProxyScheme::Socks5h)
        );
        assert_eq!(
            ProxyScheme::detect("SOCKS5://127.0.0.1:1080"),
            Some(ProxyScheme::Socks5)
        );
        assert_eq!(
            ProxyScheme::detect("http://corp.proxy:8080"),
            Some(ProxyScheme::Http)
        );
        assert_eq!(ProxyScheme::detect("garbage"), None);
    }

    #[test]
    fn parse_profile_toml() {
        let t = r#"type = "proxy"
url = "socks5h://127.0.0.1:9050"
no_proxy = ["127.0.0.1", "localhost"]
"#;
        let p: OutboundProfile = toml::from_str(t).unwrap();
        match p {
            OutboundProfile::Proxy { url, no_proxy } => {
                assert_eq!(url, "socks5h://127.0.0.1:9050");
                assert_eq!(no_proxy.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parse_direct_profile() {
        let t = r#"type = "direct""#;
        let p: OutboundProfile = toml::from_str(t).unwrap();
        assert!(matches!(p, OutboundProfile::Direct));
    }

    #[test]
    fn availability() {
        assert!(OutboundProfile::Direct.is_available());
        assert!(OutboundProfile::Proxy {
            url: "socks5h://127.0.0.1:9050".into(),
            no_proxy: vec![]
        }
        .is_available());
        // Wireguard / Tor depend on cargo features; in default builds they
        // are false. We only assert that the function doesn't panic.
        let _ = OutboundProfile::Tor { config_path: None }.is_available();
    }
}
