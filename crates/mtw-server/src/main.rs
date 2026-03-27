//! mtwRequest Server
//!
//! Production binary that runs the mtwRequest real-time framework.
//! Now with native data store (SQLite) and bridge support.
//!
//! Usage:
//!   mtw-server                     # loads mtw.toml from current dir
//!   mtw-server --config path.toml  # custom config path
//!   MTW_HOST=0.0.0.0 MTW_PORT=9090 mtw-server  # env overrides

use std::net::SocketAddr;
use std::sync::Arc;

use mtw_core::MtwConfig;
use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use mtw_router::{ChannelManager, MiddlewareChain, MtwRouter};
use mtw_store::MtwStore;
use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,mtw=debug".into()),
        )
        .init();

    // Load config
    let config = load_config()?;

    let host = std::env::var("MTW_HOST").unwrap_or_else(|_| config.server.host.clone());
    let port: u16 = std::env::var("MTW_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(config.server.port);

    let ws_path = &config.transport.websocket.path;
    let ping_interval = config.transport.websocket.ping_interval;

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;

    // ── Data Store ──────────────────────────────────────────
    let store: Option<Arc<dyn MtwStore>> = if let Some(ref store_cfg) = config.store {
        let st = mtw_store::from_config(&mtw_store::StoreConfig {
            path: store_cfg.path.clone(),
            readonly: store_cfg.readonly,
            pool_size: store_cfg.pool_size,
            cache_mb: store_cfg.cache_mb,
            mmap_mb: store_cfg.mmap_mb,
            busy_timeout_ms: store_cfg.busy_timeout_ms,
        })?;
        let info = st.info().await?;
        tracing::info!(
            tables = %info["table_count"],
            size = %info["size_mb"],
            "store ready"
        );
        Some(Arc::from(st))
    } else {
        tracing::info!("no [store] configured — running without data store");
        None
    };

    // ── Bridge (optional — connects when external service is available) ──
    let bridge: Option<Arc<dyn mtw_bridge::MtwBridge>> =
        if let Some(ref bridge_cfg) = config.store.as_ref().and_then(|s| s.bridge.as_ref()) {
            match mtw_bridge::from_config(&mtw_bridge::BridgeConfig {
                socket: bridge_cfg.socket.clone(),
                address: bridge_cfg.address.clone(),
                timeout_ms: bridge_cfg.timeout_ms,
            })
            .await
            {
                Ok(br) => {
                    tracing::info!("bridge connected");
                    Some(Arc::from(br))
                }
                Err(e) => {
                    tracing::warn!("bridge not available (will work without it): {}", e);
                    None
                }
            }
        } else {
            None
        };

    // ── WebSocket Transport ─────────────────────────────────
    let mut transport = WebSocketTransport::new(ws_path, ping_interval);
    let mut event_rx = transport
        .take_event_receiver()
        .expect("event receiver already taken");

    transport.listen(addr).await?;

    tracing::info!("mtwRequest server v{}", env!("CARGO_PKG_VERSION"));
    tracing::info!("listening on ws://{}{}", addr, ws_path);

    // ── Channels & Router ───────────────────────────────────
    let mut channel_mgr = ChannelManager::new();
    let mut channel_rx = channel_mgr.take_message_receiver().unwrap();

    for ch_config in &config.channels {
        let max_members = ch_config.max_members;
        let history = ch_config.history.unwrap_or(0);
        channel_mgr.create_channel(&ch_config.name, ch_config.auth, max_members, history);
        tracing::info!(
            channel = %ch_config.name,
            max_members = ?max_members,
            history = history,
            "channel created"
        );
    }

    let middleware = MiddlewareChain::new();
    let router = Arc::new(MtwRouter::new(channel_mgr, middleware));
    let transport = Arc::new(transport);

    // ── Channel message forwarder ───────────────────────────
    let transport_fwd = transport.clone();
    tokio::spawn(async move {
        while let Some((conn_id, msg)) = channel_rx.recv().await {
            if let Err(e) = transport_fwd.send(&conn_id, msg).await {
                tracing::warn!(conn_id = %conn_id, error = %e, "failed to forward channel message");
            }
        }
    });

    tracing::info!("ready — waiting for connections (Ctrl+C to stop)");

    // ── Main event loop ─────────────────────────────────────
    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                handle_event(event, &transport, &router, &store, &bridge).await;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutdown signal received");
                transport.shutdown().await?;
                break;
            }
        }
    }

    tracing::info!("server stopped");
    Ok(())
}

