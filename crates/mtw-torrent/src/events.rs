//! Event publishing helpers — turn engine progress into bridge events
//! consumable by mtwKernel via Opción A (inline event frames).

use mtw_bridge::BridgeEventBus;
use serde_json::json;

use crate::types::{TorrentDetail, TorrentStatus};

/// Convenience publisher. Wraps a `BridgeEventBus` and emits torrent-
/// scoped topics with the canonical payload shape.
#[derive(Clone)]
pub struct TorrentEventPublisher {
    bus: BridgeEventBus,
}

impl TorrentEventPublisher {
    pub fn new(bus: BridgeEventBus) -> Self {
        Self { bus }
    }

    pub fn added(&self, detail: &TorrentDetail) {
        self.bus.emit(
            "torrent.added",
            serde_json::to_value(detail).unwrap_or(json!({})),
        );
    }

    pub fn metadata_ready(&self, detail: &TorrentDetail) {
        self.bus.emit(
            "torrent.metadata_ready",
            serde_json::to_value(detail).unwrap_or(json!({})),
        );
    }

    pub fn progress(&self, detail: &TorrentDetail) {
        let p = json!({
            "infohash": detail.summary.infohash,
            "downloaded": detail.downloaded_bytes,
            "uploaded": detail.uploaded_bytes,
            "peers": detail.peers,
            "progress": detail.progress,
            "status": detail.summary.status,
            "speed_bps_down": detail.speed_bps_down,
            "speed_bps_up": detail.speed_bps_up,
        });
        self.bus.emit("torrent.progress", p);
    }

    pub fn done(&self, infohash: &str) {
        self.bus
            .emit("torrent.done", json!({ "infohash": infohash }));
    }

    pub fn error(&self, infohash: &str, message: &str) {
        self.bus.emit(
            "torrent.error",
            json!({ "infohash": infohash, "message": message }),
        );
    }

    pub fn removed(&self, infohash: &str) {
        self.bus
            .emit("torrent.removed", json!({ "infohash": infohash }));
    }

    /// Emit `torrent.progress` only when status indicates active
    /// movement (downloading/seeding). For paused/done/error, prefer
    /// the dedicated topics — this keeps the kernel's topic stream clean.
    pub fn progress_if_active(&self, detail: &TorrentDetail) {
        match detail.summary.status {
            TorrentStatus::Downloading
            | TorrentStatus::Metadata
            | TorrentStatus::Seeding => self.progress(detail),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TorrentSummary, TorrentDetail};

    fn fake_detail(status: TorrentStatus) -> TorrentDetail {
        TorrentDetail {
            summary: TorrentSummary {
                infohash: "a".repeat(40),
                magnet: format!("magnet:?xt=urn:btih:{}", "a".repeat(40)),
                name: "x".into(),
                size_bytes: 100,
                status,
                encryption_profile: "clear".into(),
                added_at: "2026-05-03T00:00:00Z".into(),
                category: None,
                tags: vec![],
                description: None,
                ext: None,
            },
            files: vec![],
            downloaded_bytes: 50,
            uploaded_bytes: 0,
            peers: 1,
            progress: 0.5,
            speed_bps_down: 1000,
            speed_bps_up: 0,
            error: None,
        }
    }

    #[tokio::test]
    async fn publishes_progress_only_when_active() {
        let bus = BridgeEventBus::default();
        let mut rx = bus.subscribe();
        let pub_ = TorrentEventPublisher::new(bus);

        pub_.progress_if_active(&fake_detail(TorrentStatus::Paused));
        // No event sent — recv should not have one.
        let timed = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(timed.is_err(), "expected no event when paused");

        pub_.progress_if_active(&fake_detail(TorrentStatus::Downloading));
        let frame = rx.recv().await.unwrap();
        assert_eq!(frame.topic, "torrent.progress");
        assert_eq!(frame.data["progress"], 0.5);
    }

    #[tokio::test]
    async fn done_and_error_topics() {
        let bus = BridgeEventBus::default();
        let mut rx = bus.subscribe();
        let pub_ = TorrentEventPublisher::new(bus);
        pub_.done("ih");
        pub_.error("ih2", "boom");
        let f1 = rx.recv().await.unwrap();
        let f2 = rx.recv().await.unwrap();
        assert_eq!(f1.topic, "torrent.done");
        assert_eq!(f2.topic, "torrent.error");
        assert_eq!(f2.data["message"], "boom");
    }
}
