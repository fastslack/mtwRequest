//! Minimal mtwRequest server tuned for benchmarks:
//!   - WebSocket transport on 0.0.0.0:7741/ws
//!   - Pre-created "bench" channel with no subscriber cap
//!   - Log level WARN by default (silence the per-message tracing overhead)
//!   - Publishes are broadcast to subscribers *excluding* the publisher
//!     (matches the `self_delivers=false` semantics in bench-clients).
//!
//! Build with `--features profile` to enable in-process sampling via
//! `pprof-rs`. A flamegraph.svg is written on SIGTERM / Ctrl+C.

use anyhow::Result;
use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use mtw_router::ChannelManager;
use mtw_transport::{ws::WebSocketTransport, MtwTransport};
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string()))
        .init();

    #[cfg(feature = "profile")]
    let _profiler_guard = {
        let freq_hz: i32 = std::env::var("BENCH_MTW_PROFILE_HZ")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(499); // prime to avoid lockstep with periodic tasks
        tracing::warn!(hz = freq_hz, "pprof profiler enabled");
        pprof::ProfilerGuardBuilder::default()
            .frequency(freq_hz)
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
            .ok()
    };

    let bind = std::env::var("BENCH_MTW_BIND").unwrap_or_else(|_| "0.0.0.0:7741".to_string());
    let addr: SocketAddr = bind.parse()?;

    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut event_rx = transport.take_event_receiver().expect("event receiver");
    transport.listen(addr).await?;
    tracing::warn!("bench-mtw-server listening on ws://{}/ws", addr);

    let mut channel_mgr = ChannelManager::new();
    let _channel_rx = channel_mgr.take_message_receiver();
    channel_mgr.create_channel("bench", false, None, 0);
    let channels = Arc::new(channel_mgr);

    let transport = Arc::new(transport);

    // Hot-path delivery: `Channel::publish` pushes envelopes straight into
    // each connection's writer mpsc via this sink, eliminating the central
    // forwarder task and its serial bottleneck.
    channels.set_direct_sink(transport.clone() as Arc<dyn mtw_protocol::EnvelopeSink>);

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    TransportEvent::Disconnected(conn_id, _) => {
                        channels.remove_connection(&conn_id);
                    }
                    TransportEvent::Message(conn_id, msg) => {
                        match msg.msg_type {
                            MsgType::Subscribe => {
                                if let Some(ch) = &msg.channel {
                                    if let Err(e) = channels.subscribe(ch, &conn_id) {
                                        let err = MtwMessage::error(400, e.to_string());
                                        let _ = transport.send(&conn_id, err).await;
                                    }
                                }
                            }
                            MsgType::Unsubscribe => {
                                if let Some(ch) = &msg.channel {
                                    channels.unsubscribe(ch, &conn_id);
                                }
                            }
                            MsgType::Publish => {
                                // Take the channel name out so we can move
                                // `msg` into publish() without cloning the
                                // full MtwMessage (HashMap + Payload + ...).
                                if let Some(ch_name) = msg.channel.clone() {
                                    if let Some(channel) = channels.get(&ch_name) {
                                        let _ = channel.publish(msg, Some(&conn_id)).await;
                                    }
                                }
                            }
                            MsgType::Ping => {
                                let pong = MtwMessage::new(MsgType::Pong, Payload::None)
                                    .with_ref(&msg.id);
                                let _ = transport.send(&conn_id, pong).await;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::warn!("shutting down");
                transport.shutdown().await?;
                break;
            }
        }
    }

    #[cfg(feature = "profile")]
    if let Some(guard) = _profiler_guard {
        if let Ok(report) = guard.report().build() {
            let out = std::env::var("BENCH_MTW_PROFILE_OUT")
                .unwrap_or_else(|_| "flamegraph.svg".to_string());
            match std::fs::File::create(&out) {
                Ok(file) => {
                    if let Err(e) = report.flamegraph(file) {
                        tracing::error!(error = %e, "flamegraph write failed");
                    } else {
                        tracing::warn!(path = %out, "flamegraph written");
                    }
                }
                Err(e) => tracing::error!(error = %e, path = %out, "flamegraph create failed"),
            }
        }
    }

    Ok(())
}
