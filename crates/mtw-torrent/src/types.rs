//! Wire-protocol types shared between the engine, the bridge tools, and
//! the kernel-side adapter. Field names match the contract documented in
//! `docs/mtwrequest-torrent-engine.prompt.md` (kernel side).

use serde::{Deserialize, Serialize};

/// Torrent lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentStatus {
    /// Accepted but not yet processed.
    Queued,
    /// Fetching metadata from the swarm (BEP-9) or webseed.
    Metadata,
    /// Downloading content.
    Downloading,
    /// Completed and serving the swarm.
    Seeding,
    /// User-paused.
    Paused,
    /// Fully downloaded. Set right after the last piece writes; the
    /// engine then transitions to `Seeding` once the seeding policy
    /// applies.
    Done,
    /// Terminal failure. `error` carries the message.
    Error,
}

/// Coarse classification of a file's kind, for the dashboard deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Video,
    Audio,
    Image,
    Text,
    Archive,
    Other,
}

impl FileKind {
    /// Best-effort classification by extension. Conservative: anything
    /// unrecognized falls back to `Other`.
    pub fn from_filename(name: &str) -> Self {
        let ext = match name.rsplit_once('.') {
            Some((_, ext)) => ext.to_ascii_lowercase(),
            None => return Self::Other,
        };
        match ext.as_str() {
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "ts" => Self::Video,
            "mp3" | "flac" | "ogg" | "opus" | "m4a" | "wav" | "aac" => Self::Audio,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "avif" => Self::Image,
            "txt" | "md" | "srt" | "vtt" | "json" | "csv" | "log" => Self::Text,
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" => Self::Archive,
            _ => Self::Other,
        }
    }
}

/// One file inside a torrent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentFile {
    /// Basename, e.g. `"sintel.mp4"`.
    pub name: String,
    /// Path inside the torrent root, with `/` separators.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Coarse classification for the deck.
    pub kind: FileKind,
}

/// Summary view of a torrent (no peer / file detail).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentSummary {
    /// 40-char lowercase hex.
    pub infohash: String,
    /// Canonical magnet URI (may include extra `&ws=` webseeds).
    pub magnet: String,
    pub name: String,
    pub size_bytes: u64,
    pub status: TorrentStatus,
    /// Profile id (e.g., `"clear"` / `"vpn-mullvad"`).
    pub encryption_profile: String,
    /// ISO 8601 timestamp.
    pub added_at: String,
    /// Categorisation passed in `torrent.add` (echoed back).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Caller-defined extension fields (kernel-side post/forum association).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<serde_json::Value>,
}

/// Full per-torrent status, including peers and progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentDetail {
    #[serde(flatten)]
    pub summary: TorrentSummary,
    pub files: Vec<TorrentFile>,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    pub peers: u32,
    /// 0.0..=1.0
    pub progress: f64,
    pub speed_bps_down: u64,
    pub speed_bps_up: u64,
    /// When `status == Error`, the engine's diagnostic message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Source of a torrent in `torrent.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TorrentSource {
    Magnet { magnet: String },
    Url { torrent_url: String },
    Buffer {
        #[serde(with = "serde_bytes_buffer")]
        torrent_buffer: Vec<u8>,
    },
}

/// Subscriptions for engine-pushed events for this torrent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyOptions {
    #[serde(default = "true_default")]
    pub progress: bool,
    #[serde(default = "true_default")]
    pub done: bool,
}

fn true_default() -> bool {
    true
}

impl Default for NotifyOptions {
    fn default() -> Self {
        Self { progress: true, done: true }
    }
}

/// Arguments to `torrent.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTorrentSpec {
    pub source: TorrentSource,
    #[serde(default)]
    pub encryption_profile: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub ext: Option<serde_json::Value>,
    #[serde(default)]
    pub notify_on: Option<NotifyOptions>,
}

