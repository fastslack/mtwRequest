//! Peer-to-peer sync transport built on [iroh](https://docs.rs/iroh).
//!
//! Each instance binds an [`Endpoint`] using a 32-byte secret seed (typically
//! coming from `mtw_identity::MtwIdentity`). The same key serves as both the
//! peer's federation identity (NodeId) and its message-signing identity, so a
//! single keypair is the source of truth across the stack.
//!
//! Wire protocol (one bidirectional stream per request):
//!
//! ```text
//! → JSON `IrohRequest` followed by stream-finish
//! ← JSON `IrohResponse` followed by stream-finish
//! ```
//!
//! See [`IrohSyncTransport`] for the client API and [`serve`] for the server
//! side that handles incoming pull/push requests.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iroh::{Endpoint, NodeAddr, NodeId};
use iroh::key::SecretKey;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::changelog::ChangeLog;
use crate::sync::MtwSyncTransport;
use crate::types::{ChangeLogEntry, FederationPeer};

/// ALPN advertised by the federation transport. Bumped when the wire format
/// changes incompatibly.
pub const ALPN: &[u8] = b"mtw/federation/1";

/// Maximum response size we will read from a peer (1 MiB by default).
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1 * 1024 * 1024;

/// Wire request from a client to a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IrohRequest {
    Pull { since_version: u64, limit: usize },
    Push { changes: Vec<ChangeLogEntry> },
}

/// Wire response from a peer to a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IrohResponse {
    Changes { entries: Vec<ChangeLogEntry> },
    Pushed { applied: u32 },
    Error { message: String },
}

/// QUIC-based P2P sync transport. Holds the local [`Endpoint`] and exposes
/// pull/push to remote peers identified by `NodeId`.
#[derive(Clone)]
pub struct IrohSyncTransport {
    endpoint: Endpoint,
    max_response_bytes: usize,
    connect_timeout: Duration,
}

impl IrohSyncTransport {
    /// Bind a new endpoint using the keypair held by `identity`. The same
    /// keypair is used to sign federation messages elsewhere in the stack —
    /// one identity for both layers.
    pub async fn new(identity: &mtw_identity::MtwIdentity) -> Result<Self, MtwError> {
        Self::from_seed(identity.to_seed_bytes()).await
    }

    /// Bind a new endpoint directly from a 32-byte secret seed. Prefer
    /// [`Self::new`] when you already have an [`mtw_identity::MtwIdentity`].
    pub async fn from_seed(secret_seed: [u8; 32]) -> Result<Self, MtwError> {
        let secret_key = SecretKey::from_bytes(&secret_seed);
        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| MtwError::Transport(format!("iroh bind: {}", e)))?;
        Ok(Self {
            endpoint,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            connect_timeout: Duration::from_secs(15),
        })
    }

    pub fn with_max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }

    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Hex-encoded `NodeId` of this transport — share with peers so they can
    /// dial in.
    pub fn node_id_hex(&self) -> String {
        self.endpoint.node_id().to_string()
    }

    /// Reference to the underlying endpoint (for direct iroh interop).
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Spawn the accept loop that handles incoming pull/push requests against
    /// `changelog`. The returned [`JoinHandle`] runs until the endpoint is
    /// closed or the task is aborted.
    pub fn spawn_server(&self, changelog: Arc<ChangeLog>) -> JoinHandle<()> {
        let endpoint = self.endpoint.clone();
        let max_response_bytes = self.max_response_bytes;
        tokio::spawn(async move {
            serve(endpoint, changelog, max_response_bytes).await;
        })
    }

    async fn dial(&self, peer: &FederationPeer) -> Result<iroh::endpoint::Connection, MtwError> {
        let node_id_hex = peer
            .node_id
            .as_ref()
            .ok_or_else(|| MtwError::Transport(format!("peer {} has no node_id", peer.id)))?;
        let node_id: NodeId = node_id_hex
            .parse()
            .map_err(|e| MtwError::Transport(format!("invalid node_id: {}", e)))?;
        let addr = NodeAddr::new(node_id);

        tokio::time::timeout(
            self.connect_timeout,
            self.endpoint.connect(addr, ALPN),
        )
        .await
        .map_err(|_| MtwError::Transport("iroh connect timeout".into()))?
        .map_err(|e| MtwError::Transport(format!("iroh connect: {}", e)))
    }

    async fn round_trip(
        &self,
        peer: &FederationPeer,
        request: &IrohRequest,
    ) -> Result<IrohResponse, MtwError> {
        let connection = self.dial(peer).await?;
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| MtwError::Transport(format!("open_bi: {}", e)))?;

        let request_bytes = serde_json::to_vec(request)
            .map_err(|e| MtwError::Transport(format!("encode request: {}", e)))?;

        write_framed(&mut send, &request_bytes).await?;
        send.finish()
            .map_err(|e| MtwError::Transport(format!("finish: {}", e)))?;

        let response_bytes = read_framed(&mut recv, self.max_response_bytes).await?;
        connection.close(0u32.into(), b"done");

        let response: IrohResponse = serde_json::from_slice(&response_bytes)
            .map_err(|e| MtwError::Transport(format!("decode response: {}", e)))?;
        Ok(response)
    }
}

