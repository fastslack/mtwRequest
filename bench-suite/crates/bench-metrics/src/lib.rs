//! Metrics primitives for the bench-suite: latency histograms, throughput
//! counters, and a canonical `BenchResult` that every scenario emits as JSON.
//!
//! All timestamps are nanoseconds since the UNIX epoch so they can travel
//! inside message payloads across process/language boundaries.

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Wall-clock nanoseconds since UNIX epoch.
pub fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before UNIX epoch")
        .as_nanos()
}

/// Shared histogram for recording one-way (fanout) or round-trip (echo)
/// latencies in nanoseconds. Bound is 60s; anything slower saturates.
#[derive(Clone)]
pub struct LatencyRecorder {
    inner: Arc<Mutex<Histogram<u64>>>,
}

impl LatencyRecorder {
    pub fn new() -> Self {
        let h = Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3)
            .expect("histogram bounds valid");
        Self {
            inner: Arc::new(Mutex::new(h)),
        }
    }

    pub fn record_nanos(&self, nanos: u64) {
        if let Ok(mut h) = self.inner.lock() {
            let _ = h.saturating_record(nanos);
        }
    }

    pub fn record_duration(&self, d: Duration) {
        let ns = d.as_nanos().min(u64::MAX as u128) as u64;
        self.record_nanos(ns);
    }

    pub fn snapshot(&self) -> LatencySnapshot {
        let h = self.inner.lock().expect("lock");
        LatencySnapshot {
            count: h.len(),
            min_ns: if h.len() == 0 { 0 } else { h.min() },
            max_ns: h.max(),
            mean_ns: h.mean() as u64,
            p50_ns: h.value_at_quantile(0.50),
            p90_ns: h.value_at_quantile(0.90),
            p99_ns: h.value_at_quantile(0.99),
            p999_ns: h.value_at_quantile(0.999),
        }
    }
}

impl Default for LatencyRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencySnapshot {
    pub count: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub mean_ns: u64,
    pub p50_ns: u64,
    pub p90_ns: u64,
    pub p99_ns: u64,
    pub p999_ns: u64,
}

impl LatencySnapshot {
    pub fn p50_us(&self) -> f64 {
        self.p50_ns as f64 / 1000.0
    }
    pub fn p99_us(&self) -> f64 {
        self.p99_ns as f64 / 1000.0
    }
}

/// One full scenario run. Serialized as JSON to `results/<ts>/<system>-<scenario>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub system: String,
    pub scenario: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub params: serde_json::Value,
    pub latency: Option<LatencySnapshot>,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub throughput_msgs_per_sec: f64,
    pub errors: u64,
    pub notes: String,
}

impl BenchResult {
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("serialize")
    }
}
