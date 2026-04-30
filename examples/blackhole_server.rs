//! Black Hole — server-authoritative physics demo.
//!
//! The server runs a gravity simulation at 120Hz over ~500 particles around a
//! central mass. Every 33ms (~30Hz) it broadcasts the full position+hue
//! snapshot to every connected viewer. Clients can click to add impulse or
//! spawn new particles; the server is the single source of truth.
//!
//! This demonstrates:
//!   * low-latency state broadcast from authoritative server
//!   * flat-array JSON payload (no per-frame allocations in the hot path
//!     beyond the JSON encode)
//!   * mouse input round-trip: click → server mutates state → 30ms later
//!     every viewer sees the effect
//!
//! Run:
//!   BIND=0.0.0.0:17743 cargo run --release --jobs $(nproc) --bin blackhole-server
//!   xdg-open 'examples/blackhole_viewer.html?port=17743'

use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const N_PARTICLES: usize = 600;
const PHYS_HZ: f32 = 120.0;
const SNAP_HZ: f32 = 30.0;
const BH_MASS: f32 = 0.08; // gravitational constant × central mass
const EVENT_HORIZON: f32 = 0.035;
const SPAWN_R: f32 = 1.05;

#[derive(Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    hue: u16,
}

struct Sim {
    particles: Vec<Particle>,
    rng: u64,
    spawned: u64,
    consumed: u64,
}

impl Sim {
    fn new() -> Self {
        let mut s = Sim {
            particles: Vec::with_capacity(N_PARTICLES),
            rng: 0xcafef00d_deadbeef,
            spawned: 0,
            consumed: 0,
        };
        for _ in 0..N_PARTICLES {
            let p = s.fresh_particle();
            s.particles.push(p);
        }
        s
    }

    fn next_f32(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        ((x >> 11) as f32) / ((1u64 << 53) as f32)
    }

    fn fresh_particle(&mut self) -> Particle {
        // spawn on outer ring with near-circular tangential velocity
        let theta = self.next_f32() * std::f32::consts::TAU;
        let r = SPAWN_R * (0.92 + 0.08 * self.next_f32());
        let x = r * theta.cos();
        let y = r * theta.sin();
        // circular orbit speed = sqrt(mass / r); add small jitter for variety
        let v_orbit = (BH_MASS / r).sqrt() * (0.85 + 0.35 * self.next_f32());
        // tangential direction (perpendicular to radius), choose CCW
        let vx = -y / r * v_orbit;
        let vy = x / r * v_orbit;
        // slight radial inward component so orbits decay → accretion
        let inward = 0.04 * v_orbit;
        let vx = vx - (x / r) * inward;
        let vy = vy - (y / r) * inward;
        let hue = ((self.next_f32() * 260.0) as u16 + 200) % 360; // blues/purples/pinks
        self.spawned += 1;
        Particle { x, y, vx, vy, hue }
    }

    fn step(&mut self, dt: f32) {
        let eps2 = 0.004;
        let escape_r2 = (SPAWN_R * 1.35) * (SPAWN_R * 1.35);
        let horizon_r2 = EVENT_HORIZON * EVENT_HORIZON;
        for i in 0..self.particles.len() {
            let p = &mut self.particles[i];
            let r2 = p.x * p.x + p.y * p.y + eps2;
            let r = r2.sqrt();
            // a = -G * M * r̂ / r² => a_x = -M * x / r³
            let inv_r3 = 1.0 / (r2 * r);
            let ax = -BH_MASS * p.x * inv_r3;
            let ay = -BH_MASS * p.y * inv_r3;
            p.vx += ax * dt;
            p.vy += ay * dt;
            // drag so orbits spiral in over time
            p.vx *= 0.9996;
            p.vy *= 0.9996;
            p.x += p.vx * dt;
            p.y += p.vy * dt;

            let real_r2 = p.x * p.x + p.y * p.y;
            if real_r2 < horizon_r2 || real_r2 > escape_r2 {
                let fresh = self.fresh_particle();
                if real_r2 < horizon_r2 {
                    self.consumed += 1;
                }
                self.particles[i] = fresh;
            }
        }
    }

    fn impulse(&mut self, cx: f32, cy: f32, radius: f32, strength: f32) {
        let r2 = radius * radius;
        for p in self.particles.iter_mut() {
            let dx = p.x - cx;
            let dy = p.y - cy;
            let d2 = dx * dx + dy * dy;
            if d2 < r2 {
                let falloff = 1.0 - (d2 / r2);
                let d = d2.sqrt().max(0.001);
                // outward kick from click point
                p.vx += (dx / d) * strength * falloff;
                p.vy += (dy / d) * strength * falloff;
            }
        }
    }

    fn snapshot(&self, positions: &mut Vec<f32>, hues: &mut Vec<u16>) {
        positions.clear();
        hues.clear();
        for p in &self.particles {
            positions.push(round3(p.x));
            positions.push(round3(p.y));
            hues.push(p.hue);
        }
    }
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let bind: SocketAddr = std::env::var("BIND")
        .unwrap_or_else(|_| "0.0.0.0:17743".into())
        .parse()?;

    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut event_rx = transport.take_event_receiver().unwrap();
    transport.listen(bind).await?;
    let transport: Arc<WebSocketTransport> = Arc::new(transport);

