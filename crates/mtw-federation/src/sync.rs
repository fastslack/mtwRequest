use async_trait::async_trait;
use mtw_core::MtwError;
use tracing::{error, info};

use crate::changelog::ChangeLog;
use crate::peer::PeerRegistry;
use crate::types::{ChangeLogEntry, FederationPeer, PeerStatus, SyncResult};

/// Transport for pulling/pushing changes to peers
#[async_trait]
pub trait MtwSyncTransport: Send + Sync {
    /// Pull changes from a peer since a given version
    async fn pull_changes(
        &self,
        peer: &FederationPeer,
        since_version: u64,
        limit: usize,
    ) -> Result<Vec<ChangeLogEntry>, MtwError>;

    /// Push changes to a peer
    async fn push_changes(
        &self,
        peer: &FederationPeer,
        changes: &[ChangeLogEntry],
    ) -> Result<bool, MtwError>;
}

/// HTTP-based sync transport using reqwest
pub struct HttpSyncTransport {
    client: reqwest::Client,
    timeout: std::time::Duration,
}

impl HttpSyncTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout: std::time::Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Default for HttpSyncTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MtwSyncTransport for HttpSyncTransport {
    async fn pull_changes(
        &self,
        peer: &FederationPeer,
        since_version: u64,
        limit: usize,
    ) -> Result<Vec<ChangeLogEntry>, MtwError> {
        let url = format!(
            "{}/api/federation/changes?since={}&limit={}",
            peer.url.trim_end_matches('/'),
            since_version,
            limit
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", peer.api_key))
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| MtwError::Transport(format!("pull failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(MtwError::Transport(format!(
                "pull failed with status: {}",
                resp.status()
            )));
        }

        let entries: Vec<ChangeLogEntry> = resp
            .json()
            .await
            .map_err(|e| MtwError::Transport(format!("parse failed: {}", e)))?;

        Ok(entries)
    }

    async fn push_changes(
        &self,
        peer: &FederationPeer,
        changes: &[ChangeLogEntry],
    ) -> Result<bool, MtwError> {
        let url = format!(
            "{}/api/federation/push",
            peer.url.trim_end_matches('/')
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", peer.api_key))
            .json(changes)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| MtwError::Transport(format!("push failed: {}", e)))?;

        Ok(resp.status().is_success())
    }
}

/// Sync engine that coordinates data synchronization with peers
pub struct SyncEngine<T: MtwSyncTransport> {
    transport: T,
    changelog: std::sync::Arc<ChangeLog>,
    peers: std::sync::Arc<PeerRegistry>,
}

impl<T: MtwSyncTransport> SyncEngine<T> {
    pub fn new(
        transport: T,
        changelog: std::sync::Arc<ChangeLog>,
        peers: std::sync::Arc<PeerRegistry>,
    ) -> Self {
        Self {
            transport,
            changelog,
            peers,
        }
    }

    /// Sync with a specific peer: pull then push
    pub async fn sync_with_peer(&self, peer: &FederationPeer) -> Result<SyncResult, MtwError> {
        let mut result = SyncResult::empty();

        // Pull changes from peer
        match self
            .transport
            .pull_changes(peer, peer.last_sync_version, 100)
            .await
        {
            Ok(entries) => {
                if !entries.is_empty() {
                    let max_version = entries.iter().map(|e| e.version).max().unwrap_or(0);
                    let applied = self.changelog.apply_remote(entries);
                    result.applied = applied;
                    self.peers.update_sync_version(&peer.id, max_version).ok();
                    info!(peer = %peer.name, applied, "pulled changes");
                }
            }
            Err(e) => {
                error!(peer = %peer.name, error = %e, "pull failed");
                self.peers.record_error(&peer.id);
                result.errors += 1;
            }
        }

        // Push local changes to peer
        let local_changes = self
            .changelog
            .get_changes_since(peer.last_sync_version, 100);
        if !local_changes.is_empty() {
            match self.transport.push_changes(peer, &local_changes).await {
                Ok(true) => {
                    info!(peer = %peer.name, count = local_changes.len(), "pushed changes");
                }
                Ok(false) => {
                    error!(peer = %peer.name, "push rejected");
                    result.errors += 1;
                }
                Err(e) => {
                    error!(peer = %peer.name, error = %e, "push failed");
                    self.peers.record_error(&peer.id);
                    result.errors += 1;
                }
            }
        }

        Ok(result)
    }

