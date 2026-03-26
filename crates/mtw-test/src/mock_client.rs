use mtw_core::MtwError;
use mtw_protocol::MtwMessage;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::timeout;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Mock WebSocket client for integration testing
pub struct MockClient {
    sink: SplitSink<WsStream, WsMessage>,
    stream: SplitStream<WsStream>,
}

impl MockClient {
    /// Connect to a WebSocket server at the given address
    pub async fn connect(addr: SocketAddr) -> Result<Self, MtwError> {
        Self::connect_url(&format!("ws://{}/ws", addr)).await
    }

    /// Connect to a WebSocket server at the given URL
    pub async fn connect_url(url: &str) -> Result<Self, MtwError> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| MtwError::Transport(format!("failed to connect: {}", e)))?;

        let (sink, stream) = ws_stream.split();

        Ok(Self { sink, stream })
    }

    /// Send an MtwMessage
    pub async fn send(&mut self, msg: MtwMessage) -> Result<(), MtwError> {
        let json = serde_json::to_string(&msg)
            .map_err(|e| MtwError::Codec(format!("serialize error: {}", e)))?;
        self.sink
            .send(WsMessage::Text(json.into()))
            .await
            .map_err(|e| MtwError::Transport(format!("send error: {}", e)))?;
        Ok(())
    }

    /// Send raw text
    pub async fn send_text(&mut self, text: &str) -> Result<(), MtwError> {
        self.sink
            .send(WsMessage::Text(text.to_string().into()))
            .await
            .map_err(|e| MtwError::Transport(format!("send error: {}", e)))?;
        Ok(())
    }

    /// Receive a message with a default timeout of 5 seconds
    pub async fn receive(&mut self) -> Result<MtwMessage, MtwError> {
        self.receive_timeout(Duration::from_secs(5))
            .await?
            .ok_or_else(|| MtwError::Transport("receive timed out".to_string()))
    }

    /// Receive a message with a custom timeout
    pub async fn receive_timeout(
        &mut self,
        duration: Duration,
    ) -> Result<Option<MtwMessage>, MtwError> {
        match timeout(duration, self.stream.next()).await {
            Ok(Some(Ok(ws_msg))) => {
                match ws_msg {
                    WsMessage::Text(text) => {
                        let msg: MtwMessage = serde_json::from_str(&text)
                            .map_err(|e| MtwError::Codec(format!("deserialize error: {}", e)))?;
                        Ok(Some(msg))
                    }
                    WsMessage::Close(_) => Ok(None),
                    _ => Ok(None),
                }
            }
            Ok(Some(Err(e))) => {
                Err(MtwError::Transport(format!("receive error: {}", e)))
            }
            Ok(None) => Ok(None), // Stream ended
            Err(_) => Ok(None),   // Timeout
        }
    }

    /// Send a subscribe message for a channel
    pub async fn subscribe(&mut self, channel: &str) -> Result<(), MtwError> {
        let msg = MtwMessage::new(
            mtw_protocol::MsgType::Subscribe,
            mtw_protocol::Payload::None,
        )
        .with_channel(channel);
        self.send(msg).await
    }

    /// Disconnect the client
    pub async fn disconnect(mut self) {
        let _ = self.sink.send(WsMessage::Close(None)).await;
    }
}
