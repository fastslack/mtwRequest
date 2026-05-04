use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use mtw_core::MtwError;

use crate::config::{NetConfig, TlsVersionFloor};
use crate::profile::OutboundProfile;

/// Build a `reqwest::ClientBuilder` configured for the given profile —
/// the caller can layer extra options (custom timeouts, default
/// headers, …) before calling `.build()`. Use this when you can't use
/// a pre-built client because you need per-call-site reqwest options.
///
/// Most callers should reach for [`build_client`] / [`NetFactory`]
/// instead.
pub fn build_client_builder(
    config: &NetConfig,
    profile_name: &str,
) -> Result<reqwest::ClientBuilder, MtwError> {
    let profile = config.lookup(profile_name).ok_or_else(|| {
        MtwError::Config(format!(
            "mtw-net: profile '{}' not found (available: {:?})",
            profile_name,
            config.available_profile_names()
        ))
    })?;

    if !profile.is_available() {
        return Err(MtwError::Config(format!(
            "mtw-net: profile '{}' (kind '{}') is not available in this build — \
             enable the relevant cargo feature on mtw-net or pick a different profile",
            profile_name,
            profile.kind_label()
        )));
    }

    let mut builder = reqwest::Client::builder()
        .user_agent(&config.user_agent)
        .https_only(config.https_only)
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs));

    if let Some(secs) = config.timeout_secs {
        builder = builder.timeout(Duration::from_secs(secs));
    }

    builder = match config.min_tls_version {
        TlsVersionFloor::Tls12 => builder.min_tls_version(reqwest::tls::Version::TLS_1_2),
        TlsVersionFloor::Tls13 => builder.min_tls_version(reqwest::tls::Version::TLS_1_3),
    };

    match profile {
        OutboundProfile::Direct => {
            // Nothing to layer on. reqwest will still honor `HTTP_PROXY` /
            // `HTTPS_PROXY` from the environment, which is the documented
            // way to opt the whole process into a system-managed VPN
            // gateway without touching code.
        }
        OutboundProfile::Proxy { url, no_proxy } => {
            let mut proxy = reqwest::Proxy::all(url.as_str())
                .map_err(|e| MtwError::Config(format!("mtw-net: invalid proxy url '{}': {}", url, e)))?;
            if !no_proxy.is_empty() {
                let bypass = no_proxy.join(",");
                proxy = proxy.no_proxy(reqwest::NoProxy::from_string(&bypass));
            }
            builder = builder.proxy(proxy);
        }
        OutboundProfile::Wireguard { .. } => {
            return Err(MtwError::Config(
                "mtw-net: wireguard profiles require the `wireguard` cargo feature \
                 (boringtun integration not yet implemented)"
                    .into(),
            ));
        }
        OutboundProfile::Tor { .. } => {
            return Err(MtwError::Config(
                "mtw-net: tor profiles require the `arti-tor` cargo feature \
                 (arti integration not yet implemented). For now use a `proxy` \
                 profile pointing at `socks5h://127.0.0.1:9050`."
                    .into(),
            ));
        }
    }

    Ok(builder)
}

/// Build a `reqwest::Client` configured for the given profile under the
/// given `NetConfig`. Each call materializes a fresh client; for repeated
/// use prefer [`NetFactory`], which caches one client per profile name.
pub fn build_client(config: &NetConfig, profile_name: &str) -> Result<reqwest::Client, MtwError> {
    build_client_builder(config, profile_name)?
        .build()
        .map_err(|e| MtwError::Internal(format!("mtw-net: build reqwest client: {}", e)))
}

/// A factory that caches one `reqwest::Client` per profile name. Cheap to
/// clone (`Arc` internally). Use this when many call sites need the same
/// profile — e.g. all torrent webseed downloads under `clear`.
#[derive(Clone)]
pub struct NetFactory {
    inner: Arc<NetFactoryInner>,
}

struct NetFactoryInner {
    config: NetConfig,
    cache: DashMap<String, reqwest::Client>,
}

impl NetFactory {
    pub fn new(mut config: NetConfig) -> Self {
        config.ensure_clear_profile();
        Self {
            inner: Arc::new(NetFactoryInner {
                config,
                cache: DashMap::new(),
            }),
        }
    }

    pub fn config(&self) -> &NetConfig {
        &self.inner.config
    }

    /// Resolve a client for the named profile. The first call for each
    /// name builds and caches the client; subsequent calls clone the
    /// cheap `reqwest::Client` handle.
    pub fn client_for(&self, profile_name: &str) -> Result<reqwest::Client, MtwError> {
        let key = if profile_name.is_empty() || profile_name == "default" {
            self.inner.config.default_profile.clone()
        } else {
            profile_name.to_string()
        };

        if let Some(c) = self.inner.cache.get(&key) {
            return Ok(c.clone());
        }
        let client = build_client(&self.inner.config, &key)?;
        self.inner.cache.insert(key, client.clone());
        Ok(client)
    }

    /// Returns a client for the default profile. Equivalent to
    /// `client_for("default")`.
    pub fn default_client(&self) -> Result<reqwest::Client, MtwError> {
        self.client_for("default")
    }

    pub fn available_profiles(&self) -> Vec<String> {
        self.inner.config.available_profile_names()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::OutboundProfile;

    #[test]
    fn build_direct_client() {
        let cfg = NetConfig::default();
        let client = build_client(&cfg, "clear").unwrap();
        // Smoke-check: client is constructed.
        let _ = client;
    }

    #[test]
    fn unknown_profile_errors() {
        let cfg = NetConfig::default();
        let err = build_client(&cfg, "nope").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not found"), "{}", msg);
    }

    #[test]
    fn bad_proxy_url_errors() {
        let mut cfg = NetConfig::default();
        cfg.profiles.insert(
            "broken".into(),
            OutboundProfile::Proxy {
                url: "garbage".into(),
                no_proxy: vec![],
            },
        );
        // Profile reports unavailable for unknown schemes.
        let err = build_client(&cfg, "broken").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not available"), "{}", msg);
    }

    #[test]
    fn factory_caches_clients() {
        let mut cfg = NetConfig::default();
        cfg.profiles.insert(
            "p1".into(),
            OutboundProfile::Proxy {
                url: "socks5h://127.0.0.1:9050".into(),
                no_proxy: vec![],
            },
        );
        let factory = NetFactory::new(cfg);
        let a = factory.client_for("p1").unwrap();
        let b = factory.client_for("p1").unwrap();
        // reqwest::Client is Clone-shared; can't directly compare, but the
        // cache should not have grown a second entry.
        assert_eq!(factory.inner.cache.len(), 1);
        let _ = (a, b);
    }

    #[test]
    fn vpn_and_tor_profiles_reject_without_features() {
        let mut cfg = NetConfig::default();
        cfg.profiles.insert(
            "wg".into(),
            OutboundProfile::Wireguard {
                private_key: "k".into(),
                peer_public_key: "p".into(),
                endpoint: "1.2.3.4:51820".into(),
                allowed_ips: vec!["0.0.0.0/0".into()],
                dns: vec![],
                mtu: 1420,
            },
        );
        cfg.profiles
            .insert("tor".into(), OutboundProfile::Tor { config_path: None });

        // In the default build (no `wireguard` / `arti-tor` features) both
        // refuse to materialize.
        if !cfg!(feature = "wireguard") {
            let err = build_client(&cfg, "wg").unwrap_err();
            assert!(format!("{}", err).contains("not available"));
        }
        if !cfg!(feature = "arti-tor") {
            let err = build_client(&cfg, "tor").unwrap_err();
            assert!(format!("{}", err).contains("not available"));
        }
    }
}
