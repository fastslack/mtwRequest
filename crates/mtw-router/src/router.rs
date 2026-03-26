use std::sync::Arc;

use mtw_core::MtwError;
use mtw_protocol::{ConnId, MsgType, MtwMessage, TransportEvent};

use crate::channel::ChannelManager;
use crate::middleware::{MiddlewareChain, MiddlewareContext};

/// Message handler callback type
pub type MessageHandler =
    Arc<dyn Fn(ConnId, MtwMessage) -> futures::future::BoxFuture<'static, Result<Option<MtwMessage>, MtwError>> + Send + Sync>;

/// The main router — connects transport events to channels and handlers
pub struct MtwRouter {
    channels: Arc<ChannelManager>,
    middleware: Arc<MiddlewareChain>,
    handlers: Vec<(MsgType, MessageHandler)>,
}

impl MtwRouter {
    pub fn new(channels: ChannelManager, middleware: MiddlewareChain) -> Self {
        Self {
            channels: Arc::new(channels),
            middleware: Arc::new(middleware),
            handlers: Vec::new(),
        }
    }

    /// Register a handler for a specific message type
    pub fn on(&mut self, msg_type: MsgType, handler: MessageHandler) {
        self.handlers.push((msg_type, handler));
    }

    /// Get a reference to the channel manager
    pub fn channels(&self) -> &Arc<ChannelManager> {
        &self.channels
    }

    /// Process an incoming transport event
    pub async fn handle_event(
        &self,
        event: TransportEvent,
        transport_send: &dyn Fn(ConnId, MtwMessage) -> futures::future::BoxFuture<'static, Result<(), MtwError>>,
    ) -> Result<(), MtwError> {
        match event {
            TransportEvent::Connected(conn_id, meta) => {
                tracing::info!(conn_id = %conn_id, addr = ?meta.remote_addr, "client connected");
                Ok(())
            }

            TransportEvent::Disconnected(conn_id, reason) => {
                tracing::info!(conn_id = %conn_id, reason = ?reason, "client disconnected");
                self.channels.remove_connection(&conn_id);
                Ok(())
            }

            TransportEvent::Message(conn_id, msg) => {
                let ctx = MiddlewareContext {
                    conn_id: conn_id.clone(),
                    channel: msg.channel.clone(),
                };

                // Run through middleware chain
                let processed = self.middleware.process_inbound(msg, &ctx).await?;
                let msg = match processed {
                    Some(m) => m,
                    None => return Ok(()), // message was halted
                };

                // Handle by message type
                match &msg.msg_type {
                    MsgType::Subscribe => {
                        if let Some(channel) = &msg.channel {
                            self.channels.subscribe(channel, &conn_id)?;
                            // Send ack
                            let ack = MtwMessage::response(
                                &msg.id,
                                mtw_protocol::Payload::Json(serde_json::json!({
                                    "subscribed": channel,
                                })),
                            );
                            transport_send(conn_id, ack).await?;
                        }
                    }

                    MsgType::Unsubscribe => {
                        if let Some(channel) = &msg.channel {
                            self.channels.unsubscribe(channel, &conn_id);
                            let ack = MtwMessage::response(
                                &msg.id,
                                mtw_protocol::Payload::Json(serde_json::json!({
                                    "unsubscribed": channel,
                                })),
                            );
                            transport_send(conn_id, ack).await?;
                        }
                    }

                    MsgType::Publish => {
                        if let Some(channel_name) = &msg.channel {
                            if let Some(channel) = self.channels.get(channel_name) {
                                channel.publish(msg, Some(&conn_id)).await?;
                            } else {
                                let err = MtwMessage::error(404, format!("channel '{}' not found", channel_name));
                                transport_send(conn_id, err).await?;
                            }
                        }
                    }

                    MsgType::Ping => {
                        let pong = MtwMessage::response(&msg.id, mtw_protocol::Payload::None);
                        transport_send(conn_id, pong).await?;
                    }

                    other => {
                        // Check registered handlers
                        for (msg_type, handler) in &self.handlers {
                            if msg_type == other {
                                if let Some(response) = handler(conn_id.clone(), msg.clone()).await? {
                                    transport_send(conn_id.clone(), response).await?;
                                }
                            }
                        }
                    }
                }

                Ok(())
            }

            TransportEvent::Binary(conn_id, data) => {
                tracing::debug!(conn_id = %conn_id, size = data.len(), "received binary data");
                Ok(())
            }

            TransportEvent::Error(conn_id, error) => {
                tracing::error!(conn_id = %conn_id, error = %error, "transport error");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::MiddlewareChain;

    #[test]
    fn test_router_creation() {
        let channels = ChannelManager::new();
        let middleware = MiddlewareChain::new();
        let router = MtwRouter::new(channels, middleware);
        assert!(router.handlers.is_empty());
    }

    #[tokio::test]
    async fn test_handle_connect_disconnect() {
        let channels = ChannelManager::new();
        let middleware = MiddlewareChain::new();
        let router = MtwRouter::new(channels, middleware);

        let meta = mtw_protocol::ConnMetadata {
            conn_id: "conn1".to_string(),
            remote_addr: Some("127.0.0.1:1234".to_string()),
            user_agent: None,
            auth: None,
            connected_at: 0,
        };

        let send_fn = |_conn_id: ConnId, _msg: MtwMessage| -> futures::future::BoxFuture<'_, Result<(), MtwError>> {
            Box::pin(async { Ok(()) })
        };

        // Connect
        router
            .handle_event(TransportEvent::Connected("conn1".to_string(), meta), &send_fn)
            .await
            .unwrap();

        // Disconnect
        router
            .handle_event(
                TransportEvent::Disconnected(
                    "conn1".to_string(),
                    mtw_protocol::DisconnectReason::Normal,
                ),
                &send_fn,
            )
            .await
            .unwrap();
    }
}