/// Load config from file or create default
fn load_config() -> Result<MtwConfig, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = if let Some(idx) = args.iter().position(|a| a == "--config") {
        args.get(idx + 1).map(|s| s.as_str())
    } else {
        None
    };

    if let Some(path) = config_path {
        tracing::info!(path = %path, "loading config");
        return Ok(MtwConfig::from_file(path)?);
    }

    for path in &["mtw.toml", "config/mtw.toml", "/etc/mtw/mtw.toml"] {
        if std::path::Path::new(path).exists() {
            tracing::info!(path = %path, "loading config");
            return Ok(MtwConfig::from_file(path)?);
        }
    }

    tracing::info!("no config file found, using defaults");
    Ok(MtwConfig::default_config())
}

/// Handle a single transport event
async fn handle_event(
    event: TransportEvent,
    transport: &Arc<WebSocketTransport>,
    router: &Arc<MtwRouter>,
    store: &Option<Arc<dyn MtwStore>>,
    bridge: &Option<Arc<dyn mtw_bridge::MtwBridge>>,
) {
    match event {
        TransportEvent::Connected(ref conn_id, ref meta) => {
            tracing::info!(
                conn_id = %conn_id,
                addr = ?meta.remote_addr,
                "client connected"
            );
        }

        TransportEvent::Disconnected(ref conn_id, ref reason) => {
            tracing::info!(conn_id = %conn_id, reason = ?reason, "client disconnected");
            router.channels().remove_connection(conn_id);
        }

        TransportEvent::Message(ref conn_id, ref msg) => {
            if let Err(e) = handle_message(conn_id, msg, transport, router, store, bridge).await {
                tracing::error!(conn_id = %conn_id, error = %e, "message handling error");
                let err_msg = MtwMessage::error(500, e.to_string());
                let _ = transport.send(conn_id, err_msg).await;
            }
        }

        TransportEvent::Binary(ref conn_id, ref data) => {
            tracing::debug!(conn_id = %conn_id, size = data.len(), "binary data received");
        }

        TransportEvent::Error(ref conn_id, ref error) => {
            tracing::error!(conn_id = %conn_id, error = %error, "transport error");
        }
    }
}

