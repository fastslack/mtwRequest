//! Bench orchestrator. Spawns subscribers + publisher against a target system
//! (mtw/centrifugo/socketio/nats), runs one of three scenarios, and writes a
//! canonical `BenchResult` JSON under `results/<timestamp>/`.
//!
//! Payload layout: first 16 bytes = publisher's `now_nanos()` as little-endian
//! u128. Remaining bytes are zero-padded to `--payload-bytes`. Subscribers
//! decode the header to compute one-way latency.

use anyhow::{Context, Result};
use bench_clients::System;
use bench_metrics::{now_nanos, BenchResult, LatencyRecorder};
use bytes::Bytes;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const TS_HEADER_LEN: usize = 16;

#[derive(Parser)]
#[command(
    name = "bench-runner",
    about = "Pub/sub benchmark: mtwRequest vs Centrifugo vs Socket.IO vs NATS"
)]
struct Cli {
    /// Target system: mtw | centrifugo | socketio | nats
    #[arg(long)]
    system: String,

    /// Connection URL (defaults to the system's loopback default)
    #[arg(long)]
    url: Option<String>,

    /// Output directory for results JSON
    #[arg(long, default_value = "./results")]
    out_dir: PathBuf,

    /// Filename tag (defaults to scenario name)
    #[arg(long)]
    tag: Option<String>,

    #[command(subcommand)]
    scenario: Scenario,
}

