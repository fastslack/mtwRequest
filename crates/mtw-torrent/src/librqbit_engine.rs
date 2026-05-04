//! Real BitTorrent engine, built on `librqbit` 8.x.
//!
//! Gated by the `librqbit-engine` cargo feature. When the feature is off
//! this module is not compiled and [`crate::init`] falls back to the
//! in-memory [`crate::mock::MockEngine`].
//!
//! ## Architecture
//!
//! - One `librqbit::Session` owns the swarm state, peer connections,
//!   DHT, and on-disk pieces.
//! - **Async-add semantics:** `librqbit::Session::add_torrent` for a
//!   magnet awaits `resolve_magnet`, which can take ~30s+ on poorly-
//!   seeded torrents. We can't have the bridge tool block that long,
//!   so [`add`] spawns the underlying call in a tokio task and returns
//!   immediately with a synthetic detail (`status: metadata`). The
//!   user-supplied metadata (category/tags/description/ext) lives in
//!   `meta_sidecar`, a side-map keyed by infohash that survives the
//!   librqbit handle's lifetime.
//! - **Event pump:** a single shared task polls every 2s, computes
//!   detail per torrent, and emits `torrent.progress` /
//!   `torrent.metadata_ready` / `torrent.done` / `torrent.error`
//!   through the [`mtw_bridge::BridgeEventBus`].
//! - **Data plane:** librqbit's own `HttpApi` bound to
//!   `config.http_listen` serves files with native `Range` support.
//!   Stream URLs returned by [`stream_url`] point at it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use librqbit::{
    api::{Api, TorrentIdOrHash},
    http_api::{HttpApi, HttpApiOptions},
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session, SessionOptions,
    TorrentStats, TorrentStatsState,
};
use mtw_bridge::BridgeEventBus;
use mtw_core::MtwError;

use crate::config::TorrentConfig;
use crate::engine::{ListFilter, ListResult, TorrentEngine};
use crate::events::TorrentEventPublisher;
use crate::magnet::{extract_infohash, extract_name};
use crate::types::{
    AddTorrentSpec, FileKind, HttpServerInfo, StreamingTuning, TorrentDetail, TorrentFile,
    TorrentHealth, TorrentSource, TorrentStatus, TorrentSummary,
};

type TorrentHandle = Arc<ManagedTorrent>;

/// Per-infohash sidecar of fields librqbit doesn't track.
#[derive(Clone)]
struct MetaSidecar {
    encryption_profile: String,
    category: Option<String>,
    tags: Vec<String>,
    description: Option<String>,
    ext: Option<serde_json::Value>,
    added_at: String,
    /// Best-effort `name` while the torrent is still resolving (no
    /// metadata yet). Falls back to the infohash when absent.
    pending_name: Option<String>,
    /// Original magnet URI, kept so list/get can echo it back even
    /// before metadata resolves.
    pending_magnet: Option<String>,
    /// `true` while `session.add_torrent` is still resolving the magnet
    /// in the spawned task. Cleared once it returns.
    pending_resolve: bool,
}

pub struct LibrqbitEngine {
    session: Arc<Session>,
    http_listen_actual: String,
    storage_path: String,
    streaming: StreamingTuning,
    meta_sidecar: Arc<DashMap<String, MetaSidecar>>,
    publisher: Option<TorrentEventPublisher>,
}

impl LibrqbitEngine {
    /// Build the engine, start the session, bind the data-plane HTTP
    /// server, and spawn the progress pump (if `event_bus` is given).
    pub async fn new(
        config: &TorrentConfig,
        storage: &Path,
        event_bus: Option<BridgeEventBus>,
    ) -> Result<Self, MtwError> {
        let opts = SessionOptions {
            persistence: Some(librqbit::SessionPersistenceConfig::Json {
                folder: Some(storage.join("_meta")),
            }),
            fastresume: true,
            ..Default::default()
        };
        let session = Session::new_with_opts(storage.to_path_buf(), opts)
            .await
            .map_err(|e| MtwError::module("torrent", format!("librqbit session: {:#}", e)))?;

        let listener = tokio::net::TcpListener::bind(&config.http_listen)
            .await
            .map_err(|e| {
                MtwError::module(
                    "torrent",
                    format!(
                        "torrent: bind data-plane http_listen '{}': {}",
                        config.http_listen, e
                    ),
                )
            })?;
        let actual = listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| config.http_listen.clone());

