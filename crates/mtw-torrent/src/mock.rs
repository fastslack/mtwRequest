//! In-memory mock engine.
//!
//! Used for unit tests and as a default `clear`-only stub when the
//! `librqbit-engine` feature is disabled. The mock fakes successful
//! adds, persists state in a `DashMap`, and never touches the network or
//! disk — so it returns `0`-everywhere progress and zero peers.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;

use crate::engine::{ListFilter, ListResult, TorrentEngine};
use crate::magnet::{extract_infohash, extract_name};
use crate::types::{
    AddTorrentSpec, HttpServerInfo, StreamingTuning, TorrentDetail, TorrentHealth, TorrentSource,
    TorrentStatus, TorrentSummary,
};

/// In-memory engine. `Arc<MockEngine>` is `Clone`-cheap for sharing.
pub struct MockEngine {
    storage_path: String,
    inner: Arc<MockState>,
}

struct MockState {
    items: DashMap<String, TorrentDetail>,
}

impl MockEngine {
    pub fn new(storage_path: impl Into<String>) -> Self {
        Self {
            storage_path: storage_path.into(),
            inner: Arc::new(MockState {
                items: DashMap::new(),
            }),
        }
    }

    /// Number of torrents currently held. Test-only convenience.
    pub fn count(&self) -> usize {
        self.inner.items.len()
    }
}

