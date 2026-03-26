use async_trait::async_trait;
use mtw_core::MtwError;
use mtw_protocol::{ConnId, MtwMessage, TransportEvent};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Mock transport for testing — records sent messages and allows injecting events
pub struct MockTransport {
    /// All messages that have been sent
    sent_messages: Arc<Mutex<Vec<(ConnId, MtwMessage)>>>,
    /// Broadcast messages
    broadcast_messages: Arc<Mutex<Vec<MtwMessage>>>,
    /// Event sender to inject transport events
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    /// Event receiver (taken by the consumer)
    event_rx: Option<mpsc::UnboundedReceiver<TransportEvent>>,
    /// Tracked connections
    connections: Arc<Mutex<Vec<ConnId>>>,
}

impl MockTransport {
    /// Create a new mock transport
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            sent_messages: Arc::new(Mutex::new(Vec::new())),
            broadcast_messages: Arc::new(Mutex::new(Vec::new())),
            event_tx,
            event_rx: Some(event_rx),
            connections: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get all messages sent to specific connections
    pub async fn sent_messages(&self) -> Vec<(ConnId, MtwMessage)> {
        self.sent_messages.lock().await.clone()
    }

    /// Get all broadcast messages
    pub async fn broadcast_messages(&self) -> Vec<MtwMessage> {
        self.broadcast_messages.lock().await.clone()
    }

    /// Inject a transport event (as if it came from a real connection)
    pub fn inject_event(&self, event: TransportEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Inject a message event from a specific connection
    pub fn inject_message(&self, conn_id: ConnId, msg: MtwMessage) {
        self.inject_event(TransportEvent::Message(conn_id, msg));
    }

    /// Add a mock connection
    pub async fn add_connection(&self, conn_id: ConnId) {
        self.connections.lock().await.push(conn_id);
    }

    /// Clear all recorded messages
    pub async fn clear(&self) {
        self.sent_messages.lock().await.clear();
        self.broadcast_messages.lock().await.clear();
    }

    /// Get the number of messages sent to specific connections
    pub async fn sent_count(&self) -> usize {
        self.sent_messages.lock().await.len()
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl mtw_transport::MtwTransport for MockTransport {
    fn name(&self) -> &str {
        "mock"
    }

    async fn listen(&mut self, _addr: SocketAddr) -> Result<(), MtwError> {
        Ok(())
    }

    async fn send(&self, conn_id: &ConnId, msg: MtwMessage) -> Result<(), MtwError> {
        self.sent_messages
            .lock()
            .await
            .push((conn_id.clone(), msg));
        Ok(())
    }

    async fn send_binary(&self, _conn_id: &ConnId, _data: &[u8]) -> Result<(), MtwError> {
        Ok(())
    }

    async fn broadcast(&self, msg: MtwMessage) -> Result<(), MtwError> {
        self.broadcast_messages.lock().await.push(msg);
        Ok(())
    }

    async fn close(&self, conn_id: &ConnId) -> Result<(), MtwError> {
        self.connections
            .lock()
            .await
            .retain(|c| c != conn_id);
        Ok(())
    }

    fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<TransportEvent>> {
        self.event_rx.take()
    }

    fn connection_count(&self) -> usize {
        // Use try_lock to avoid blocking; return 0 if the lock can't be acquired
        self.connections
            .try_lock()
            .map(|conns| conns.len())
            .unwrap_or(0)
    }

    fn has_connection(&self, conn_id: &ConnId) -> bool {
        self.connections
            .try_lock()
            .map(|conns| conns.contains(conn_id))
            .unwrap_or(false)
    }

    async fn shutdown(&self) -> Result<(), MtwError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_protocol::{MsgType, Payload};
    use mtw_transport::MtwTransport;

    #[tokio::test]
    async fn test_mock_send_records_messages() {
        let transport = MockTransport::new();
        let msg = MtwMessage::event("hello");
        transport.send(&"conn1".to_string(), msg).await.unwrap();

        let sent = transport.sent_messages().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "conn1");
        assert_eq!(sent[0].1.payload.as_text(), Some("hello"));
    }

    #[tokio::test]
    async fn test_mock_broadcast() {
        let transport = MockTransport::new();
        let msg = MtwMessage::event("broadcast");
        transport.broadcast(msg).await.unwrap();

        let broadcasts = transport.broadcast_messages().await;
        assert_eq!(broadcasts.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_inject_event() {
        let mut transport = MockTransport::new();
        let rx = transport.take_event_receiver().unwrap();

        let msg = MtwMessage::event("injected");
        transport.inject_message("conn1".to_string(), msg);

        // Wrap rx in a variable to use recv
        let mut rx = rx;
        let event = rx.recv().await.unwrap();
        match event {
            TransportEvent::Message(conn_id, msg) => {
                assert_eq!(conn_id, "conn1");
                assert_eq!(msg.payload.as_text(), Some("injected"));
            }
            _ => panic!("expected Message event"),
        }
    }

    #[tokio::test]
    async fn test_mock_connections() {
        let transport = MockTransport::new();
        transport.add_connection("conn1".to_string()).await;
        assert!(transport.has_connection(&"conn1".to_string()));
        assert_eq!(transport.connection_count(), 1);

        transport.close(&"conn1".to_string()).await.unwrap();
        assert!(!transport.has_connection(&"conn1".to_string()));
    }

    #[tokio::test]
    async fn test_mock_clear() {
        let transport = MockTransport::new();
        transport
            .send(&"conn1".to_string(), MtwMessage::event("msg1"))
            .await
            .unwrap();
        transport
            .broadcast(MtwMessage::event("msg2"))
            .await
            .unwrap();

        assert_eq!(transport.sent_count().await, 1);

        transport.clear().await;
        assert_eq!(transport.sent_count().await, 0);
        assert!(transport.broadcast_messages().await.is_empty());
    }

    #[tokio::test]
    async fn test_mock_listen() {
        let mut transport = MockTransport::new();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert!(transport.listen(addr).await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_name() {
        let transport = MockTransport::new();
        assert_eq!(transport.name(), "mock");
    }
}