#[async_trait]
impl MtwSyncTransport for IrohSyncTransport {
    async fn pull_changes(
        &self,
        peer: &FederationPeer,
        since_version: u64,
        limit: usize,
    ) -> Result<Vec<ChangeLogEntry>, MtwError> {
        let req = IrohRequest::Pull { since_version, limit };
        match self.round_trip(peer, &req).await? {
            IrohResponse::Changes { entries } => Ok(entries),
            IrohResponse::Error { message } => Err(MtwError::Transport(format!(
                "remote error on pull: {}",
                message
            ))),
            IrohResponse::Pushed { .. } => {
                Err(MtwError::Transport("unexpected pushed response on pull".into()))
            }
        }
    }

    async fn push_changes(
        &self,
        peer: &FederationPeer,
        changes: &[ChangeLogEntry],
    ) -> Result<bool, MtwError> {
        let req = IrohRequest::Push {
            changes: changes.to_vec(),
        };
        match self.round_trip(peer, &req).await? {
            IrohResponse::Pushed { .. } => Ok(true),
            IrohResponse::Error { message } => Err(MtwError::Transport(format!(
                "remote error on push: {}",
                message
            ))),
            IrohResponse::Changes { .. } => {
                Err(MtwError::Transport("unexpected changes response on push".into()))
            }
        }
    }
}

/// Run the server-side accept loop. Each incoming connection is handled in its
/// own task; each bidirectional stream within a connection is one request.
pub async fn serve(endpoint: Endpoint, changelog: Arc<ChangeLog>, max_request_bytes: usize) {
    info!(node_id = %endpoint.node_id(), "iroh federation server listening");
    while let Some(incoming) = endpoint.accept().await {
        let changelog = changelog.clone();
        tokio::spawn(async move {
            let connecting = match incoming.accept() {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "rejected incoming");
                    return;
                }
            };
            let connection = match connecting.await {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "incoming connect failed");
                    return;
                }
            };
            loop {
                match connection.accept_bi().await {
                    Ok((send, recv)) => {
                        // Handle inline so the connection cannot drop (and
                        // tear down the stream) before the response is fully
                        // written and acked.
                        handle_stream(send, recv, changelog.clone(), max_request_bytes).await;
                    }
                    Err(e) => {
                        debug!(error = %e, "stream accept ended");
                        return;
                    }
                }
            }
        });
    }
}

async fn handle_stream(
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    changelog: Arc<ChangeLog>,
    max_request_bytes: usize,
) {
    let request_bytes = match read_framed(&mut recv, max_request_bytes).await {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "read request failed");
            return;
        }
    };

    let response = match serde_json::from_slice::<IrohRequest>(&request_bytes) {
        Ok(IrohRequest::Pull { since_version, limit }) => {
            let entries = changelog.get_changes_since(since_version, limit);
            IrohResponse::Changes { entries }
        }
        Ok(IrohRequest::Push { changes }) => {
            let applied = changelog.apply_remote(changes);
            IrohResponse::Pushed { applied }
        }
        Err(e) => IrohResponse::Error {
            message: format!("invalid request: {}", e),
        },
    };

    let response_bytes = match serde_json::to_vec(&response) {
        Ok(b) => b,
        Err(e) => {
            error!(error = %e, "encode response failed");
            return;
        }
    };

    if let Err(e) = write_framed(&mut send, &response_bytes).await {
        warn!(error = %e, "write response failed");
        return;
    }
    if let Err(e) = send.finish() {
        warn!(error = %e, "finish failed");
    }
    let _ = send.stopped().await;
}

