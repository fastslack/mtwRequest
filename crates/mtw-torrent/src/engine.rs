//! Engine abstraction. Concrete implementations live in:
//! - `mock.rs` (always available, for tests and the `clear`-only stub)
//! - `librqbit_engine.rs` (gated by feature `librqbit-engine`)

use async_trait::async_trait;
use mtw_core::MtwError;

use crate::types::{
    AddTorrentSpec, TorrentDetail, TorrentHealth, TorrentStatus, TorrentSummary,
};

/// Filter for `torrent.list`.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub status: Option<TorrentStatus>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Result of `torrent.list`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ListResult {
    pub torrents: Vec<TorrentSummary>,
    pub total: usize,
}

/// The control-plane interface every torrent backend implements. Methods
/// are async because real engines (librqbit) drive their own runtime
/// tasks; the mock can return immediately.
#[async_trait]
pub trait TorrentEngine: Send + Sync {
    /// Add a new torrent. Idempotent on `infohash`: when the same hash
    /// already exists, the existing torrent is returned with metadata
    /// updates (description, tags) re-applied.
    async fn add(&self, spec: AddTorrentSpec) -> Result<TorrentDetail, MtwError>;

    /// Lookup a single torrent by 40-char hex infohash.
    async fn get(&self, infohash: &str) -> Result<TorrentDetail, MtwError>;

    /// List torrents.
    async fn list(&self, filter: &ListFilter) -> Result<ListResult, MtwError>;

    /// Remove a torrent. When `delete_files` is true, the on-disk data
    /// is purged. Otherwise only the swarm/index entry is dropped.
    async fn remove(&self, infohash: &str, delete_files: bool) -> Result<(), MtwError>;

    /// Pause downloading and seeding for a torrent.
    async fn pause(&self, infohash: &str) -> Result<TorrentDetail, MtwError>;

    /// Resume a paused torrent.
    async fn resume(&self, infohash: &str) -> Result<TorrentDetail, MtwError>;

    /// Engine-level diagnostics. Used by the kernel for capability
    /// detection at boot.
    async fn health(&self) -> Result<TorrentHealth, MtwError>;

    /// Signed URL (or path, scheme-less) the kernel proxy should hit to
    /// stream a single file with HTTP Range support. The engine is
    /// responsible for keeping this URL stable across restarts.
    ///
    /// Returns `None` when the data plane is not running (e.g. the
    /// engine is the `clear`-only stub without a backing HTTP server).
    async fn stream_url(&self, infohash: &str, file_idx: usize) -> Result<Option<String>, MtwError>;
}