#[derive(Subcommand, Clone)]
enum Scenario {
    /// 1 publisher → N subscribers. Measures one-way fanout latency + throughput.
    Fanout {
        #[arg(long, default_value_t = 1000)]
        subs: usize,
        #[arg(long, default_value_t = 10_000)]
        messages: u64,
        #[arg(long, default_value_t = 128)]
        payload_bytes: usize,
        #[arg(long, default_value_t = 200)]
        warmup: u64,
        #[arg(long, default_value_t = 60)]
        timeout_secs: u64,
        #[arg(long, default_value = "bench")]
        channel: String,
    },
    /// Round-trip latency via an echo-bot (publisher → A → bot → B → publisher).
    Echo {
        #[arg(long, default_value_t = 5_000)]
        messages: u64,
        #[arg(long, default_value_t = 128)]
        payload_bytes: usize,
        #[arg(long, default_value_t = 100)]
        warmup: u64,
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
    /// Connection storm: open N connections with limited concurrency.
    Connect {
        #[arg(long, default_value_t = 1_000)]
        count: usize,
        #[arg(long, default_value_t = 64)]
        concurrency: usize,
        #[arg(long, default_value_t = 60)]
        timeout_secs: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();

    let cli = Cli::parse();
    let system = System::parse(&cli.system)?;
    let url = cli
        .url
        .clone()
        .unwrap_or_else(|| system.default_url().to_string());

    let result = match cli.scenario.clone() {
        Scenario::Fanout {
            subs,
            messages,
            payload_bytes,
            warmup,
            timeout_secs,
            channel,
        } => {
            run_fanout(
                system,
                &url,
                subs,
                messages,
                payload_bytes,
                warmup,
                timeout_secs,
                &channel,
            )
            .await?
        }
        Scenario::Echo {
            messages,
            payload_bytes,
            warmup,
            timeout_secs,
        } => run_echo(system, &url, messages, payload_bytes, warmup, timeout_secs).await?,
        Scenario::Connect {
            count,
            concurrency,
            timeout_secs,
        } => run_connect(system, &url, count, concurrency, timeout_secs).await?,
    };

    let tag = cli
        .tag
        .unwrap_or_else(|| result.scenario.clone());
    std::fs::create_dir_all(&cli.out_dir).context("create out_dir")?;
    let path = cli
        .out_dir
        .join(format!("{}-{}.json", system.as_str(), tag));
    std::fs::write(&path, result.to_json_pretty())?;
    eprintln!("wrote {}", path.display());
    println!("{}", result.to_json_pretty());
    Ok(())
}

// -----------------------------------------------------------------------------
// Fanout
// -----------------------------------------------------------------------------

async fn run_fanout(
    system: System,
    url: &str,
    subs: usize,
    messages: u64,
    payload_bytes: usize,
    warmup: u64,
    timeout_secs: u64,
    channel: &str,
) -> Result<BenchResult> {
    let latency = LatencyRecorder::new();
    let total_received = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let total_expected_per_sub = warmup + messages;

    // Spawn subscribers.
    let mut sub_handles = Vec::with_capacity(subs);
    for _ in 0..subs {
        let url = url.to_string();
        let channel = channel.to_string();
        let latency = latency.clone();
        let total_received = total_received.clone();
        let errors = errors.clone();
        let handle = tokio::spawn(async move {
            let mut client = system.new_client();
            if client.connect(&url).await.is_err() {
                errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
            if client.subscribe(&channel).await.is_err() {
                errors.fetch_add(1, Ordering::Relaxed);
                let _ = client.close().await;
                return;
            }
            let mut seen: u64 = 0;
            while seen < total_expected_per_sub {
                match tokio::time::timeout(Duration::from_secs(timeout_secs), client.recv()).await
                {
                    Ok(Ok(msg)) => {
                        if seen >= warmup && msg.payload.len() >= TS_HEADER_LEN {
                            let mut buf = [0u8; TS_HEADER_LEN];
                            buf.copy_from_slice(&msg.payload[..TS_HEADER_LEN]);
                            let sent_ns = u128::from_le_bytes(buf);
                            let now = now_nanos();
                            if now > sent_ns {
                                latency
                                    .record_nanos((now - sent_ns).min(u64::MAX as u128) as u64);
                            }
                            total_received.fetch_add(1, Ordering::Relaxed);
                        }
                        seen += 1;
                    }
                    _ => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                }
            }
            let _ = client.close().await;
        });
        sub_handles.push(handle);
    }

    // Wait for subscribers to be connected. Scale with subscriber count — NATS
    // can open thousands of subs quickly, WebSocket-based systems need longer.
    let warmup_ms = 500 + (subs as u64 * 3).min(5_000);
    tokio::time::sleep(Duration::from_millis(warmup_ms)).await;

    // Publisher.
    let mut publisher = system.new_client();
    publisher.connect(url).await.context("publisher connect")?;

    let started_at = chrono::Utc::now();
    let started = Instant::now();

    // Warmup messages (subscribers skip these in their latency histogram).
    for _ in 0..warmup {
        let _ = publisher
            .publish(channel, make_payload(payload_bytes))
            .await;
    }

    // Settle briefly so the warmup fully drains before the timing window.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let timed_start = Instant::now();

    for _ in 0..messages {
        if publisher
            .publish(channel, make_payload(payload_bytes))
            .await
            .is_err()
        {
            errors.fetch_add(1, Ordering::Relaxed);
        }
    }
    let publish_elapsed = timed_start.elapsed();

    // Wait for subscribers.
    for h in sub_handles {
        let _ = h.await;
    }
    let elapsed = started.elapsed();
    let _ = publisher.close().await;

    let received = total_received.load(Ordering::Relaxed);
    let expected = subs as u64 * messages;
    let duration_secs = publish_elapsed.as_secs_f64().max(1e-9);
    let throughput = received as f64 / duration_secs;

    Ok(BenchResult {
        system: system.to_string(),
        scenario: "fanout".to_string(),
        started_at,
        duration_ms: elapsed.as_millis() as u64,
        params: serde_json::json!({
            "subs": subs,
            "messages": messages,
            "payload_bytes": payload_bytes,
            "warmup": warmup,
            "channel": channel,
            "publish_elapsed_ms": publish_elapsed.as_millis(),
        }),
        latency: Some(latency.snapshot()),
        messages_sent: messages,
        messages_received: received,
        throughput_msgs_per_sec: throughput,
        errors: errors.load(Ordering::Relaxed),
        notes: format!(
            "expected {} deliveries, got {} ({:.2}%)",
            expected,
            received,
            100.0 * received as f64 / expected.max(1) as f64
        ),
    })
}

// -----------------------------------------------------------------------------
// Echo (round-trip via echo-bot)
// -----------------------------------------------------------------------------

async fn run_echo(
    system: System,
    url: &str,
    messages: u64,
    payload_bytes: usize,
    warmup: u64,
    timeout_secs: u64,
) -> Result<BenchResult> {
    let ping = format!("bench-ping-{}", std::process::id());
    let pong = format!("bench-pong-{}", std::process::id());

    let latency = LatencyRecorder::new();
    let errors = Arc::new(AtomicU64::new(0));

    // Echo-bot: subscribes to "ping", republishes each message to "pong".
    let bot_ping = ping.clone();
    let bot_pong = pong.clone();
    let bot_url = url.to_string();
    let bot_errors = errors.clone();
    let bot = tokio::spawn(async move {
        let mut client = system.new_client();
        if client.connect(&bot_url).await.is_err() {
            bot_errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if client.subscribe(&bot_ping).await.is_err() {
            bot_errors.fetch_add(1, Ordering::Relaxed);
            let _ = client.close().await;
            return;
        }
        let total = warmup + messages;
        for _ in 0..total {
            match tokio::time::timeout(Duration::from_secs(timeout_secs), client.recv()).await {
                Ok(Ok(msg)) => {
                    // Echo verbatim — the header timestamp is preserved.
                    let _ = client.publish(&bot_pong, msg.payload).await;
                }
                _ => {
                    bot_errors.fetch_add(1, Ordering::Relaxed);
                    break;
                }
            }
        }
        let _ = client.close().await;
    });

    // Give the bot a moment to subscribe.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Requester: subscribe to pong, publish to ping, measure RTT.
    let mut requester = system.new_client();
    requester.connect(url).await.context("requester connect")?;
    requester
        .subscribe(&pong)
        .await
        .context("requester subscribe pong")?;

    // Give subscription time to register server-side.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let started_at = chrono::Utc::now();
    let started = Instant::now();

    // Warmup
    for _ in 0..warmup {
        let _ = requester.publish(&ping, make_payload(payload_bytes)).await;
        let _ = tokio::time::timeout(Duration::from_secs(timeout_secs), requester.recv()).await;
    }

    let timed_start = Instant::now();
    let mut received: u64 = 0;
    for _ in 0..messages {
        let payload = make_payload(payload_bytes);
        let sent_ns = read_ts_header(&payload);
        if requester.publish(&ping, payload).await.is_err() {
            errors.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        match tokio::time::timeout(Duration::from_secs(timeout_secs), requester.recv()).await {
            Ok(Ok(_)) => {
                let now = now_nanos();
                if now > sent_ns {
                    latency.record_nanos((now - sent_ns).min(u64::MAX as u128) as u64);
                }
                received += 1;
            }
            _ => {
                errors.fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
    }
    let rtt_elapsed = timed_start.elapsed();

    let _ = requester.close().await;
    let _ = bot.await;
    let elapsed = started.elapsed();

    let throughput = received as f64 / rtt_elapsed.as_secs_f64().max(1e-9);

    Ok(BenchResult {
        system: system.to_string(),
        scenario: "echo".to_string(),
        started_at,
        duration_ms: elapsed.as_millis() as u64,
        params: serde_json::json!({
            "messages": messages,
            "payload_bytes": payload_bytes,
            "warmup": warmup,
            "ping_channel": ping,
            "pong_channel": pong,
        }),
        latency: Some(latency.snapshot()),
        messages_sent: messages,
        messages_received: received,
        throughput_msgs_per_sec: throughput,
        errors: errors.load(Ordering::Relaxed),
        notes: format!(
            "RTT latency (publisher → bot → publisher); {} round-trips completed",
            received
        ),
    })
}

// -----------------------------------------------------------------------------
// Connect storm
// -----------------------------------------------------------------------------

async fn run_connect(
    system: System,
    url: &str,
    count: usize,
    concurrency: usize,
    timeout_secs: u64,
) -> Result<BenchResult> {
    let latency = LatencyRecorder::new();
    let errors = Arc::new(AtomicU64::new(0));
    let succeeded = Arc::new(AtomicU64::new(0));
    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));

    let started_at = chrono::Utc::now();
    let started = Instant::now();

    let mut handles = Vec::with_capacity(count);
    for _ in 0..count {
        let sem = semaphore.clone();
        let url = url.to_string();
        let latency = latency.clone();
        let errors = errors.clone();
        let succeeded = succeeded.clone();
        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore");
            let t0 = Instant::now();
            let mut client = system.new_client();
            let connect_result =
                tokio::time::timeout(Duration::from_secs(timeout_secs), client.connect(&url)).await;
            match connect_result {
                Ok(Ok(())) => {
                    latency.record_duration(t0.elapsed());
                    succeeded.fetch_add(1, Ordering::Relaxed);
                    let _ = client.close().await;
                }
                _ => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }
    let elapsed = started.elapsed();
    let ok = succeeded.load(Ordering::Relaxed);
    let connects_per_sec = ok as f64 / elapsed.as_secs_f64().max(1e-9);

    Ok(BenchResult {
        system: system.to_string(),
        scenario: "connect".to_string(),
        started_at,
        duration_ms: elapsed.as_millis() as u64,
        params: serde_json::json!({
            "count": count,
            "concurrency": concurrency,
        }),
        latency: Some(latency.snapshot()),
        messages_sent: count as u64,
        messages_received: ok,
        throughput_msgs_per_sec: connects_per_sec,
        errors: errors.load(Ordering::Relaxed),
        notes: format!(
            "opened {}/{} connections, {:.1} connects/s",
            ok, count, connects_per_sec
        ),
    })
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

fn make_payload(size: usize) -> Bytes {
    let now = now_nanos();
    let mut buf = vec![0u8; size.max(TS_HEADER_LEN)];
    buf[..TS_HEADER_LEN].copy_from_slice(&now.to_le_bytes());
    Bytes::from(buf)
}

fn read_ts_header(payload: &[u8]) -> u128 {
    if payload.len() < TS_HEADER_LEN {
        return 0;
    }
    let mut buf = [0u8; TS_HEADER_LEN];
    buf.copy_from_slice(&payload[..TS_HEADER_LEN]);
    u128::from_le_bytes(buf)
}
