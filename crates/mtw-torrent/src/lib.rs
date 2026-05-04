//! # mtw-torrent
//!
//! Torrent engine module for mtwRequest. Exposes the `torrent.*` tools
//! over the bridge for mtwKernel and emits live progress events through
//! the bridge's event bus.
//!
//! ## Layout
//!
//! - [`engine`] — the [`engine::TorrentEngine`] trait every backend
//!   implements.
//! - [`mock`] — in-memory implementation used for tests and as a no-op
//!   stub when the `librqbit-engine` feature is off.
//! - [`librqbit_engine`] — real implementation built on `librqbit`,
//!   gated behind the `librqbit-engine` cargo feature.
//! - [`tools`] — bridge tool registration.
//! - [`events`] — event publisher mapping engine state changes onto
//!   `torrent.progress`/`torrent.done`/etc. event topics.
//! - [`config`] — `TorrentConfig` and per-profile encryption settings.
//! - [`types`] — wire-protocol types shared with the kernel.
//!
//! ## Bootstrap
//!
//! Most callers want [`init`], which loads config, picks an engine
//! based on cargo features, and registers the tools on the bridge:
//!
//! ```no_run
//! use std::sync::Arc;
//! use mtw_bridge::BridgeServer;
//! use mtw_torrent::{init, TorrentConfig};
//!
//! # async fn boot(bridge: &BridgeServer) -> Result<(), mtw_core::MtwError> {
//! let cfg = TorrentConfig::default();
//! let _service = init(cfg, bridge).await?;
//! # Ok(()) }
//! ```

pub mod config;
pub mod engine;
pub mod events;
pub(crate) mod magnet;
pub mod mock;
pub mod tools;
pub mod types;

#[cfg(feature = "librqbit-engine")]
pub mod librqbit_engine;

use std::sync::Arc;

use mtw_bridge::BridgeServer;
use mtw_core::MtwError;

pub use config::{EncryptionProfile, PeerTrafficPolicy, StreamingConfig, TorrentConfig};
pub use engine::{ListFilter, ListResult, TorrentEngine};
pub use events::TorrentEventPublisher;
pub use types::{
    AddTorrentSpec, EncryptionProfileInfo, FileKind, HttpServerInfo, NotifyOptions,
    StreamingTuning, TorrentDetail, TorrentFile, TorrentHealth, TorrentSource, TorrentStatus,
    TorrentSummary,
};

/// Live handle for the torrent module. Holds the engine, the publisher,
/// and the (mutable) config. Drop the handle to release engine
/// resources.
pub struct TorrentService {
    engine: Arc<dyn TorrentEngine>,
    publisher: TorrentEventPublisher,
    config: Arc<std::sync::RwLock<TorrentConfig>>,
}

impl TorrentService {
    pub fn engine(&self) -> &Arc<dyn TorrentEngine> {
        &self.engine
    }
    pub fn publisher(&self) -> &TorrentEventPublisher {
        &self.publisher
    }
    pub fn config(&self) -> Arc<std::sync::RwLock<TorrentConfig>> {
        self.config.clone()
    }
}

/// Bootstrap the torrent module:
///
/// 1. Load any external profiles file.
/// 2. Ensure the `clear` profile exists.
/// 3. Pick an engine — `librqbit-engine` feature → real one; otherwise
///    [`mock::MockEngine`] (handy for unit tests, but also a safe
///    fallback so the bridge tools register).
/// 4. Register the `torrent.*` tools on the bridge.
///
/// Returns a [`TorrentService`] that callers can store on their
/// services struct alongside the bridge handle.
pub async fn init(
    mut config: TorrentConfig,
    bridge: &BridgeServer,
) -> Result<TorrentService, MtwError> {
    if !config.enabled {
        return Err(MtwError::module(
            "torrent",
            "torrent module disabled by config (set [torrent].enabled = true)",
        ));
    }

    config.load_profiles_file()?;
    config.ensure_clear_profile();

    let storage = config.resolved_storage_path();
    if let Err(e) = std::fs::create_dir_all(&storage) {
        tracing::warn!(
            error = %e,
            path = %storage.display(),
            "torrent: could not create storage dir; continuing — engine will fail later"
        );
    }

    let bus = bridge.event_bus();
    let engine: Arc<dyn TorrentEngine> = build_engine(&config, &storage, bus.clone()).await?;
    let publisher = TorrentEventPublisher::new(bus);
    let config = Arc::new(std::sync::RwLock::new(config));
    tools::register_tools(bridge, engine.clone(), config.clone());

    tracing::info!(
        storage = %storage.display(),
        engine = "configured",
        "torrent module initialised"
    );

    Ok(TorrentService {
        engine,
        publisher,
        config,
    })
}

#[cfg(feature = "librqbit-engine")]
async fn build_engine(
    config: &TorrentConfig,
    storage: &std::path::Path,
    event_bus: mtw_bridge::BridgeEventBus,
) -> Result<Arc<dyn TorrentEngine>, MtwError> {
    let engine = librqbit_engine::LibrqbitEngine::new(config, storage, Some(event_bus)).await?;
    Ok(Arc::new(engine))
}

#[cfg(not(feature = "librqbit-engine"))]
async fn build_engine(
    _config: &TorrentConfig,
    storage: &std::path::Path,
    _event_bus: mtw_bridge::BridgeEventBus,
) -> Result<Arc<dyn TorrentEngine>, MtwError> {
    tracing::warn!(
        "torrent: librqbit-engine feature is disabled — using in-memory mock engine. \
         Real torrents will NOT download. Build with `--features librqbit-engine`."
    );
    Ok(Arc::new(mock::MockEngine::new(
        storage.to_string_lossy().to_string(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn init_registers_all_tools_with_mock_fallback() {
        let socket = format!("/tmp/mtw-torrent-init-{}.sock", ulid::Ulid::new());
        let server = mtw_bridge::BridgeServer::new(&socket);

        let dir = tempfile::tempdir().unwrap();
        let cfg = TorrentConfig {
            storage_path: Some(dir.path().to_path_buf()),
            ..TorrentConfig::default()
        };
        let _svc = init(cfg, &server).await.unwrap();
        assert_eq!(server.tool_count(), 9);
    }

    #[tokio::test]
    async fn init_refuses_disabled_module() {
        let server = mtw_bridge::BridgeServer::new("/tmp/mtw-torrent-disabled.sock");
        let cfg = TorrentConfig {
            enabled: false,
            ..TorrentConfig::default()
        };
        let outcome = init(cfg, &server).await;
        match outcome {
            Err(e) => assert!(format!("{}", e).contains("disabled")),
            Ok(_) => panic!("init should refuse disabled module"),
        }
    }
}