/// One profile entry returned by `torrent.encryption.profiles`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionProfileInfo {
    pub id: String,
    pub label: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Aggregate engine status returned by `torrent.health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentHealth {
    pub engine: String,
    pub version: String,
    pub active_torrents: u32,
    pub total_peers: u32,
    pub download_speed_bps: u64,
    pub upload_speed_bps: u64,
    pub storage_used_bytes: u64,
    pub storage_path: String,
    pub http_server: HttpServerInfo,
    pub encryption_profiles_available: Vec<String>,
    /// True when the engine pushes events through the same bridge socket
    /// (Opción A in the contract). The kernel uses this to decide whether
    /// to subscribe to a separate WS or rely on inline frames.
    pub events_inline: bool,
    pub streaming: StreamingTuning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpServerInfo {
    /// Where the data plane listens, e.g., `"127.0.0.1:9999"` or
    /// `"unix:/tmp/mtw-torrent-data.sock"`. Empty when the engine is
    /// running but the server has not bound yet.
    pub listen: String,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingTuning {
    pub prefetch_window_bytes: u64,
    pub read_buffer_bytes: u64,
}

impl Default for StreamingTuning {
    fn default() -> Self {
        Self {
            prefetch_window_bytes: 16 * 1024 * 1024,
            read_buffer_bytes: 1024 * 1024,
        }
    }
}

mod serde_bytes_buffer {
    //! msgpack natively encodes `Vec<u8>` as `bin8/16/32` when round-
    //! tripped through `serde_bytes`, but we keep JSON compatibility for
    //! tests by accepting either base64-string or array-of-int.
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        serde_bytes::Bytes::new(b).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        serde_bytes::ByteBuf::deserialize(d).map(|bb| bb.into_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_kind_classification() {
        assert_eq!(FileKind::from_filename("a.mp4"), FileKind::Video);
        assert_eq!(FileKind::from_filename("Sintel.MKV"), FileKind::Video);
        assert_eq!(FileKind::from_filename("track.flac"), FileKind::Audio);
        assert_eq!(FileKind::from_filename("README"), FileKind::Other);
        assert_eq!(FileKind::from_filename(""), FileKind::Other);
        assert_eq!(FileKind::from_filename(".hidden"), FileKind::Other);
    }

    #[test]
    fn torrent_source_parses_magnet() {
        let v: TorrentSource =
            serde_json::from_value(serde_json::json!({"magnet": "magnet:?xt=..."})).unwrap();
        assert!(matches!(v, TorrentSource::Magnet { .. }));
    }

    #[test]
    fn torrent_source_parses_url() {
        let v: TorrentSource = serde_json::from_value(
            serde_json::json!({"torrent_url": "https://archive.org/x.torrent"}),
        )
        .unwrap();
        assert!(matches!(v, TorrentSource::Url { .. }));
    }

    #[test]
    fn add_spec_round_trip() {
        let spec = AddTorrentSpec {
            source: TorrentSource::Magnet { magnet: "magnet:?xt=urn:btih:abc".into() },
            encryption_profile: Some("clear".into()),
            category: Some("film".into()),
            tags: vec!["sintel".into()],
            description: None,
            ext: Some(serde_json::json!({"post_id": "p1"})),
            notify_on: Some(NotifyOptions::default()),
        };
        let j = serde_json::to_value(&spec).unwrap();
        let back: AddTorrentSpec = serde_json::from_value(j).unwrap();
        assert_eq!(back.encryption_profile.as_deref(), Some("clear"));
        assert_eq!(back.tags, vec!["sintel".to_string()]);
    }

    #[test]
    fn detail_serializes_with_flattened_summary() {
        let detail = TorrentDetail {
            summary: TorrentSummary {
                infohash: "a".repeat(40),
                magnet: "magnet:?xt=urn:btih:".to_string() + &"a".repeat(40),
                name: "x".into(),
                size_bytes: 1234,
                status: TorrentStatus::Downloading,
                encryption_profile: "clear".into(),
                added_at: "2026-05-03T00:00:00Z".into(),
                category: None,
                tags: vec![],
                description: None,
                ext: None,
            },
            files: vec![],
            downloaded_bytes: 100,
            uploaded_bytes: 0,
            peers: 3,
            progress: 0.08,
            speed_bps_down: 10_000,
            speed_bps_up: 0,
            error: None,
        };
        let j = serde_json::to_value(&detail).unwrap();
        // Flatten: top-level keys come from both summary and detail.
        assert_eq!(j["status"], "downloading");
        assert_eq!(j["progress"], 0.08);
        assert_eq!(j["peers"], 3);
        assert_eq!(j["infohash"].as_str().unwrap().len(), 40);
    }
}
