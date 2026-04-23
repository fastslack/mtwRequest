pub mod ws;

use async_trait::async_trait;
use mtw_core::MtwError;
use mtw_protocol::{ConnId, MtwMessage, SharedEnvelope, TransportEvent};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Transport trait — abstraction over WebSocket, HTTP, SSE, etc.
#[async_trait]
pub trait MtwTransport: Send + Sync {
    /// Transport name
    fn name(&self) -> &str;

    /// Start listening for connections
    async fn listen(&mut self, addr: SocketAddr) -> Result<(), MtwError>;

    /// Send a message to a specific connection
    async fn send(&self, conn_id: &ConnId, msg: MtwMessage) -> Result<(), MtwError>;

    /// Fan-out hot path for channel broadcasts: deliver a pre-built
    /// envelope whose wire form is encoded at most once per format,
    /// regardless of how many subscribers receive it.
    ///
    /// The default impl falls back to `send`, which re-encodes per-call —
    /// override in real transports.
    async fn send_envelope(
        &self,
        conn_id: &ConnId,
        envelope: Arc<SharedEnvelope>,
    ) -> Result<(), MtwError> {
        self.send(conn_id, envelope.message.clone()).await
    }

    /// Send raw binary data to a specific connection
    async fn send_binary(&self, conn_id: &ConnId, data: &[u8]) -> Result<(), MtwError>;

    /// Broadcast a message to all connections
    async fn broadcast(&self, msg: MtwMessage) -> Result<(), MtwError>;

    /// Close a specific connection
    async fn close(&self, conn_id: &ConnId) -> Result<(), MtwError>;

    /// Get the event receiver
    fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<TransportEvent>>;

    /// Get the number of active connections
    fn connection_count(&self) -> usize;

    /// Check if a connection exists
    fn has_connection(&self, conn_id: &ConnId) -> bool;

    /// Shutdown the transport
    async fn shutdown(&self) -> Result<(), MtwError>;
}