        let api = Api::new(session.clone(), None, None);
        let http = HttpApi::new(api, Some(HttpApiOptions::default()));
        let http_fut = http.make_http_api_and_run(listener, None);
        tokio::spawn(async move {
            if let Err(e) = http_fut.await {
                tracing::error!(error = %e, "torrent: data-plane HTTP server exited with error");
            }
        });

        let publisher = event_bus.map(TorrentEventPublisher::new);
        let meta_sidecar: Arc<DashMap<String, MetaSidecar>> = Arc::new(DashMap::new());

        let engine = Self {
            session: session.clone(),
            http_listen_actual: actual.clone(),
            storage_path: storage.to_string_lossy().to_string(),
            streaming: StreamingTuning {
                prefetch_window_bytes: config.streaming.prefetch_window_bytes,
                read_buffer_bytes: config.streaming.read_buffer_bytes,
            },
            meta_sidecar: meta_sidecar.clone(),
            publisher: publisher.clone(),
        };

        if let Some(pub_) = publisher {
            spawn_progress_pump(session, meta_sidecar, pub_);
        }

        tracing::info!(listen = %actual, "torrent: librqbit engine + data plane up");
        Ok(engine)
    }

    fn parse_id(infohash: &str) -> Result<TorrentIdOrHash, MtwError> {
        TorrentIdOrHash::parse(infohash)
            .map_err(|e| MtwError::module("torrent", format!("invalid infohash: {}", e)))
    }

    fn handle(&self, infohash: &str) -> Option<TorrentHandle> {
        let id = Self::parse_id(infohash).ok()?;
        self.session.get(id)
    }

    /// Real detail for a resolved librqbit handle, with sidecar overlay.
    fn detail_for(&self, handle: &TorrentHandle) -> TorrentDetail {
        let info_hash = handle.info_hash().as_string();
        let stats: TorrentStats = handle.stats();
        let name = handle.name().unwrap_or_else(|| info_hash.clone());

        let (size_bytes, files) = match handle.metadata.load().as_ref() {
            Some(meta) => {
                let info = &meta.info;
                let mut files = Vec::new();
                let mut total: u64 = 0;
                if let Ok(iter) = info.iter_file_details() {
                    for f in iter {
                        let path = f
                            .filename
                            .to_string()
                            .unwrap_or_else(|_| String::from("<unreadable>"))
                            .replace(std::path::MAIN_SEPARATOR, "/");
                        let basename = path
                            .rsplit_once('/')
                            .map(|(_, b)| b.to_string())
                            .unwrap_or_else(|| path.clone());
                        let kind = FileKind::from_filename(&basename);
                        total = total.saturating_add(f.len);
                        files.push(TorrentFile {
                            name: basename,
                            path,
                            size: f.len,
                            kind,
                        });
                    }
                }
                (total.max(stats.total_bytes), files)
            }
            None => (stats.total_bytes, vec![]),
        };

        let status = map_status(&stats);
        let progress = if stats.total_bytes > 0 {
            (stats.progress_bytes as f64) / (stats.total_bytes as f64)
        } else {
            0.0
        };
        let (down_bps, up_bps) = stats
            .live
            .as_ref()
            .map(|live| {
                (
                    mbps_to_bps(live.download_speed.mbps),
                    mbps_to_bps(live.upload_speed.mbps),
                )
            })
            .unwrap_or((0, 0));
        let peers = stats
            .live
            .as_ref()
            .map(|l| l.snapshot.peer_stats.live as u32)
            .unwrap_or(0);

        let mut summary = TorrentSummary {
            infohash: info_hash.clone(),
            magnet: format!("magnet:?xt=urn:btih:{}", info_hash),
            name,
            size_bytes,
            status,
            encryption_profile: "clear".into(),
            added_at: String::new(),
            category: None,
            tags: vec![],
            description: None,
            ext: None,
        };
        overlay_sidecar(&self.meta_sidecar, &mut summary);

        TorrentDetail {
            summary,
            files,
            downloaded_bytes: stats.progress_bytes,
            uploaded_bytes: stats.uploaded_bytes,
            peers,
            progress,
            speed_bps_down: down_bps,
            speed_bps_up: up_bps,
            error: stats.error.clone(),
        }
    }

    /// Synthetic detail for a torrent that's still resolving its magnet.
    /// Built entirely from the sidecar — librqbit doesn't have a handle
    /// yet (or has one whose `metadata.load()` is None).
    fn synthetic_pending_detail(&self, infohash: &str, fallback_magnet: &str) -> TorrentDetail {
        let sidecar = self.meta_sidecar.get(infohash);
        let (name, magnet, profile, category, tags, description, ext, added_at) = sidecar
            .as_ref()
            .map(|s| {
                (
                    s.pending_name.clone().unwrap_or_else(|| infohash.to_string()),
                    s.pending_magnet.clone().unwrap_or_else(|| fallback_magnet.to_string()),
                    s.encryption_profile.clone(),
                    s.category.clone(),
                    s.tags.clone(),
                    s.description.clone(),
                    s.ext.clone(),
                    s.added_at.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    infohash.to_string(),
                    fallback_magnet.to_string(),
                    "clear".into(),
                    None,
                    Vec::new(),
                    None,
                    None,
                    iso_now(),
                )
            });

        TorrentDetail {
            summary: TorrentSummary {
                infohash: infohash.to_string(),
                magnet,
                name,
                size_bytes: 0,
                status: TorrentStatus::Metadata,
                encryption_profile: profile,
                added_at,
                category,
                tags,
                description,
                ext,
            },
            files: vec![],
            downloaded_bytes: 0,
            uploaded_bytes: 0,
            peers: 0,
            progress: 0.0,
            speed_bps_down: 0,
            speed_bps_up: 0,
            error: None,
        }
    }
}

