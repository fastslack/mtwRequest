//! Hive Mind — N-to-N pub/sub mesh stress demo.
//!
//! Every client (bot or viewer) is auto-subscribed to the `hive` channel on
//! connect. When any client publishes, the core routes the envelope through
//! the zero-copy `channel.publish` hot path: a single ArcSwap load, no DashMap
//! lookups, one `ConnTarget.deliver` per subscriber. This is the demo for
//! mesh throughput — every bot is both a publisher AND a subscriber.
//!
//! Run:
//!   BIND=0.0.0.0:17742 cargo run --release --jobs $(nproc) --bin hive-server
//!
//! Then:
//!   cargo run --release --jobs $(nproc) --bin hive-bot -- --count 1000 --url ws://127.0.0.1:17742/ws
//!   xdg-open 'examples/hive_viewer.html?port=17742'

use mtw_protocol::{EnvelopeSink, MsgType, MtwMessage, Payload, TransportEvent};
use mtw_router::ChannelManager;
use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CHANNEL: &str = "hive";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let bind: SocketAddr = std::env::var("BIND")
        .unwrap_or_else(|_| "0.0.0.0:17742".into())
        .parse()?;

    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut event_rx = transport.take_event_receiver().unwrap();
    transport.listen(bind).await?;
    let transport: Arc<WebSocketTransport> = Arc::new(transport);

    // Pub/sub channel with the zero-copy hot path wired in.
    let mut mgr = ChannelManager::new();
    drop(mgr.take_message_receiver()); // we use the direct sink path only
    mgr.set_direct_sink(transport.clone() as Arc<dyn EnvelopeSink>);
    let mgr = Arc::new(mgr);
    mgr.create_channel(CHANNEL, false, None, 0);

    tracing::info!(%bind, "hive mind server up");
    tracing::info!("open examples/hive_viewer.html?port={} and fire bots", bind.port());

    let msgs_in = Arc::new(AtomicU64::new(0));
    let msgs_out = Arc::new(AtomicU64::new(0));
    let started = Instant::now();

    // Metrics loop: broadcasts a `metrics` event every second to every
    // subscriber so the viewer HUD stays live.
    {
        let transport = transport.clone();
        let msgs_in = msgs_in.clone();
        let msgs_out = msgs_out.clone();
        let mgr_metrics = mgr.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let mut last_in = 0u64;
            let mut last_out = 0u64;
            loop {
                tick.tick().await;
                let in_now = msgs_in.load(Ordering::Relaxed);
                let out_now = msgs_out.load(Ordering::Relaxed);
                let conns = transport.connection_count() as u64;
                let inbound_ps = in_now.saturating_sub(last_in);
                let outbound_ps = out_now.saturating_sub(last_out);
                last_in = in_now;
                last_out = out_now;
                let m = MtwMessage::new(
                    MsgType::Event,
                    Payload::Json(serde_json::json!({
                        "t": "metrics",
                        "conns": conns,
                        "inbound_per_sec": inbound_ps,
                        "outbound_per_sec": outbound_ps,
                        "uptime_s": started.elapsed().as_secs(),
                    })),
                );
                if transport.broadcast(m).await.is_ok() {
                    msgs_out.fetch_add(conns, Ordering::Relaxed);
                }
                let subs = mgr_metrics
                    .get(CHANNEL)
                    .map(|c| c.subscriber_count())
                    .unwrap_or(0);
                tracing::info!(
                    conns,
                    subs,
                    inbound_per_sec = inbound_ps,
                    outbound_per_sec = outbound_ps,
                    "tick"
                );
            }
        });
    }

    // Event loop.
    loop {
        tokio::select! {
            Some(ev) = event_rx.recv() => match ev {
                TransportEvent::Connected(conn_id, _) => {
                    // Auto-subscribe every new connection to the hive.
                    if let Err(e) = mgr.subscribe(CHANNEL, &conn_id) {
                        tracing::warn!(%conn_id, error=%e, "subscribe failed");
                    }
                }
                TransportEvent::Disconnected(conn_id, _) => {
                    mgr.remove_connection(&conn_id);
                }
                TransportEvent::Error(conn_id, err) => {
                    tracing::warn!(%conn_id, %err, "transport error");
                }
                TransportEvent::Message(conn_id, msg) => {
                    msgs_in.fetch_add(1, Ordering::Relaxed);
                    // Forward any publish-style message on the hive channel.
                    // We accept both MsgType::Publish and MsgType::Event so
                    // bots can use either.
                    let is_hive = msg.channel.as_deref() == Some(CHANNEL)
                        || matches!(&msg.payload, Payload::Json(v)
                            if v.get("t").and_then(|t| t.as_str()) == Some("pos"));
                    if is_hive {
                        if let Some(ch) = mgr.get(CHANNEL) {
                            // Exclude the sender — they don't need to see their own echo.
                            let sent = ch.publish(msg, Some(&conn_id)).await.unwrap_or(0);
                            msgs_out.fetch_add(sent as u64, Ordering::Relaxed);
                        }
                    }
                }
                _ => {}
            },
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                transport.shutdown().await?;
                return Ok(());
            }
        }
    }
}
