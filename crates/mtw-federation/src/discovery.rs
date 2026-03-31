use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

use crate::types::FederationInstance;

/// Configuration for peer discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub broadcast_port: u16,
    pub discovery_interval_secs: u64,
    pub instance_name: String,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            broadcast_port: 7742,
            discovery_interval_secs: 30,
            instance_name: "mtwRequest".to_string(),
        }
    }
}

/// Announcement packet sent during discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryAnnouncement {
    pub instance_id: String,
    pub name: String,
    pub url: String,
    pub version: String,
}

/// Peer discovery service for LAN discovery
pub struct PeerDiscovery {
    config: DiscoveryConfig,
    instance: FederationInstance,
    discovered: RwLock<HashMap<String, FederationInstance>>,
}

impl PeerDiscovery {
    pub fn new(config: DiscoveryConfig, instance: FederationInstance) -> Self {
        Self {
            config,
            instance,
            discovered: RwLock::new(HashMap::new()),
        }
    }

    /// Create announcement payload for broadcasting
    pub fn announcement(&self) -> DiscoveryAnnouncement {
        DiscoveryAnnouncement {
            instance_id: self.instance.instance_id.clone(),
            name: self.instance.name.clone(),
            url: self.instance.url.clone(),
            version: "0.2.0".to_string(),
        }
    }

    /// Process a received announcement
    pub fn on_announcement(&self, announcement: DiscoveryAnnouncement) {
        // Don't discover ourselves
        if announcement.instance_id == self.instance.instance_id {
            return;
        }

        let now = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        let instance = FederationInstance {
            instance_id: announcement.instance_id.clone(),
            name: announcement.name,
            url: announcement.url,
            created_at: now,
        };

        self.discovered
            .write()
            .unwrap()
            .insert(announcement.instance_id, instance);
    }

    /// Get all discovered peers
    pub fn discovered_peers(&self) -> Vec<FederationInstance> {
        self.discovered
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect()
    }

    /// Get discovery config
    pub fn config(&self) -> &DiscoveryConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_instance(id: &str) -> FederationInstance {
        FederationInstance {
            instance_id: id.to_string(),
            name: format!("Instance {}", id),
            url: format!("http://{}.local:7741", id),
            created_at: "0".to_string(),
        }
    }

    #[test]
    fn test_discovery_announcement() {
        let discovery = PeerDiscovery::new(DiscoveryConfig::default(), make_instance("local"));
        let ann = discovery.announcement();
        assert_eq!(ann.instance_id, "local");
    }

    #[test]
    fn test_on_announcement() {
        let discovery = PeerDiscovery::new(DiscoveryConfig::default(), make_instance("local"));

        discovery.on_announcement(DiscoveryAnnouncement {
            instance_id: "remote-1".into(),
            name: "Remote 1".into(),
            url: "http://remote1.local:7741".into(),
            version: "0.2.0".into(),
        });

        assert_eq!(discovery.discovered_peers().len(), 1);
    }

    #[test]
    fn test_ignore_self() {
        let discovery = PeerDiscovery::new(DiscoveryConfig::default(), make_instance("local"));

        discovery.on_announcement(DiscoveryAnnouncement {
            instance_id: "local".into(),
            name: "Self".into(),
            url: "http://localhost:7741".into(),
            version: "0.2.0".into(),
        });

        assert_eq!(discovery.discovered_peers().len(), 0);
    }
}