#[async_trait]
impl TorrentEngine for LibrqbitEngine {
    async fn add(&self, spec: AddTorrentSpec) -> Result<TorrentDetail, MtwError> {
        // Three sources, only `Magnet` and `Url` need async metadata
        // resolution. `Buffer` carries the .torrent inline so resolution
        // is synchronous-ish — but to keep one code path we still spawn.
        let (infohash_hint, magnet_hint, name_hint, add_arg) = match spec.source.clone() {
            TorrentSource::Magnet { magnet } => (
                extract_infohash(&magnet),
                Some(magnet.clone()),
                extract_name(&magnet),
                AddTorrent::from_url(magnet),
            ),
            TorrentSource::Url { torrent_url } => {
                (None, None, None, AddTorrent::from_url(torrent_url))
            }
            TorrentSource::Buffer { torrent_buffer } => (
                None,
                None,
                None,
                AddTorrent::from_bytes(bytes::Bytes::from(torrent_buffer)),
            ),
        };

        let added_at = iso_now();
        let profile = spec
            .encryption_profile
            .clone()
            .unwrap_or_else(|| "clear".into());

        // If we know the infohash up front (the magnet case), seed the
        // sidecar and return a synthetic detail immediately. The actual
        // librqbit add runs in the background.
        if let Some(ih) = infohash_hint.clone() {
            self.meta_sidecar.insert(
                ih.clone(),
                MetaSidecar {
                    encryption_profile: profile.clone(),
                    category: spec.category.clone(),
                    tags: spec.tags.clone(),
                    description: spec.description.clone(),
                    ext: spec.ext.clone(),
                    added_at: added_at.clone(),
                    pending_name: name_hint.clone(),
                    pending_magnet: magnet_hint.clone(),
                    pending_resolve: true,
                },
            );

            let session = self.session.clone();
            let sidecar = self.meta_sidecar.clone();
            let publisher = self.publisher.clone();
            let ih_for_task = ih.clone();
            tokio::spawn(async move {
                let opts = AddTorrentOptions {
                    overwrite: true,
                    ..Default::default()
                };
                match session.add_torrent(add_arg, Some(opts)).await {
                    Ok(_resp) => {
                        if let Some(mut entry) = sidecar.get_mut(&ih_for_task) {
                            entry.pending_resolve = false;
                        }
                        // The progress pump task will pick it up on its
                        // next tick and emit `torrent.metadata_ready` —
                        // doing it here would race with stats settling.
                        tracing::info!(infohash = %ih_for_task, "librqbit: magnet resolved");
                    }
                    Err(e) => {
                        let msg = format!("{:#}", e);
                        tracing::warn!(infohash = %ih_for_task, error = %msg, "librqbit: add failed");
                        sidecar.remove(&ih_for_task);
                        if let Some(p) = publisher {
                            p.error(&ih_for_task, &msg);
                        }
                    }
                }
            });

            // Synthetic detail from the just-seeded sidecar.
            return Ok(self.synthetic_pending_detail(
                &ih,
                magnet_hint.as_deref().unwrap_or(""),
            ));
        }

        // No infohash hint (URL or buffer) → we have to await the add to
        // learn the infohash. This path is acceptable because both URL
        // and buffer carry the metadata directly; librqbit doesn't need
        // peer-resolved metadata, so it returns quickly.
        let opts = AddTorrentOptions {
            overwrite: true,
            ..Default::default()
        };
        let resp = self
            .session
            .add_torrent(add_arg, Some(opts))
            .await
            .map_err(|e| MtwError::module("torrent", format!("librqbit add: {:#}", e)))?;
        let handle = match resp {
            AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => h,
            AddTorrentResponse::ListOnly(_) => {
                return Err(MtwError::module(
                    "torrent",
                    "librqbit returned ListOnly — did you set list_only?",
                ));
            }
        };
        let resolved_ih = handle.info_hash().as_string();
        self.meta_sidecar.insert(
            resolved_ih.clone(),
            MetaSidecar {
                encryption_profile: profile,
                category: spec.category,
                tags: spec.tags,
                description: spec.description,
                ext: spec.ext,
                added_at,
                pending_name: None,
                pending_magnet: None,
                pending_resolve: false,
            },
        );
        Ok(self.detail_for(&handle))
    }

