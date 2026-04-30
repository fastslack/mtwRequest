//! Agent Rain — concurrent AI-style streaming demo.
//!
//! Spawns ~80 virtual "agents" on the server. Each one runs an endless loop:
//!   start a stream (MsgType::AgentTask)
//!     → emit 15–60 chunks (MsgType::Stream, ref_id = task id)
//!     → close stream (MsgType::StreamEnd)
//!     → short pause
//!     → start again
//!
//! Every broadcast goes to every viewer. The viewer renders each active
//! stream as a Matrix-rain column where tokens cascade down as they arrive.
//!
//! This showcases:
//!   * MsgType::Stream with ref_id correlation (the same primitive a real
//!     AI agent uses to send token-by-token output to UI clients)
//!   * hundreds of independent async stream producers multiplexing through
//!     one WS transport
//!
//! Run:
//!   BIND=0.0.0.0:17744 cargo run --release --jobs $(nproc) --bin rain-server
//!   xdg-open 'examples/rain_viewer.html?port=17744'

use mtw_protocol::{MsgType, MtwMessage, Payload, TransportEvent};
use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const N_AGENTS: usize = 80;

// Three "moods" keep the visual interesting — prose, code, data — each with
// its own vocabulary. Agents pick a mood at spawn and keep it for the run.
const PROSE: &[&str] = &[
    "the ", "model ", "predicts ", "that ", "with ", "high ", "confidence ",
    "the ", "next ", "token ", "will ", "be ", "consistent ", "with ",
    "previous ", "context\n", "meanwhile ", "attention ", "weights ",
    "converge ", "toward ", "a ", "stable ", "distribution ",
    "and ", "the ", "gradient ", "flow ", "appears ", "healthy\n",
    "however ", "a ", "subtle ", "divergence ", "remains ", "visible ",
    "between ", "layers ", "4 ", "and ", "7 ", "which ", "may ",
    "indicate ", "a ", "dying ", "ReLU\n",
];

const CODE: &[&str] = &[
    "fn ", "async ", "await ", "impl ", "pub ", "struct ", "trait ", "use ",
    "Result", "<", "T", ",", " E", ">", " { ", "Ok(", "())", ";\n",
    "match ", "value ", "{ ", "Some(", "v", ") => ", "v, ", "None => ",
    "return ", "Err(", "\"empty\"", "), }\n",
    "loop { ", "select!", "{ ", "msg = rx.recv() => ", "process(", "msg", "),",
    " _ = shutdown.recv() => break, ", "} }\n",
    "let mut ", "buf = Vec::with_capacity(", "256", ");\n",
    "#[tokio::main]\n", "#[derive(Debug, Clone)]\n",
];

const DATA: &[&str] = &[
    "0x", "3a", "4f", "9c", "e1", " ", "→ ", "[", "ok", "] ",
    "tok=", "1e3 ", "lat=", "8µs ", "p50=", "12 ", "p99=", "47 ",
    "batch=", "256 ", "chunks=", "41 ", "mem=", "7.2MB ", "rss=", "14M ",
    "rt=", "0.91 ", "drop=", "0 ", "| ", "shard=", "[0..7] ", "\n",
    "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", " ",
    "0.", "01 ", "0.", "94 ", "-0.", "15 ", "NaN ", "+inf ",
];

#[derive(Clone, Copy)]
enum Mood { Prose, Code, Data }

impl Mood {
    fn words(self) -> &'static [&'static str] {
        match self {
            Mood::Prose => PROSE,
            Mood::Code  => CODE,
            Mood::Data  => DATA,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Mood::Prose => "prose",
            Mood::Code  => "code",
            Mood::Data  => "data",
        }
    }
}

struct Rng(u64);
impl Rng {
    fn seeded(s: u64) -> Self { Self(s | 1) }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        self.0 = x; x
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 { lo + self.next() % (hi - lo) }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T { &xs[(self.next() as usize) % xs.len()] }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let bind: SocketAddr = std::env::var("BIND")
        .unwrap_or_else(|_| "0.0.0.0:17744".into())
        .parse()?;

    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut event_rx = transport.take_event_receiver().unwrap();
    transport.listen(bind).await?;
    let transport: Arc<WebSocketTransport> = Arc::new(transport);

    tracing::info!(%bind, n_agents = N_AGENTS, "agent rain server up");
    tracing::info!("open examples/rain_viewer.html?port={}", bind.port());

    let msgs_out = Arc::new(AtomicU64::new(0));
    let streams_started = Arc::new(AtomicU64::new(0));
    let streams_ended = Arc::new(AtomicU64::new(0));
    let tokens_emitted = Arc::new(AtomicU64::new(0));
    let started = Instant::now();

