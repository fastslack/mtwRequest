//! Hive Mind — synthetic bot swarm.
//!
//! Each bot:
//!   1. Connects to the hive server (auto-subscribed on connect)
//!   2. Publishes its own position at `PUB_HZ` (default 10Hz) as a JSON event
//!      on channel "hive"
//!   3. Reads incoming fanouts (discarded — viewer renders)
//!
//! Trajectory is deterministic per-bot: a swirling-galaxy pattern where each
//! bot's orbital radius and angular phase are a function of its index.
//!
//! Run:
//!   cargo run --release --jobs $(nproc) --bin hive-bot -- --count 1000

use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (count, url, concurrency, hz) = parse_args();
    println!(
        "hive bot · {count} bots · {hz} Hz · url {url} · connect-concurrency {concurrency}"
    );

    let live = Arc::new(AtomicU64::new(0));
    let published = Arc::new(AtomicU64::new(0));
    let received = Arc::new(AtomicU64::new(0));
    let sem = Arc::new(Semaphore::new(concurrency));
    let t0 = Instant::now();

    {
        let live = live.clone();
        let published = published.clone();
        let received = received.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let mut last_pub = 0u64;
            let mut last_rcv = 0u64;
            loop {
                tick.tick().await;
                let l = live.load(Ordering::Relaxed);
                let p = published.load(Ordering::Relaxed);
                let r = received.load(Ordering::Relaxed);
                println!(
                    "live={l:>6} | pub/s={:>7} | recv/s={:>9} | recv/bot/s={:>5}",
                    p.saturating_sub(last_pub),
                    r.saturating_sub(last_rcv),
                    if l > 0 { (r.saturating_sub(last_rcv)) / l } else { 0 }
                );
                last_pub = p;
                last_rcv = r;
            }
        });
    }

    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let url = url.clone();
        let live = live.clone();
        let published = published.clone();
        let received = received.clone();
        let sem = sem.clone();
        let t0 = t0;
        let h = tokio::spawn(async move {
            let permit = sem.acquire_owned().await.unwrap();
            let ws = match connect_async(&url).await {
                Ok((ws, _)) => ws,
                Err(e) => {
                    if i < 5 {
                        eprintln!("bot {i} connect failed: {e}");
                    }
                    drop(permit);
                    return;
                }
            };
            drop(permit);
            live.fetch_add(1, Ordering::Relaxed);
            let (mut tx, mut rx) = ws.split();

            // reader: count inbound fanouts
            let received_r = received.clone();
            let reader = tokio::spawn(async move {
                while let Some(Ok(m)) = rx.next().await {
                    match m {
                        Message::Text(_) | Message::Binary(_) => {
                            received_r.fetch_add(1, Ordering::Relaxed);
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });

            // writer: publish synthetic galaxy position at PUB_HZ
            let period = Duration::from_secs_f32(1.0 / hz.max(0.1));
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // small phase offset so bots don't all fire on the same tick edge
            tokio::time::sleep(Duration::from_micros(
                ((i as u64).wrapping_mul(997)) % (period.as_micros() as u64).max(1),
            ))
            .await;

            let hue = ((i as u32).wrapping_mul(137)) % 360;
            loop {
                interval.tick().await;
                let t = t0.elapsed().as_secs_f32();
                let (x, y) = galaxy_xy(i, t);
                let msg = serde_json::json!({
                    "id": format!("b{i}"),
                    "type": "event",
                    "channel": "hive",
                    "payload": { "kind": "json", "data": {
                        "t": "pos",
                        "i": i,
                        "x": round3(x),
                        "y": round3(y),
                        "h": hue,
                    }},
                    "timestamp": 0
                })
                .to_string();
                if tx.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
                published.fetch_add(1, Ordering::Relaxed);
            }

            let _ = reader.await;
            live.fetch_sub(1, Ordering::Relaxed);
        });
        handles.push(h);
    }

    tokio::signal::ctrl_c().await?;
    println!("\nbye");
    Ok(())
}

/// Swirling-galaxy trajectory. Maps bot index + time to a 2D position in
/// roughly [-1, 1]². Bots at the same "arm" move together; the spiral
/// tightens over time.
fn galaxy_xy(i: usize, t: f32) -> (f32, f32) {
    let idx = i as f32;
    // two arms + some perpendicular wobble
    let arm = (idx * 2.399_963) % std::f32::consts::TAU; // golden angle
    let r = 0.30 + 0.45 * ((idx * 0.0131 + t * 0.35).sin() * 0.5 + 0.5);
    let th = arm + t * (0.18 + (idx * 0.000_017) as f32);
    let x = r * th.cos() + 0.07 * (t * 0.9 + idx * 0.021).sin();
    let y = r * th.sin() + 0.07 * (t * 0.7 + idx * 0.017).cos();
    (x, y)
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn parse_args() -> (usize, String, usize, f32) {
    let mut count = 500usize;
    let mut url = "ws://127.0.0.1:17742/ws".to_string();
    let mut concurrency = 256usize;
    let mut hz: f32 = 10.0;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--count" | "-c" => {
                count = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(count);
                i += 2;
            }
            "--url" | "-u" => {
                if let Some(v) = args.get(i + 1) {
                    url = v.clone();
                }
                i += 2;
            }
            "--concurrency" | "-k" => {
                concurrency = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(concurrency);
                i += 2;
            }
            "--hz" => {
                hz = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(hz);
                i += 2;
            }
            "--help" | "-h" => {
                println!("hive-bot --count N --url URL [--concurrency K] [--hz 10]");
                std::process::exit(0);
            }
            _ => i += 1,
        }
    }
    (count, url, concurrency, hz)
}
