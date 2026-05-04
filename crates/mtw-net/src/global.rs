//! Global, process-wide [`NetFactory`].
//!
//! Why a global? Tens of `reqwest::Client` construction sites are
//! scattered across mtw-ai, mtw-integrations, mtw-exchange, etc. We
//! don't want to thread a factory handle through every constructor —
//! we want **one** call at boot to install the policy and have every
//! call site honour it implicitly.
//!
//! ## Usage
//!
//! ```no_run
//! use mtw_net::{NetConfig, NetFactory, install, default_client};
//!
//! # fn boot() -> Result<(), mtw_core::MtwError> {
//! // Once at boot:
//! let cfg = NetConfig::default();
//! install(NetFactory::new(cfg)).ok();
//!
//! // Anywhere later:
//! let client = default_client();
//! # Ok(()) }
//! ```
//!
//! Tests should construct their own [`NetFactory`] directly and not
//! touch the global — there is no `uninstall` because `OnceLock` is
//! one-shot. Tests that need isolation use `factory.client_for(...)`.

use std::sync::OnceLock;

use mtw_core::MtwError;

use crate::factory::NetFactory;

static GLOBAL: OnceLock<NetFactory> = OnceLock::new();

/// Install the process-wide factory. Idempotent on the first call;
/// subsequent calls return `Err(factory)` so the caller can decide
/// whether that's a bug or a benign double-init.
pub fn install(factory: NetFactory) -> Result<(), NetFactory> {
    GLOBAL.set(factory)
}

/// True after [`install`] has succeeded once.
pub fn is_installed() -> bool {
    GLOBAL.get().is_some()
}

/// Borrow the installed factory. Returns `None` if `install` was never
/// called — most callers prefer [`default_client`] / [`client_for`]
/// which auto-fall-back to a stock `reqwest::Client`.
pub fn factory() -> Option<&'static NetFactory> {
    GLOBAL.get()
}

/// Build a `reqwest::Client` for the named profile. Falls back to
/// [`default_client`] when no factory is installed.
pub fn client_for(profile: &str) -> reqwest::Client {
    match GLOBAL.get() {
        Some(f) => match f.client_for(profile) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    profile = %profile,
                    error = %e,
                    "mtw-net: profile lookup failed, falling back to direct client"
                );
                stock_client()
            }
        },
        None => stock_client(),
    }
}

/// Build a `reqwest::Client` for the configured default profile. Falls
/// back to a vanilla `reqwest::Client::new()` when no factory is
/// installed — that's the previous behaviour, kept on purpose so a
/// missing `install` call is a *no-op* rather than a hard failure.
pub fn default_client() -> reqwest::Client {
    match GLOBAL.get() {
        Some(f) => match f.default_client() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "mtw-net: default profile lookup failed, falling back to direct client"
                );
                stock_client()
            }
        },
        None => stock_client(),
    }
}

/// Like [`client_for`], but propagates errors instead of swallowing
/// them. Use when a missing/broken profile MUST fail loudly (e.g. a
/// privacy-sensitive caller that won't fall back to clear traffic).
pub fn client_for_strict(profile: &str) -> Result<reqwest::Client, MtwError> {
    let f = GLOBAL.get().ok_or_else(|| {
        MtwError::Config(
            "mtw-net: no factory installed — call install() before constructing privacy-sensitive clients"
                .into(),
        )
    })?;
    f.client_for(profile)
}

/// Build a fresh `reqwest::ClientBuilder` pre-configured with the
/// installed default profile. Lets callers add per-site options
/// (timeout, default headers) without giving up the egress policy.
/// Falls back to a vanilla `reqwest::Client::builder()` when no
/// factory is installed.
pub fn default_client_builder() -> reqwest::ClientBuilder {
    match GLOBAL.get() {
        Some(f) => {
            let cfg = f.config();
            let profile = cfg.default_profile.clone();
            match crate::factory::build_client_builder(cfg, &profile) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "mtw-net: default builder failed, falling back to stock"
                    );
                    reqwest::Client::builder()
                }
            }
        }
        None => reqwest::Client::builder(),
    }
}

/// Stock reqwest client used as the fallback. Wrapped so we can swap
/// in `https_only(true)` later if we want a stricter floor for the
/// "no factory installed" path.
fn stock_client() -> reqwest::Client {
    reqwest::Client::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NetConfig;

    // NB: this module's `install` is global-state — tests for the
    // installed path need to be the *only* test that calls install in
    // the binary's run. Other tests use the fallback path.

    #[test]
    fn fallback_when_not_installed() {
        // Don't call install here. default_client should still return a
        // working client.
        let _ = default_client();
        let _ = client_for("any");
    }

    #[test]
    fn strict_lookup_errors_when_not_installed() {
        // Hard failure when no factory and the caller wants enforcement.
        let err = client_for_strict("clear").unwrap_err();
        assert!(format!("{}", err).contains("no factory installed"));
    }

    #[test]
    fn factory_constructs_independently() {
        // Smoke check that NetFactory::new works without globals.
        let f = NetFactory::new(NetConfig::default());
        let _ = f.default_client().unwrap();
    }
}