/// Handle a single incoming message
async fn handle_message(
    conn_id: &String,
    msg: &MtwMessage,
    transport: &Arc<WebSocketTransport>,
    router: &Arc<MtwRouter>,
    store: &Option<Arc<dyn MtwStore>>,
    bridge: &Option<Arc<dyn mtw_bridge::MtwBridge>>,
) -> Result<(), mtw_core::MtwError> {
    match &msg.msg_type {
        MsgType::Subscribe => {
            if let Some(channel) = &msg.channel {
                router.channels().subscribe(channel, conn_id)?;
                let ack = MtwMessage::response(
                    &msg.id,
                    Payload::Json(serde_json::json!({
                        "subscribed": channel,
                        "members": router.channels()
                            .get(channel)
                            .map(|c| c.subscriber_count())
                            .unwrap_or(0),
                    })),
                );
                transport.send(conn_id, ack).await?;
                tracing::debug!(conn_id = %conn_id, channel = %channel, "subscribed");
            }
        }

        MsgType::Unsubscribe => {
            if let Some(channel) = &msg.channel {
                router.channels().unsubscribe(channel, conn_id);
                let ack = MtwMessage::response(
                    &msg.id,
                    Payload::Json(serde_json::json!({"unsubscribed": channel})),
                );
                transport.send(conn_id, ack).await?;
            }
        }

        MsgType::Publish => {
            if let Some(channel_name) = &msg.channel {
                if let Some(channel) = router.channels().get(channel_name) {
                    let count = channel.publish(msg.clone(), Some(conn_id)).await?;
                    let ack = MtwMessage::response(
                        &msg.id,
                        Payload::Json(serde_json::json!({
                            "published": channel_name,
                            "recipients": count,
                        })),
                    );
                    transport.send(conn_id, ack).await?;
                } else {
                    let err =
                        MtwMessage::error(404, format!("channel '{}' not found", channel_name));
                    transport.send(conn_id, err).await?;
                }
            }
        }

        // ── Store Query: read data directly from SQLite (fast path) ──
        MsgType::Request => {
            let response = handle_request(msg, store, bridge).await;
            transport.send(conn_id, response).await?;
        }

        MsgType::Connect => {
            // Client handshake — respond with ack containing conn_id
            let ack = MtwMessage::new(
                MsgType::Ack,
                Payload::Json(serde_json::json!({ "conn_id": conn_id })),
            )
            .with_ref(&msg.id);
            transport.send(conn_id, ack).await?;
            tracing::debug!(conn_id = %conn_id, "connect handshake completed");
        }

        MsgType::Ping => {
            let pong = MtwMessage::new(MsgType::Pong, Payload::None).with_ref(&msg.id);
            transport.send(conn_id, pong).await?;
        }

        MsgType::Disconnect => {
            tracing::debug!(conn_id = %conn_id, "client requested disconnect");
            transport.close(conn_id).await?;
        }

        _ => {
            tracing::debug!(conn_id = %conn_id, msg_type = ?msg.msg_type, "unhandled message type");
        }
    }

    Ok(())
}

/// Handle a Request message — route to store (reads) or bridge (writes)
async fn handle_request(
    msg: &MtwMessage,
    store: &Option<Arc<dyn MtwStore>>,
    bridge: &Option<Arc<dyn mtw_bridge::MtwBridge>>,
) -> MtwMessage {
    // Extract action from metadata or payload
    let action = msg
        .metadata
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let tool = msg
        .metadata
        .get("tool")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // ── Route: query (read) → Store (Rust, fast) ──
    if action == "query" || action == "query_raw" {
        if let Some(store) = store {
            let result = if action == "query_raw" {
                let sql = msg.payload.as_text().unwrap_or("");
                let params = msg
                    .metadata
                    .get("params")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                store.query_raw(sql, &params).await
            } else {
                let table = msg.payload.as_text().unwrap_or("");
                store.query(table, serde_json::json!({})).await
            };

            return match result {
                Ok(data) => MtwMessage::response(&msg.id, Payload::Json(data)),
                Err(e) => MtwMessage::error(500, e.to_string()).with_ref(&msg.id),
            };
        }
        return MtwMessage::error(503, "no store configured").with_ref(&msg.id);
    }

    // ── Route: store_info → Store metadata ──
    if action == "store_info" {
        if let Some(store) = store {
            return match store.info().await {
                Ok(info) => MtwMessage::response(&msg.id, Payload::Json(info)),
                Err(e) => MtwMessage::error(500, e.to_string()).with_ref(&msg.id),
            };
        }
        return MtwMessage::error(503, "no store configured").with_ref(&msg.id);
    }

    // ── Route: tool call → Bridge (external service) ──
    if !tool.is_empty() {
        if let Some(bridge) = bridge {
            let args = match &msg.payload {
                Payload::Json(v) => v.clone(),
                Payload::Text(t) => serde_json::json!({"input": t}),
                _ => serde_json::json!({}),
            };

            return match bridge.call_tool(tool, args).await {
                Ok(result) => MtwMessage::response(&msg.id, Payload::Json(result)),
                Err(e) => MtwMessage::error(500, e.to_string()).with_ref(&msg.id),
            };
        }
        return MtwMessage::error(503, "no bridge configured").with_ref(&msg.id);
    }

    // ── Fallback: echo ──
    MtwMessage::response(&msg.id, msg.payload.clone())
}