    async fn get(&self, infohash: &str) -> Result<TorrentDetail, MtwError> {
        if let Some(h) = self.handle(infohash) {
            return Ok(self.detail_for(&h));
        }
        // Maybe it's still resolving — fall back to the sidecar.
        if self.meta_sidecar.contains_key(infohash) {
            return Ok(self.synthetic_pending_detail(infohash, ""));
        }
        Err(MtwError::module(
            "torrent",
            format!("infohash {} not found", infohash),
        ))
    }

    async fn list(&self, filter: &ListFilter) -> Result<ListResult, MtwError> {
        // Resolved torrents from the session.
        let mut all: Vec<TorrentSummary> = self.session.with_torrents(|iter| {
            iter.map(|(_, mgr)| self.detail_for(mgr).summary).collect()
        });
        // Pending magnets that the session hasn't resolved yet — visible
        // to the kernel under `status: metadata` so the UI can render
        // them right after `add` returned.
        let already: std::collections::HashSet<String> =
            all.iter().map(|s| s.infohash.clone()).collect();
        for entry in self.meta_sidecar.iter() {
            if entry.pending_resolve && !already.contains(entry.key()) {
                all.push(
                    self.synthetic_pending_detail(entry.key(), "")
                        .summary,
                );
            }
        }

        if let Some(want) = filter.status {
            all.retain(|s| s.status == want);
        }
        let total = all.len();
        let off = filter.offset.unwrap_or(0);
        let lim = filter.limit.unwrap_or(100);
        let page = all.into_iter().skip(off).take(lim).collect();
        Ok(ListResult {
            torrents: page,
            total,
        })
    }

    async fn remove(&self, infohash: &str, delete_files: bool) -> Result<(), MtwError> {
        self.meta_sidecar.remove(infohash);
        let id = Self::parse_id(infohash)?;
        // If the session never resolved (still pending), `delete` will
        // return NotFound — swallow that so the caller can rely on
        // remove being idempotent.
        match self.session.delete(id, delete_files).await {
            Ok(_) => {}
            Err(e) => {
                let msg = format!("{:#}", e);
                if !msg.to_ascii_lowercase().contains("not found") {
                    return Err(MtwError::module("torrent", format!("librqbit delete: {}", msg)));
                }
            }
        }
        if let Some(p) = &self.publisher {
            p.removed(infohash);
        }
        Ok(())
    }

