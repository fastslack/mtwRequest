//! Orbital Swarm — stress demo server.
//!
//! Broadcasts periodic "pulse" events with a random origin on the unit sphere
//! to every connected WS client. Once per second, also broadcasts a "metrics"
//! event so the viewer's HUD can show live connection count, outbound msgs/sec,
//! and uptime.
//!
//! Run:
//!     cargo run --release --bin swarm-server
//!     # optional: PULSE_HZ=30 cargo run --release --bin swarm-server
//!
//! Then open examples/swarm_viewer.html in a browser and fire up bots:
//!     cargo run --release --bin swarm-bot -- --count 10000

use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let bind: SocketAddr = std::env::var("BIND")
        .unwrap_or_else(|_| "0.0.0.0:7741".into())
        .parse()?;

    let pulse_hz: f32 = std::env::var("PULSE_HZ")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0);

    let mut transport = WebSocketTransport::new("/ws", 30);
    let event_rx = transport.take_event_receiver().unwrap();
    transport.listen(bind).await?;
    let transport = Arc::new(transport);

    tracing::info!(%bind, pulse_hz, "orbital swarm server up");
    tracing::info!("open examples/swarm_viewer.html and connect to ws://{}/ws", bind);

    let msgs_sent = Arc::new(AtomicU64::new(0));
    let pulses_sent = Arc::new(AtomicU64::new(0));
    let started = Instant::now();

    spawn_pulse_loop(
        transport.clone(),
        msgs_sent.clone(),
        pulses_sent.clone(),
        pulse_hz,
    );
    spawn_metrics_loop(
        transport.clone(),
        msgs_sent.clone(),
        pulses_sent.clone(),
        started,
    );
    spawn_event_loop(transport.clone(), event_rx, pulses_sent.clone(), msgs_sent.clone());

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    transport.shutdown().await?;
    Ok(())
}

fn spawn_pulse_loop(
    transport: Arc<WebSocketTransport>,
    msgs_sent: Arc<AtomicU64>,
    pulses_sent: Arc<AtomicU64>,
    pulse_hz: f32,
) {
    let period = Duration::from_secs_f32(1.0 / pulse_hz.max(0.1));
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut rng = XorShift::seeded();
        loop {
            tick.tick().await;
            let (ox, oy, oz) = rng.unit_vec3();
            let pulse_id = pulses_sent.fetch_add(1, Ordering::Relaxed) + 1;
            let hue = (pulse_id.wrapping_mul(67) % 360) as u32;
            let ts_ms = now_ms();

            let msg = MtwMessage::new(
                MsgType::Event,
                Payload::Json(serde_json::json!({
                    "t": "pulse",
                    "id": pulse_id,
                    "ox": ox, "oy": oy, "oz": oz,
                    "h": hue,
                    "ts": ts_ms,
                })),
            );
            let fanout = transport.connection_count() as u64;
            if transport.broadcast(msg).await.is_ok() {
                msgs_sent.fetch_add(fanout, Ordering::Relaxed);
            }
        }
    });
}

fn spawn_metrics_loop(
    transport: Arc<WebSocketTransport>,
    msgs_sent: Arc<AtomicU64>,
    pulses_sent: Arc<AtomicU64>,
    started: Instant,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut last_msgs = 0u64;
        let mut last_pulses = 0u64;
        loop {
            tick.tick().await;
            let msgs = msgs_sent.load(Ordering::Relaxed);
            let pulses = pulses_sent.load(Ordering::Relaxed);
            let mps = msgs.saturating_sub(last_msgs);
            let pps = pulses.saturating_sub(last_pulses);
            last_msgs = msgs;
            last_pulses = pulses;
            let conns = transport.connection_count() as u64;
            let uptime_s = started.elapsed().as_secs();

            // The metrics event itself is broadcast, so it contributes N more
            // deliveries this tick. That's fine — the HUD will reflect it.
            let msg = MtwMessage::new(
                MsgType::Event,
                Payload::Json(serde_json::json!({
                    "t": "metrics",
                    "conns": conns,
                    "msgs_per_sec": mps,
                    "pulses_per_sec": pps,
                    "uptime_s": uptime_s,
                })),
            );
            let fanout = conns;
            if transport.broadcast(msg).await.is_ok() {
                msgs_sent.fetch_add(fanout, Ordering::Relaxed);
            }

            if uptime_s % 5 == 0 {
                tracing::info!(
                    conns,
                    msgs_per_sec = mps,
                    pulses_per_sec = pps,
                    uptime_s,
                    "tick"
                );
            }
        }
    });
}

fn spawn_event_loop(
    transport: Arc<WebSocketTransport>,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<TransportEvent>,
    pulses_sent: Arc<AtomicU64>,
    msgs_sent: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        while let Some(ev) = event_rx.recv().await {
            match ev {
                TransportEvent::Connected(id, _) => tracing::debug!(%id, "connected"),
                TransportEvent::Disconnected(id, _) => tracing::debug!(%id, "disconnected"),
                TransportEvent::Message(_id, msg) => {
                    // User-triggered click_pulse: re-broadcast so every viewer
                    // sees the same ripple.
                    if let Payload::Json(v) = &msg.payload {
                        if v.get("t").and_then(|t| t.as_str()) == Some("click_pulse") {
                            let ox = v.get("ox").and_then(|x| x.as_f64()).unwrap_or(0.0);
                            let oy = v.get("oy").and_then(|x| x.as_f64()).unwrap_or(0.0);
                            let oz = v.get("oz").and_then(|x| x.as_f64()).unwrap_or(1.0);
                            let pulse_id = pulses_sent.fetch_add(1, Ordering::Relaxed) + 1;
                            let hue = v
                                .get("h")
                                .and_then(|x| x.as_u64())
                                .unwrap_or((now_ms() % 360) as u64);
                            let out = MtwMessage::new(
                                MsgType::Event,
                                Payload::Json(serde_json::json!({
                                    "t": "pulse",
                                    "id": pulse_id,
                                    "ox": ox, "oy": oy, "oz": oz,
                                    "h": hue,
                                    "ts": now_ms(),
                                    "user": true,
                                })),
                            );
                            let fanout = transport.connection_count() as u64;
                            if transport.broadcast(out).await.is_ok() {
                                msgs_sent.fetch_add(fanout, Ordering::Relaxed);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    });
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Tiny PRNG (xorshift64*) so we don't pull in the `rand` crate.
struct XorShift(u64);
impl XorShift {
    fn seeded() -> Self {
        Self(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
                | 1,
        )
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 8) as f32) / ((1u64 << 56) as f32)
    }
    /// Uniform random point on the unit sphere (Marsaglia method).
    fn unit_vec3(&mut self) -> (f32, f32, f32) {
        let phi = self.next_f32() * std::f32::consts::TAU;
        let cos_theta = self.next_f32() * 2.0 - 1.0;
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        (sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta)
    }
}