fn iso_now() -> String {
    // UNIX epoch seconds → ISO 8601 (UTC). We avoid pulling chrono into
    // this crate just for the formatter.
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

/// Convert UNIX seconds to civil time (UTC) using Howard Hinnant's
/// algorithm. Avoids a chrono dep for one date.
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

/// Compute a deterministic 40-char hex pseudo-infohash from a string,
/// for the mock. NOT a real BitTorrent infohash.
fn mock_infohash(seed: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // Mix four hashers with different seeds to fill 160 bits of entropy.
    let mut buf = [0u8; 20];
    for i in 0..4 {
        let mut h = DefaultHasher::new();
        i.hash(&mut h);
        seed.hash(&mut h);
        let bytes = h.finish().to_le_bytes();
        buf[i * 5..(i + 1) * 5].copy_from_slice(&bytes[..5]);
    }
    hex::encode(buf)
}

#[async_trait]
impl TorrentEngine for MockEngine {
    async fn add(&self, spec: AddTorrentSpec) -> Result<TorrentDetail, MtwError> {
        let (infohash, magnet, name) = match &spec.source {
            TorrentSource::Magnet { magnet } => {
                let ih = extract_infohash(magnet).unwrap_or_else(|| mock_infohash(magnet));
                let name = extract_name(magnet).unwrap_or_else(|| "torrent".into());
                (ih, magnet.clone(), name)
            }
            TorrentSource::Url { torrent_url } => {
                let ih = mock_infohash(torrent_url);
                let magnet = format!("magnet:?xt=urn:btih:{}", ih);
                (ih, magnet, torrent_url.clone())
            }
            TorrentSource::Buffer { torrent_buffer } => {
                let ih = mock_infohash(&format!("buf:{}", torrent_buffer.len()));
                let magnet = format!("magnet:?xt=urn:btih:{}", ih);
                (ih, magnet, format!("buffer-{}b", torrent_buffer.len()))
            }
        };

        // Idempotent on infohash — re-apply metadata fields and return
        // the existing entry.
        if let Some(mut existing) = self.inner.items.get_mut(&infohash) {
            if let Some(d) = &spec.description {
                existing.summary.description = Some(d.clone());
            }
            if !spec.tags.is_empty() {
                existing.summary.tags = spec.tags.clone();
            }
            if let Some(c) = &spec.category {
                existing.summary.category = Some(c.clone());
            }
            return Ok(existing.clone());
        }

        let profile = spec
            .encryption_profile
            .clone()
            .unwrap_or_else(|| "clear".into());

        let detail = TorrentDetail {
            summary: TorrentSummary {
                infohash: infohash.clone(),
                magnet,
                name,
                size_bytes: 0,
                status: TorrentStatus::Metadata,
                encryption_profile: profile,
                added_at: iso_now(),
                category: spec.category.clone(),
                tags: spec.tags.clone(),
                description: spec.description.clone(),
                ext: spec.ext.clone(),
            },
            files: vec![],
            downloaded_bytes: 0,
            uploaded_bytes: 0,
            peers: 0,
            progress: 0.0,
            speed_bps_down: 0,
            speed_bps_up: 0,
            error: None,
        };

        self.inner.items.insert(infohash, detail.clone());
        Ok(detail)
    }

    async fn get(&self, infohash: &str) -> Result<TorrentDetail, MtwError> {
        self.inner
            .items
            .get(infohash)
            .map(|e| e.clone())
            .ok_or_else(|| MtwError::module("torrent", format!("infohash {} not found", infohash)))
    }

    async fn list(&self, filter: &ListFilter) -> Result<ListResult, MtwError> {
        let mut all: Vec<TorrentSummary> = self
            .inner
            .items
            .iter()
            .map(|e| e.value().summary.clone())
            .filter(|s| filter.status.map_or(true, |st| st == s.status))
            .collect();
        all.sort_by(|a, b| a.added_at.cmp(&b.added_at));
        let total = all.len();
        let off = filter.offset.unwrap_or(0);
        let lim = filter.limit.unwrap_or(100);
        let page = all.into_iter().skip(off).take(lim).collect();
        Ok(ListResult {
            torrents: page,
            total,
        })
    }

    async fn remove(&self, infohash: &str, _delete_files: bool) -> Result<(), MtwError> {
        self.inner.items.remove(infohash).ok_or_else(|| {
            MtwError::module("torrent", format!("infohash {} not found", infohash))
        })?;
        Ok(())
    }

    async fn pause(&self, infohash: &str) -> Result<TorrentDetail, MtwError> {
        let mut d = self
            .inner
            .items
            .get_mut(infohash)
            .ok_or_else(|| MtwError::module("torrent", format!("infohash {} not found", infohash)))?;
        d.summary.status = TorrentStatus::Paused;
        Ok(d.clone())
    }

    async fn resume(&self, infohash: &str) -> Result<TorrentDetail, MtwError> {
        let mut d = self
            .inner
            .items
            .get_mut(infohash)
            .ok_or_else(|| MtwError::module("torrent", format!("infohash {} not found", infohash)))?;
        d.summary.status = TorrentStatus::Downloading;
        Ok(d.clone())
    }

    async fn health(&self) -> Result<TorrentHealth, MtwError> {
        Ok(TorrentHealth {
            engine: "mock".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            active_torrents: self.inner.items.len() as u32,
            total_peers: 0,
            download_speed_bps: 0,
            upload_speed_bps: 0,
            storage_used_bytes: 0,
            storage_path: self.storage_path.clone(),
            http_server: HttpServerInfo {
                listen: String::new(),
                ready: false,
            },
            encryption_profiles_available: vec!["clear".into()],
            events_inline: true,
            streaming: StreamingTuning::default(),
        })
    }

    async fn stream_url(
        &self,
        infohash: &str,
        _file_idx: usize,
    ) -> Result<Option<String>, MtwError> {
        // Mock has no data plane.
        if !self.inner.items.contains_key(infohash) {
            return Err(MtwError::module(
                "torrent",
                format!("infohash {} not found", infohash),
            ));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileKind, TorrentFile};

    #[tokio::test]
    async fn add_and_get_round_trip() {
        let engine = MockEngine::new("/tmp/mtw-torrent-test");
        let spec = AddTorrentSpec {
            source: TorrentSource::Magnet {
                magnet: format!("magnet:?xt=urn:btih:{}", "a".repeat(40)),
            },
            encryption_profile: Some("clear".into()),
            category: Some("film".into()),
            tags: vec!["test".into()],
            description: None,
            ext: None,
            notify_on: None,
        };
        let added = engine.add(spec).await.unwrap();
        assert_eq!(added.summary.infohash, "a".repeat(40));
        assert_eq!(added.summary.encryption_profile, "clear");
        assert_eq!(added.summary.status, TorrentStatus::Metadata);

        let got = engine.get(&added.summary.infohash).await.unwrap();
        assert_eq!(got.summary.infohash, added.summary.infohash);
    }

    #[tokio::test]
    async fn add_is_idempotent_on_infohash() {
        let engine = MockEngine::new("/tmp/mtw-torrent-test");
        let magnet = format!("magnet:?xt=urn:btih:{}", "b".repeat(40));
        let spec1 = AddTorrentSpec {
            source: TorrentSource::Magnet { magnet: magnet.clone() },
            encryption_profile: None,
            category: None,
            tags: vec![],
            description: Some("first".into()),
            ext: None,
            notify_on: None,
        };
        let r1 = engine.add(spec1).await.unwrap();
        let spec2 = AddTorrentSpec {
            source: TorrentSource::Magnet { magnet },
            encryption_profile: None,
            category: None,
            tags: vec!["new-tag".into()],
            description: Some("updated".into()),
            ext: None,
            notify_on: None,
        };
        let r2 = engine.add(spec2).await.unwrap();
        assert_eq!(r1.summary.infohash, r2.summary.infohash);
        assert_eq!(r2.summary.description.as_deref(), Some("updated"));
        assert_eq!(r2.summary.tags, vec!["new-tag".to_string()]);
        assert_eq!(engine.count(), 1);
    }

    #[tokio::test]
    async fn list_filters_by_status() {
        let engine = MockEngine::new("/tmp/mtw-torrent-test");
        for c in ["a", "b", "c"] {
            engine
                .add(AddTorrentSpec {
                    source: TorrentSource::Magnet {
                        magnet: format!("magnet:?xt=urn:btih:{}", c.repeat(40)),
                    },
                    encryption_profile: None,
                    category: None,
                    tags: vec![],
                    description: None,
                    ext: None,
                    notify_on: None,
                })
                .await
                .unwrap();
            // Tiny gap so added_at differentiates ordering.
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        // Pause one to test status filter.
        engine.pause(&"a".repeat(40)).await.unwrap();
        let paused = engine
            .list(&ListFilter {
                status: Some(TorrentStatus::Paused),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(paused.torrents.len(), 1);
        let all = engine.list(&ListFilter::default()).await.unwrap();
        assert_eq!(all.total, 3);
    }

    #[tokio::test]
    async fn pause_and_resume_transition_state() {
        let engine = MockEngine::new("/tmp/mtw-torrent-test");
        let det = engine
            .add(AddTorrentSpec {
                source: TorrentSource::Magnet {
                    magnet: format!("magnet:?xt=urn:btih:{}", "d".repeat(40)),
                },
                encryption_profile: None,
                category: None,
                tags: vec![],
                description: None,
                ext: None,
                notify_on: None,
            })
            .await
            .unwrap();
        let p = engine.pause(&det.summary.infohash).await.unwrap();
        assert_eq!(p.summary.status, TorrentStatus::Paused);
        let r = engine.resume(&det.summary.infohash).await.unwrap();
        assert_eq!(r.summary.status, TorrentStatus::Downloading);
    }

    #[tokio::test]
    async fn remove_drops_entry() {
        let engine = MockEngine::new("/tmp/mtw-torrent-test");
        let det = engine
            .add(AddTorrentSpec {
                source: TorrentSource::Magnet {
                    magnet: format!("magnet:?xt=urn:btih:{}", "e".repeat(40)),
                },
                encryption_profile: None,
                category: None,
                tags: vec![],
                description: None,
                ext: None,
                notify_on: None,
            })
            .await
            .unwrap();
        engine.remove(&det.summary.infohash, false).await.unwrap();
        assert!(engine.get(&det.summary.infohash).await.is_err());
    }

    #[test]
    fn add_extracts_dn_as_name() {
        let engine = MockEngine::new("/tmp/mtw-torrent-test");
        let m = format!(
            "magnet:?xt=urn:btih:{}&dn=Sintel.mp4&tr=udp://t",
            "f".repeat(40)
        );
        let added = tokio_test_helper(engine.add(AddTorrentSpec {
            source: TorrentSource::Magnet { magnet: m },
            encryption_profile: None,
            category: None,
            tags: vec![],
            description: None,
            ext: None,
            notify_on: None,
        }))
        .unwrap();
        assert_eq!(added.summary.infohash, "f".repeat(40));
        assert_eq!(added.summary.name, "Sintel.mp4");
    }

    /// Run an async fn synchronously inside a sync test. Used only here
    /// to avoid converting the test to `#[tokio::test]`, which is a much
    /// bigger refactor for one assertion.
    fn tokio_test_helper<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(fut)
    }

    #[test]
    fn epoch_to_civil_known_value() {
        // 2024-01-01T00:00:00Z → 1704067200
        let (y, m, d, h, mi, s) = epoch_to_civil(1704067200);
        assert_eq!((y, m, d, h, mi, s), (2024, 1, 1, 0, 0, 0));
        // Just past midnight on a leap-year boundary.
        let (y, m, d, _, _, _) = epoch_to_civil(1709251200);
        assert_eq!((y, m, d), (2024, 3, 1));
    }

    #[test]
    fn _exhaustive_filekind_keeps_compiler_happy() {
        // Touch enum variants so unused-import lints don't trigger when
        // upstream refactors files.
        let _ = (FileKind::Other, TorrentFile {
            name: "x".into(),
            path: "x".into(),
            size: 0,
            kind: FileKind::Other,
        });
    }
}
