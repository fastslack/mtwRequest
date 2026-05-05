//! # mtw-net
//!
//! Centralized outbound networking for mtwRequest.
//!
//! Every crate that needs to reach the internet (AI providers, exchange,
//! integrations, registry, federation HTTP, torrent webseeds, …) constructs
//! its `reqwest::Client` through this crate so a single policy applies:
//!
//! - HTTPS-only (configurable),
//! - TLS minimum version floor,
//! - HTTP/SOCKS5 proxy support,
//! - per-call profile selection (`clear` / `<custom>` / future `vpn` / `tor` / `i2p`),
//! - shared user-agent.
//!
//! ## Example
//!
//! ```no_run
//! use mtw_net::{NetConfig, OutboundProfile, build_client};
//!
//! let mut cfg = NetConfig::default();
//! cfg.profiles.insert(
//!     "tor-trackers".into(),
//!     OutboundProfile::Proxy {
//!         url: "socks5h://127.0.0.1:9050".into(),
//!         no_proxy: vec![],
//!     },
//! );
//! let client = build_client(&cfg, "tor-trackers").unwrap();
//! ```
//!
//! ## Roadmap
//!
//! - `OutboundProfile::Wireguard` (userspace WG via `boringtun`, feature
//!   `wireguard`)
//! - `OutboundProfile::Tor` native (via `arti`, feature `arti-tor`)
//! - `OutboundProfile::I2p`
//! - `TcpDialer` / `UdpBinder` traits for non-HTTP traffic (BitTorrent peers,
//!   custom protocols) so the same profile applies end-to-end.

pub mod config;
pub mod profile;
pub mod factory;
pub mod global;

#[cfg(feature = "wireguard")]
pub mod wireguard;

#[cfg(feature = "wireguard")]
pub mod wireguard_stack;

pub use config::{NetConfig, TlsVersionFloor};
pub use factory::{build_client, build_client_builder, NetFactory};
pub use global::{
    client_for, client_for_strict, default_client, default_client_builder, factory, install,
    is_installed,
};
pub use profile::{OutboundProfile, ProxyScheme};