    async fn pause(&self, infohash: &str) -> Result<TorrentDetail, MtwError> {
        let h = self.handle(infohash).ok_or_else(|| {
            MtwError::module("torrent", format!("infohash {} not yet resolved", infohash))
        })?;
        self.session
            .pause(&h)
            .await
            .map_err(|e| MtwError::module("torrent", format!("librqbit pause: {:#}", e)))?;
        Ok(self.detail_for(&h))
    }

    async fn resume(&self, infohash: &str) -> Result<TorrentDetail, MtwError> {
        let h = self.handle(infohash).ok_or_else(|| {
            MtwError::module("torrent", format!("infohash {} not yet resolved", infohash))
        })?;
        self.session
            .unpause(&h)
            .await
            .map_err(|e| MtwError::module("torrent", format!("librqbit unpause: {:#}", e)))?;
        Ok(self.detail_for(&h))
    }

    async fn health(&self) -> Result<TorrentHealth, MtwError> {
        let stats = self.session.stats_snapshot();
        let active = self.session.with_torrents(|iter| iter.count()) as u32;
        let storage_used = walk_dir_size(Path::new(&self.storage_path)).unwrap_or(0);
        Ok(TorrentHealth {
            engine: "librqbit".into(),
            version: librqbit::version().to_string(),
            active_torrents: active,
            total_peers: stats.peers.live as u32,
            download_speed_bps: mbps_to_bps(stats.download_speed.mbps),
            upload_speed_bps: mbps_to_bps(stats.upload_speed.mbps),
            storage_used_bytes: storage_used,
            storage_path: self.storage_path.clone(),
            http_server: HttpServerInfo {
                listen: self.http_listen_actual.clone(),
                ready: true,
            },
            encryption_profiles_available: vec!["clear".into()],
            events_inline: true,
            streaming: self.streaming.clone(),
        })
    }

    async fn stream_url(
        &self,
        infohash: &str,
        file_idx: usize,
    ) -> Result<Option<String>, MtwError> {
        // Stream URL is fine to return even before metadata has fully
        // resolved — librqbit's HTTP API will 503 if the torrent isn't
        // ready yet, and the kernel proxy can re-try.
        if !self.meta_sidecar.contains_key(infohash) && self.handle(infohash).is_none() {
            return Err(MtwError::module(
                "torrent",
                format!("infohash {} not found", infohash),
            ));
        }
        Ok(Some(format!(
            "http://{}/torrents/{}/stream/{}",
            self.http_listen_actual, infohash, file_idx
        )))
    }
}

/// Apply user-supplied metadata (category, tags, profile, etc.) onto a
/// detail we just built from a librqbit handle.
fn overlay_sidecar(
    sidecar: &Arc<DashMap<String, MetaSidecar>>,
    summary: &mut TorrentSummary,
) {
    if let Some(s) = sidecar.get(&summary.infohash) {
        summary.encryption_profile = s.encryption_profile.clone();
        if summary.added_at.is_empty() {
            summary.added_at = s.added_at.clone();
        }
        if summary.category.is_none() {
            summary.category = s.category.clone();
        }
        if summary.tags.is_empty() {
            summary.tags = s.tags.clone();
        }
        if summary.description.is_none() {
            summary.description = s.description.clone();
        }
        if summary.ext.is_none() {
            summary.ext = s.ext.clone();
        }
    }
}

/// Single shared task that walks every torrent every 2s and emits
/// progress / state-change events. Cheaper than per-torrent watchers
/// and survives torrents being added/removed.
fn spawn_progress_pump(
    session: Arc<Session>,
    sidecar: Arc<DashMap<String, MetaSidecar>>,
    publisher: TorrentEventPublisher,
) {
    tokio::spawn(async move {
        // Per-torrent state we track to fire one-shot transition events.
        let mut emitted_metadata: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut emitted_done: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        let mut interval = tokio::time::interval(Duration::from_secs(2));
        // Drop the immediate tick — first emission should wait one period.
        interval.tick().await;

        loop {
            interval.tick().await;

            let snapshots: Vec<TorrentDetail> = session.with_torrents(|iter| {
                iter.map(|(_, mgr)| build_detail_for_pump(mgr, &sidecar))
                    .collect()
            });

            for detail in snapshots {
                let ih = detail.summary.infohash.clone();

                // metadata_ready: first time we see resolved metadata
                // for an infohash that was pending.
                let has_files = !detail.files.is_empty();
                if has_files && emitted_metadata.insert(ih.clone()) {
                    if let Some(mut s) = sidecar.get_mut(&ih) {
                        s.pending_resolve = false;
                    }
                    publisher.metadata_ready(&detail);
                }

                // progress (only while active)
                publisher.progress_if_active(&detail);

                // done: state == Done, emit once.
                if detail.summary.status == TorrentStatus::Done
                    && emitted_done.insert(ih.clone())
                {
                    publisher.done(&ih);
                }
            }
        }
    });
}

