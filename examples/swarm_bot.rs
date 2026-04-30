//! Orbital Swarm — headless WS bot swarm.
//!
//! Opens N WebSocket connections to the swarm server and passively receives
//! the pulse/metrics broadcast. Used to pump up the HUD connection count and
//! stress the fanout path.
//!
//! Run:
//!     cargo run --release --bin swarm-bot -- --count 10000
//!     cargo run --release --bin swarm-bot -- --count 50000 --url ws://127.0.0.1:7741/ws

use futures::StreamExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (count, url, connect_concurrency) = parse_args();
    println!(
        "orbital swarm bot · target {count} connections · url {url} · concurrency {connect_concurrency}"
    );

    let received = Arc::new(AtomicU64::new(0));
    let live = Arc::new(AtomicU64::new(0));
    let sem = Arc::new(Semaphore::new(connect_concurrency));

    // Stats reporter
    {
        let received = received.clone();
        let live = live.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let mut last = 0u64;
            loop {
                tick.tick().await;
                let now = received.load(Ordering::Relaxed);
                let conns = live.load(Ordering::Relaxed);
                println!(
                    "live={conns:>7} | msgs/s={:>9} | total_recv={now}",
                    now.saturating_sub(last)
                );
                last = now;
            }
        });
    }

    // Spawn connection tasks — the semaphore caps in-flight handshakes so we
    // don't SYN-flood ourselves on the way to 50k.
    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let url = url.clone();
        let received = received.clone();
        let live = live.clone();
        let sem = sem.clone();
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
            let (_tx, mut rx) = ws.split();
            while let Some(msg) = rx.next().await {
                match msg {
                    Ok(Message::Text(_)) | Ok(Message::Binary(_)) => {
                        received.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
            live.fetch_sub(1, Ordering::Relaxed);
        });
        handles.push(h);
    }

    // Wait forever — ctrl-c ends the party.
    tokio::signal::ctrl_c().await?;
    println!("\nbye");
    Ok(())
}

fn parse_args() -> (usize, String, usize) {
    let mut count = 1000usize;
    let mut url = "ws://127.0.0.1:7741/ws".to_string();
    let mut concurrency = 256usize;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--count" | "-c" => {
                count = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(count);
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
            "--help" | "-h" => {
                println!(
                    "swarm-bot --count N --url ws://host:port/ws [--concurrency 256]"
                );
                std::process::exit(0);
            }
            _ => i += 1,
        }
    }
    (count, url, concurrency)
}