    /// Sync with all active peers
    pub async fn sync_all(&self) -> (u32, u32) {
        let peers = self.peers.list_peers();
        let mut synced = 0;
        let mut failed = 0;

        for peer in peers {
            if peer.status != PeerStatus::Active {
                continue;
            }
            match self.sync_with_peer(&peer).await {
                Ok(result) => {
                    if result.errors == 0 {
                        synced += 1;
                    } else {
                        failed += 1;
                    }
                }
                Err(_) => {
                    failed += 1;
                    self.peers
                        .update_status(&peer.id, PeerStatus::Unreachable)
                        .ok();
                }
            }
        }

        (synced, failed)
    }
}

/// Resolve conflict using Last-Write-Wins strategy
pub fn resolve_lww(
    local: &ChangeLogEntry,
    remote: &ChangeLogEntry,
) -> crate::types::ConflictResolution {
    if remote.updated_at >= local.updated_at {
        crate::types::ConflictResolution::RemoteWins
    } else {
        crate::types::ConflictResolution::LocalWins
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChangeAction;
    use std::sync::Arc;

    struct MockTransport {
        pull_result: Vec<ChangeLogEntry>,
        push_ok: bool,
    }

    #[async_trait]
    impl MtwSyncTransport for MockTransport {
        async fn pull_changes(
            &self,
            _peer: &FederationPeer,
            _since: u64,
            _limit: usize,
        ) -> Result<Vec<ChangeLogEntry>, MtwError> {
            Ok(self.pull_result.clone())
        }

        async fn push_changes(
            &self,
            _peer: &FederationPeer,
            _changes: &[ChangeLogEntry],
        ) -> Result<bool, MtwError> {
            Ok(self.push_ok)
        }
    }

    fn make_peer() -> FederationPeer {
        FederationPeer {
            id: "peer-1".into(),
            name: "Test Peer".into(),
            url: "http://localhost:8080".into(),
            api_key: "key".into(),
            status: PeerStatus::Active,
            last_seen_at: None,
            last_sync_version: 0,
            sync_errors: 0,
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    #[tokio::test]
    async fn test_sync_with_peer() {
        let changelog = Arc::new(ChangeLog::new("local"));
        let peers = Arc::new(PeerRegistry::new());
        let peer = make_peer();
        peers.add_peer(peer.clone()).unwrap();

        let transport = MockTransport {
            pull_result: vec![ChangeLogEntry {
                id: "r1".into(),
                version: 1,
                instance_id: "remote".into(),
                table_name: "tasks".into(),
                row_id: "t1".into(),
                action: ChangeAction::Insert,
                data_json: serde_json::json!({"title": "Remote task"}),
                updated_at: "100".into(),
                created_at: "100".into(),
            }],
            push_ok: true,
        };

        let engine = SyncEngine::new(transport, changelog, peers);
        let result = engine.sync_with_peer(&peer).await.unwrap();
        assert_eq!(result.applied, 1);
        assert_eq!(result.errors, 0);
    }

    #[test]
    fn test_resolve_lww() {
        let local = ChangeLogEntry {
            id: "l1".into(),
            version: 1,
            instance_id: "local".into(),
            table_name: "t".into(),
            row_id: "r".into(),
            action: ChangeAction::Update,
            data_json: serde_json::json!({}),
            updated_at: "100".into(),
            created_at: "100".into(),
        };
        let remote = ChangeLogEntry {
            updated_at: "200".into(),
            ..local.clone()
        };

        assert_eq!(
            resolve_lww(&local, &remote),
            crate::types::ConflictResolution::RemoteWins
        );
        assert_eq!(
            resolve_lww(&remote, &local),
            crate::types::ConflictResolution::LocalWins
        );
    }
}