/// Standalone version of `LibrqbitEngine::detail_for` reused by the
/// pump task, which doesn't have an `&self` reference.
fn build_detail_for_pump(
    handle: &TorrentHandle,
    sidecar: &Arc<DashMap<String, MetaSidecar>>,
) -> TorrentDetail {
    let info_hash = handle.info_hash().as_string();
    let stats: TorrentStats = handle.stats();
    let name = handle.name().unwrap_or_else(|| info_hash.clone());

    let (size_bytes, files) = match handle.metadata.load().as_ref() {
        Some(meta) => {
            let info = &meta.info;
            let mut files = Vec::new();
            let mut total: u64 = 0;
            if let Ok(iter) = info.iter_file_details() {
                for f in iter {
                    let path = f
                        .filename
                        .to_string()
                        .unwrap_or_else(|_| String::from("<unreadable>"))
                        .replace(std::path::MAIN_SEPARATOR, "/");
                    let basename = path
                        .rsplit_once('/')
                        .map(|(_, b)| b.to_string())
                        .unwrap_or_else(|| path.clone());
                    let kind = FileKind::from_filename(&basename);
                    total = total.saturating_add(f.len);
                    files.push(TorrentFile {
                        name: basename,
                        path,
                        size: f.len,
                        kind,
                    });
                }
            }
            (total.max(stats.total_bytes), files)
        }
        None => (stats.total_bytes, vec![]),
    };

    let status = map_status(&stats);
    let progress = if stats.total_bytes > 0 {
        (stats.progress_bytes as f64) / (stats.total_bytes as f64)
    } else {
        0.0
    };
    let (down_bps, up_bps) = stats
        .live
        .as_ref()
        .map(|live| {
            (
                mbps_to_bps(live.download_speed.mbps),
                mbps_to_bps(live.upload_speed.mbps),
            )
        })
        .unwrap_or((0, 0));
    let peers = stats
        .live
        .as_ref()
        .map(|l| l.snapshot.peer_stats.live as u32)
        .unwrap_or(0);

    let mut summary = TorrentSummary {
        infohash: info_hash.clone(),
        magnet: format!("magnet:?xt=urn:btih:{}", info_hash),
        name,
        size_bytes,
        status,
        encryption_profile: "clear".into(),
        added_at: String::new(),
        category: None,
        tags: vec![],
        description: None,
        ext: None,
    };
    overlay_sidecar(sidecar, &mut summary);

    TorrentDetail {
        summary,
        files,
        downloaded_bytes: stats.progress_bytes,
        uploaded_bytes: stats.uploaded_bytes,
        peers,
        progress,
        speed_bps_down: down_bps,
        speed_bps_up: up_bps,
        error: stats.error.clone(),
    }
}

fn map_status(stats: &TorrentStats) -> TorrentStatus {
    if stats.error.is_some() {
        return TorrentStatus::Error;
    }
    if stats.finished {
        return TorrentStatus::Done;
    }
    match stats.state {
        TorrentStatsState::Initializing => TorrentStatus::Metadata,
        TorrentStatsState::Live => TorrentStatus::Downloading,
        TorrentStatsState::Paused => TorrentStatus::Paused,
        TorrentStatsState::Error => TorrentStatus::Error,
    }
}

fn iso_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day, hour, min, sec) = epoch_to_civil(now);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, min, sec
    )
}

fn epoch_to_civil(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let rem = (secs % 86400) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let z = days + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m as u32, d as u32, hour, min, sec)
}

fn walk_dir_size(path: &Path) -> Result<u64, std::io::Error> {
    if !path.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    let mut stack: Vec<std::path::PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}

#[inline]
fn mbps_to_bps(mbps: f64) -> u64 {
    (mbps * 1024.0 * 1024.0 * 8.0) as u64
}
