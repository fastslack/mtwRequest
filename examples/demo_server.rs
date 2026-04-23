//! Demo: mtwRequest server with WebSocket transport
//!
//! Run with: cargo run --example demo_server
//! Then connect with: cargo run --example demo_client

use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;
use mtw_router::{ChannelManager, MiddlewareChain, MtwRouter};
use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let addr: SocketAddr = "127.0.0.1:7741".parse()?;

    // Create WebSocket transport
    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut event_rx = transport.take_event_receiver().unwrap();

    // Start listening
    transport.listen(addr).await?;
    tracing::info!("mtwRequest server running on ws://{}", addr);

    // Create channel manager and router
    let mut channel_mgr = ChannelManager::new();
    // We don't spawn a forwarder loop — publish delivers directly via the
    // sink installed below. Drop the receiver so the unbounded mpsc is free.
    drop(channel_mgr.take_message_receiver());

    // Pre-create some channels
    channel_mgr.create_channel("chat.general", false, Some(100), 50);
    channel_mgr.create_channel("chat.random", false, None, 20);
    channel_mgr.create_channel("notifications", false, None, 10);

    let middleware = MiddlewareChain::new();
    let router = Arc::new(MtwRouter::new(channel_mgr, middleware));

    let transport = Arc::new(transport);

    // Hot-path direct delivery: publish pushes envelopes straight into each
    // connection's writer mpsc, skipping the central forwarder.
    router
        .channels()
        .set_direct_sink(transport.clone() as Arc<dyn mtw_protocol::EnvelopeSink>);

    // Main event loop
    tracing::info!("waiting for connections...");
    tracing::info!("channels: chat.general, chat.random, notifications");
    tracing::info!("press Ctrl+C to stop");

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                let transport_ref = transport.clone();
                let router_ref = router.clone();

                // Handle each event
                match &event {
                    TransportEvent::Connected(conn_id, meta) => {
                        tracing::info!(
                            conn_id = %conn_id,
                            addr = ?meta.remote_addr,
                            "new client connected"
                        );
                    }
                    TransportEvent::Disconnected(conn_id, reason) => {
                        tracing::info!(conn_id = %conn_id, reason = ?reason, "client disconnected");
                        router_ref.channels().remove_connection(conn_id);
                    }
                    TransportEvent::Message(conn_id, msg) => {
                        tracing::info!(
                            conn_id = %conn_id,
                            msg_type = ?msg.msg_type,
                            channel = ?msg.channel,
                            "received message"
                        );

                        match &msg.msg_type {
                            MsgType::Subscribe => {
                                if let Some(channel) = &msg.channel {
                                    match router_ref.channels().subscribe(channel, conn_id) {
                                        Ok(()) => {
                                            let ack = MtwMessage::response(
                                                &msg.id,
                                                Payload::Json(serde_json::json!({
                                                    "subscribed": channel,
                                                    "members": router_ref.channels()
                                                        .get(channel)
                                                        .map(|c| c.subscriber_count())
                                                        .unwrap_or(0),
                                                })),
                                            );
                                            let _ = transport_ref.send(conn_id, ack).await;
                                            tracing::info!(conn_id = %conn_id, channel = %channel, "subscribed");
                                        }
                                        Err(e) => {
                                            let err = MtwMessage::error(400, e.to_string());
                                            let _ = transport_ref.send(conn_id, err).await;
                                        }
                                    }
                                }
                            }

                            MsgType::Unsubscribe => {
                                if let Some(channel) = &msg.channel {
                                    router_ref.channels().unsubscribe(channel, conn_id);
                                    let ack = MtwMessage::response(
                                        &msg.id,
                                        Payload::Json(serde_json::json!({"unsubscribed": channel})),
                                    );
                                    let _ = transport_ref.send(conn_id, ack).await;
                                }
                            }

                            MsgType::Publish => {
                                if let Some(channel_name) = &msg.channel {
                                    if let Some(channel) = router_ref.channels().get(channel_name) {
                                        let count = channel.publish(msg.clone(), Some(conn_id)).await.unwrap_or(0);
                                        tracing::info!(
                                            channel = %channel_name,
                                            recipients = count,
                                            "message published"
                                        );
                                        // Ack to sender
                                        let ack = MtwMessage::response(
                                            &msg.id,
                                            Payload::Json(serde_json::json!({
                                                "published": channel_name,
                                                "recipients": count,
                                            })),
                                        );
                                        let _ = transport_ref.send(conn_id, ack).await;
                                    } else {
                                        let err = MtwMessage::error(404, format!("channel '{}' not found", channel_name));
                                        let _ = transport_ref.send(conn_id, err).await;
                                    }
                                }
                            }

                            MsgType::Request => {
                                // Echo request back as response
                                let response = MtwMessage::response(&msg.id, msg.payload.clone());
                                let _ = transport_ref.send(conn_id, response).await;
                            }

                            MsgType::Ping => {
                                let pong = MtwMessage::new(MsgType::Pong, Payload::None)
                                    .with_ref(&msg.id);
                                let _ = transport_ref.send(conn_id, pong).await;
                            }

                            _ => {
                                tracing::debug!(msg_type = ?msg.msg_type, "unhandled message type");
                            }
                        }
                    }
                    TransportEvent::Binary(conn_id, data) => {
                        tracing::info!(conn_id = %conn_id, size = data.len(), "binary data");
                    }
                    TransportEvent::Error(conn_id, error) => {
                        tracing::error!(conn_id = %conn_id, error = %error, "transport error");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down...");
                transport.shutdown().await?;
                break;
            }
        }
    }

    Ok(())
}