    tracing::info!(%bind, n_particles=N_PARTICLES, phys_hz=PHYS_HZ, snap_hz=SNAP_HZ,
        "black hole server up");

    let sim = Arc::new(Mutex::new(Sim::new()));
    let msgs_out = Arc::new(AtomicU64::new(0));
    let msgs_in = Arc::new(AtomicU64::new(0));
    let started = Instant::now();

    // ── physics loop ────────────────────────────────────────────────
    {
        let sim = sim.clone();
        tokio::spawn(async move {
            let period = Duration::from_secs_f32(1.0 / PHYS_HZ);
            let mut tick = tokio::time::interval(period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let dt = 1.0 / PHYS_HZ;
            loop {
                tick.tick().await;
                sim.lock().unwrap().step(dt);
            }
        });
    }

    // ── snapshot broadcast loop ─────────────────────────────────────
    {
        let sim = sim.clone();
        let transport = transport.clone();
        let msgs_out = msgs_out.clone();
        tokio::spawn(async move {
            let period = Duration::from_secs_f32(1.0 / SNAP_HZ);
            let mut tick = tokio::time::interval(period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut positions: Vec<f32> = Vec::with_capacity(N_PARTICLES * 2);
            let mut hues: Vec<u16> = Vec::with_capacity(N_PARTICLES);
            loop {
                tick.tick().await;
                let (sp, cn) = {
                    let s = sim.lock().unwrap();
                    s.snapshot(&mut positions, &mut hues);
                    (s.spawned, s.consumed)
                };
                let msg = MtwMessage::new(
                    MsgType::Event,
                    Payload::Json(serde_json::json!({
                        "t": "snap",
                        "p": &positions,
                        "h": &hues,
                        "sp": sp,
                        "cn": cn,
                    })),
                );
                let conns = transport.connection_count() as u64;
                if transport.broadcast(msg).await.is_ok() {
                    msgs_out.fetch_add(conns, Ordering::Relaxed);
                }
            }
        });
    }

    // ── metrics 1Hz ─────────────────────────────────────────────────
    {
        let transport = transport.clone();
        let msgs_in = msgs_in.clone();
        let msgs_out = msgs_out.clone();
        let sim = sim.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let mut last_in = 0u64;
            let mut last_out = 0u64;
            loop {
                tick.tick().await;
                let in_now = msgs_in.load(Ordering::Relaxed);
                let out_now = msgs_out.load(Ordering::Relaxed);
                let conns = transport.connection_count() as u64;
                let (sp, cn) = {
                    let s = sim.lock().unwrap();
                    (s.spawned, s.consumed)
                };
                let m = MtwMessage::new(
                    MsgType::Event,
                    Payload::Json(serde_json::json!({
                        "t": "metrics",
                        "conns": conns,
                        "inbound_per_sec": in_now.saturating_sub(last_in),
                        "outbound_per_sec": out_now.saturating_sub(last_out),
                        "uptime_s": started.elapsed().as_secs(),
                        "spawned": sp,
                        "consumed": cn,
                        "particles": N_PARTICLES,
                    })),
                );
                last_in = in_now;
                last_out = out_now;
                let fanout = conns;
                if transport.broadcast(m).await.is_ok() {
                    msgs_out.fetch_add(fanout, Ordering::Relaxed);
                }
            }
        });
    }

    // ── input event loop ────────────────────────────────────────────
    loop {
        tokio::select! {
            Some(ev) = event_rx.recv() => match ev {
                TransportEvent::Message(_conn_id, msg) => {
                    msgs_in.fetch_add(1, Ordering::Relaxed);
                    if let Payload::Json(v) = &msg.payload {
                        match v.get("t").and_then(|t| t.as_str()) {
                            Some("impulse") => {
                                let x = v.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                                let y = v.get("y").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                                let strength = v.get("s").and_then(|x| x.as_f64()).unwrap_or(0.25) as f32;
                                sim.lock().unwrap().impulse(x, y, 0.22, strength);
                            }
                            Some("spawn") => {
                                let x = v.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                                let y = v.get("y").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                                let mut s = sim.lock().unwrap();
                                // replace one particle near the click with a fresh one starting at the click position
                                if let Some(idx) = (0..s.particles.len()).next() {
                                    let speed = (BH_MASS / (x*x + y*y + 0.01).sqrt()).sqrt() * 0.9;
                                    // perpendicular to radius for a tangential orbit
                                    let r = (x*x + y*y).sqrt().max(0.01);
                                    let vx = -y / r * speed;
                                    let vy =  x / r * speed;
                                    s.particles[idx] = Particle {
                                        x, y, vx, vy,
                                        hue: ((x.atan2(y) * 180.0 / std::f32::consts::PI + 180.0) as u16) % 360,
                                    };
                                }
                            }
                            _ => {}
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
