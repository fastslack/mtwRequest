use dashmap::DashMap;
use mtw_core::MtwError;

use crate::types::{FederationPeer, PeerStatus, SyncStatus};

/// Registry managing federation peers
pub struct PeerRegistry {
    peers: DashMap<String, FederationPeer>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self {
            peers: DashMap::new(),
        }
    }

    /// Add a peer
    pub fn add_peer(&self, peer: FederationPeer) -> Result<(), MtwError> {
        if self.peers.contains_key(&peer.id) {
            return Err(MtwError::Internal(format!(
                "peer already exists: {}",
                peer.id
            )));
        }
        self.peers.insert(peer.id.clone(), peer);
        Ok(())
    }

    /// Remove a peer by ID
    pub fn remove_peer(&self, id: &str) -> Option<FederationPeer> {
        self.peers.remove(id).map(|(_, p)| p)
    }

    /// Get a peer by ID
    pub fn get_peer(&self, id: &str) -> Option<FederationPeer> {
        self.peers.get(id).map(|p| p.clone())
    }

    /// List all peers
    pub fn list_peers(&self) -> Vec<FederationPeer> {
        self.peers.iter().map(|e| e.value().clone()).collect()
    }

    /// Update peer status
    pub fn update_status(&self, id: &str, status: PeerStatus) -> Result<(), MtwError> {
        let mut peer = self
            .peers
            .get_mut(id)
            .ok_or_else(|| MtwError::Internal(format!("peer not found: {}", id)))?;
        peer.status = status;
        Ok(())
    }

    /// Update peer's last sync version
    pub fn update_sync_version(&self, id: &str, version: u64) -> Result<(), MtwError> {
        let mut peer = self
            .peers
            .get_mut(id)
            .ok_or_else(|| MtwError::Internal(format!("peer not found: {}", id)))?;
        peer.last_sync_version = version;
        peer.last_seen_at = Some(now_string());
        Ok(())
    }

    /// Increment sync error count
    pub fn record_error(&self, id: &str) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.sync_errors += 1;
        }
    }

    /// Get sync status for all peers
    pub fn sync_statuses(&self, pending_fn: impl Fn(u64) -> u64) -> Vec<SyncStatus> {
        self.peers
            .iter()
            .map(|e| {
                let peer = e.value();
                SyncStatus {
                    peer_id: peer.id.clone(),
                    peer_name: peer.name.clone(),
                    peer_url: peer.url.clone(),
                    status: peer.status,
                    last_seen_at: peer.last_seen_at.clone(),
                    last_sync_version: peer.last_sync_version,
                    pending_changes: pending_fn(peer.last_sync_version),
                    sync_errors: peer.sync_errors,
                }
            })
            .collect()
    }
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn now_string() -> String {
    format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_peer(id: &str) -> FederationPeer {
        FederationPeer {
            id: id.to_string(),
            name: format!("Peer {}", id),
            url: format!("http://peer-{}.local", id),
            api_key: "key".to_string(),
            status: PeerStatus::Active,
            last_seen_at: None,
            last_sync_version: 0,
            sync_errors: 0,
            created_at: now_string(),
            updated_at: now_string(),
        }
    }

    #[test]
    fn test_add_and_list() {
        let reg = PeerRegistry::new();
        reg.add_peer(make_peer("1")).unwrap();
        reg.add_peer(make_peer("2")).unwrap();
        assert_eq!(reg.list_peers().len(), 2);
    }

    #[test]
    fn test_duplicate_peer() {
        let reg = PeerRegistry::new();
        reg.add_peer(make_peer("1")).unwrap();
        assert!(reg.add_peer(make_peer("1")).is_err());
    }

    #[test]
    fn test_remove() {
        let reg = PeerRegistry::new();
        reg.add_peer(make_peer("1")).unwrap();
        assert!(reg.remove_peer("1").is_some());
        assert_eq!(reg.list_peers().len(), 0);
    }

    #[test]
    fn test_update_status() {
        let reg = PeerRegistry::new();
        reg.add_peer(make_peer("1")).unwrap();
        reg.update_status("1", PeerStatus::Unreachable).unwrap();
        assert_eq!(reg.get_peer("1").unwrap().status, PeerStatus::Unreachable);
    }

    #[test]
    fn test_update_sync_version() {
        let reg = PeerRegistry::new();
        reg.add_peer(make_peer("1")).unwrap();
        reg.update_sync_version("1", 42).unwrap();
        let peer = reg.get_peer("1").unwrap();
        assert_eq!(peer.last_sync_version, 42);
        assert!(peer.last_seen_at.is_some());
    }
}