    // Spawn N agents.
    for i in 0..N_AGENTS {
        let transport = transport.clone();
        let msgs_out = msgs_out.clone();
        let streams_started = streams_started.clone();
        let streams_ended = streams_ended.clone();
        let tokens_emitted = tokens_emitted.clone();
        tokio::spawn(async move {
            run_agent(
                i as u32,
                transport,
                msgs_out,
                streams_started,
                streams_ended,
                tokens_emitted,
            )
            .await;
        });
    }

    // Metrics 1Hz.
    {
        let transport = transport.clone();
        let msgs_out = msgs_out.clone();
        let streams_started = streams_started.clone();
        let streams_ended = streams_ended.clone();
        let tokens_emitted = tokens_emitted.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let mut last_toks = 0u64;
            let mut last_starts = 0u64;
            let mut last_msgs = 0u64;
            loop {
                tick.tick().await;
                let toks = tokens_emitted.load(Ordering::Relaxed);
                let starts = streams_started.load(Ordering::Relaxed);
                let ends = streams_ended.load(Ordering::Relaxed);
                let msgs = msgs_out.load(Ordering::Relaxed);
                let conns = transport.connection_count() as u64;
                let m = MtwMessage::new(
                    MsgType::Event,
                    Payload::Json(serde_json::json!({
                        "t": "metrics",
                        "conns": conns,
                        "agents": N_AGENTS,
                        "active_streams": starts.saturating_sub(ends),
                        "streams_started": starts,
                        "tokens_per_sec": toks.saturating_sub(last_toks),
                        "starts_per_sec": starts.saturating_sub(last_starts),
                        "outbound_per_sec": msgs.saturating_sub(last_msgs),
                        "uptime_s": started.elapsed().as_secs(),
                    })),
                );
                last_toks = toks; last_starts = starts; last_msgs = msgs;
                let fanout = conns;
                if transport.broadcast(m).await.is_ok() {
                    msgs_out.fetch_add(fanout, Ordering::Relaxed);
                }
            }
        });
    }

    // Drain transport events (we don't need to do anything with them beyond
    // letting the connection registry see them).
    loop {
        tokio::select! {
            Some(_ev) = event_rx.recv() => {
                if let TransportEvent::Connected(_id, _) = &_ev {
                    tracing::debug!("viewer connected");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                transport.shutdown().await?;
                return Ok(());
            }
        }
    }
}

async fn run_agent(
    idx: u32,
    transport: Arc<WebSocketTransport>,
    msgs_out: Arc<AtomicU64>,
    streams_started: Arc<AtomicU64>,
    streams_ended: Arc<AtomicU64>,
    tokens_emitted: Arc<AtomicU64>,
) {
    let mut rng = Rng::seeded(
        (idx as u64).wrapping_mul(6364136223846793005)
            ^ std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
    );

    // stagger agent start so they don't all fire at frame 0
    tokio::time::sleep(Duration::from_millis(rng.range(0, 2500))).await;

    let agent_name = format!("agent-{idx:03}");
    let mood = match idx % 3 { 0 => Mood::Prose, 1 => Mood::Code, _ => Mood::Data };

    loop {
        // ── start stream ────────────────────────────────────────────
        let task = MtwMessage::agent_task(&agent_name, format!("{}-run", mood.label()))
            .with_metadata("mood", serde_json::Value::String(mood.label().into()))
            .with_metadata("agent_idx", serde_json::Value::from(idx));
        let task_id = task.id.clone();
        let conns = transport.connection_count() as u64;
        if transport.broadcast(task).await.is_ok() {
            msgs_out.fetch_add(conns, Ordering::Relaxed);
        }
        streams_started.fetch_add(1, Ordering::Relaxed);

        // ── emit chunks ─────────────────────────────────────────────
        let n_tokens = rng.range(12, 55);
        for _ in 0..n_tokens {
            let delay_ms = rng.range(28, 180);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            let tok = *rng.pick(mood.words());
            let chunk = MtwMessage::stream_chunk(&task_id, tok);
            let conns = transport.connection_count() as u64;
            if transport.broadcast(chunk).await.is_ok() {
                msgs_out.fetch_add(conns, Ordering::Relaxed);
            }
            tokens_emitted.fetch_add(1, Ordering::Relaxed);
        }

        // ── end stream ──────────────────────────────────────────────
        let end = MtwMessage::stream_end(&task_id);
        let conns = transport.connection_count() as u64;
        if transport.broadcast(end).await.is_ok() {
            msgs_out.fetch_add(conns, Ordering::Relaxed);
        }
        streams_ended.fetch_add(1, Ordering::Relaxed);

        // ── pause before next run ──────────────────────────────────
        tokio::time::sleep(Duration::from_millis(rng.range(400, 3500))).await;
    }
}