/// Write a length-prefixed (4-byte BE) frame to a SendStream.
async fn write_framed(
    send: &mut iroh::endpoint::SendStream,
    payload: &[u8],
) -> Result<(), MtwError> {
    let len = payload.len() as u32;
    send.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| MtwError::Transport(format!("write len: {}", e)))?;
    send.write_all(payload)
        .await
        .map_err(|e| MtwError::Transport(format!("write body: {}", e)))?;
    Ok(())
}

/// Read a length-prefixed (4-byte BE) frame from a RecvStream.
async fn read_framed(
    recv: &mut iroh::endpoint::RecvStream,
    max_bytes: usize,
) -> Result<Vec<u8>, MtwError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| MtwError::Transport(format!("read len: {}", e)))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > max_bytes {
        return Err(MtwError::Transport(format!(
            "frame {} exceeds max {}",
            len, max_bytes
        )));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| MtwError::Transport(format!("read body: {}", e)))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::PeerRegistry;
    use crate::sync::SyncEngine;
    use crate::types::{ChangeAction, PeerStatus};
    use mtw_identity::MtwIdentity;

    fn make_peer(id: &str, node_id: String) -> FederationPeer {
        FederationPeer {
            id: id.to_string(),
            name: format!("Peer {}", id),
            url: String::new(),
            api_key: String::new(),
            status: PeerStatus::Active,
            last_seen_at: None,
            last_sync_version: 0,
            sync_errors: 0,
            created_at: "0".into(),
            updated_at: "0".into(),
            node_id: Some(node_id),
        }
    }

    /// End-to-end: two endpoints in the same process, one pushes, one pulls.
    #[tokio::test]
    async fn pull_and_push_roundtrip() {
        // --- peer A (server) ---
        let id_a = MtwIdentity::generate();
        let transport_a = IrohSyncTransport::new(&id_a).await.unwrap();
        let changelog_a = Arc::new(ChangeLog::new("peer-a"));
        // Seed peer A's changelog with one entry to be pulled by peer B.
        changelog_a.record(
            "tasks",
            "task-1",
            ChangeAction::Insert,
            serde_json::json!({"title": "from A"}),
        );
        let _server_a = transport_a.spawn_server(changelog_a.clone());

        // Capture peer A's full NodeAddr (with bound socket addresses) so
        // that peer B can dial without external discovery.
        let addr_a = transport_a.endpoint().node_addr().await.unwrap();
        let node_id_a = transport_a.node_id_hex();

        // --- peer B (client) ---
        let id_b = MtwIdentity::generate();
        let transport_b = IrohSyncTransport::new(&id_b).await.unwrap();
        let changelog_b = Arc::new(ChangeLog::new("peer-b"));

        // Inject A's addressing info into B's endpoint (bypass DHT discovery
        // for the local test).
        transport_b.endpoint().add_node_addr(addr_a).unwrap();

        let peer_a = make_peer("a", node_id_a);

        // Pull from A
        let pulled = transport_b.pull_changes(&peer_a, 0, 100).await.unwrap();
        assert_eq!(pulled.len(), 1);
        assert_eq!(pulled[0].table_name, "tasks");

        // Push to A
        changelog_b.record(
            "tasks",
            "task-2",
            ChangeAction::Insert,
            serde_json::json!({"title": "from B"}),
        );
        let local = changelog_b.get_changes_since(0, 100);
        assert!(transport_b.push_changes(&peer_a, &local).await.unwrap());

        // Verify A received it
        let after = changelog_a.get_changes_since(0, 100);
        assert_eq!(after.len(), 2);

        // Drive a full SyncEngine round through iroh
        let peers = Arc::new(PeerRegistry::new());
        peers.add_peer(peer_a.clone()).unwrap();
        let engine = SyncEngine::new(transport_b.clone(), changelog_b.clone(), peers.clone());
        let result = engine.sync_with_peer(&peer_a).await.unwrap();
        // Already had task-1 from earlier pull; nothing new here, but the call must succeed.
        assert_eq!(result.errors, 0);
    }
}
