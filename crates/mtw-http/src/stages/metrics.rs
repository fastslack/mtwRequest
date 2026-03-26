use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Collected metrics for a host or endpoint.
#[derive(Debug, Clone)]
pub struct EndpointStats {
    pub request_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub total_duration_ms: u64,
    pub min_duration_ms: u64,
    pub max_duration_ms: u64,
}

impl EndpointStats {
    pub fn avg_duration_ms(&self) -> f64 {
        if self.request_count == 0 {
            0.0
        } else {
            self.total_duration_ms as f64 / self.request_count as f64
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.request_count == 0 {
            0.0
        } else {
            self.success_count as f64 / self.request_count as f64
        }
    }
}

struct AtomicStats {
    request_count: AtomicU64,
    success_count: AtomicU64,
    failure_count: AtomicU64,
    total_duration_ms: AtomicU64,
    min_duration_ms: AtomicU64,
    max_duration_ms: AtomicU64,
}

impl AtomicStats {
    fn new() -> Self {
        Self {
            request_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
            total_duration_ms: AtomicU64::new(0),
            min_duration_ms: AtomicU64::new(u64::MAX),
            max_duration_ms: AtomicU64::new(0),
        }
    }

    fn record(&self, duration: Duration, success: bool) {
        let ms = duration.as_millis() as u64;
        self.request_count.fetch_add(1, Ordering::Relaxed);
        self.total_duration_ms.fetch_add(ms, Ordering::Relaxed);

        if success {
            self.success_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failure_count.fetch_add(1, Ordering::Relaxed);
        }

        // Update min
        self.min_duration_ms.fetch_min(ms, Ordering::Relaxed);
        // Update max
        self.max_duration_ms.fetch_max(ms, Ordering::Relaxed);
    }

    fn snapshot(&self) -> EndpointStats {
        let min = self.min_duration_ms.load(Ordering::Relaxed);
        EndpointStats {
            request_count: self.request_count.load(Ordering::Relaxed),
            success_count: self.success_count.load(Ordering::Relaxed),
            failure_count: self.failure_count.load(Ordering::Relaxed),
            total_duration_ms: self.total_duration_ms.load(Ordering::Relaxed),
            min_duration_ms: if min == u64::MAX { 0 } else { min },
            max_duration_ms: self.max_duration_ms.load(Ordering::Relaxed),
        }
    }
}

/// Shared metrics collector.
#[derive(Clone)]
pub struct MetricsCollector {
    stats: Arc<DashMap<String, Arc<AtomicStats>>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            stats: Arc::new(DashMap::new()),
        }
    }

    fn record(&self, key: &str, duration: Duration, success: bool) {
        let stats = self
            .stats
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(AtomicStats::new()))
            .clone();
        stats.record(duration, success);
    }

    /// Get stats for a specific endpoint/host.
    pub fn get_stats(&self, key: &str) -> Option<EndpointStats> {
        self.stats.get(key).map(|s| s.snapshot())
    }

    /// Get stats for all tracked endpoints.
    pub fn all_stats(&self) -> Vec<(String, EndpointStats)> {
        self.stats
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().snapshot()))
            .collect()
    }

    /// Get aggregate stats across all endpoints.
    pub fn total_stats(&self) -> EndpointStats {
        let mut total = EndpointStats {
            request_count: 0,
            success_count: 0,
            failure_count: 0,
            total_duration_ms: 0,
            min_duration_ms: u64::MAX,
            max_duration_ms: 0,
        };

        for entry in self.stats.iter() {
            let s = entry.value().snapshot();
            total.request_count += s.request_count;
            total.success_count += s.success_count;
            total.failure_count += s.failure_count;
            total.total_duration_ms += s.total_duration_ms;
            total.min_duration_ms = total.min_duration_ms.min(s.min_duration_ms);
            total.max_duration_ms = total.max_duration_ms.max(s.max_duration_ms);
        }

        if total.min_duration_ms == u64::MAX {
            total.min_duration_ms = 0;
        }

        total
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Pipeline stage that collects request/response metrics.
pub struct MetricsStage {
    collector: MetricsCollector,
}

impl MetricsStage {
    pub fn new(collector: MetricsCollector) -> Self {
        Self { collector }
    }

    /// Get a reference to the metrics collector.
    pub fn collector(&self) -> &MetricsCollector {
        &self.collector
    }

    fn endpoint_key(ctx: &PipelineContext) -> String {
        format!("{} {}", ctx.request.method, ctx.request.url)
    }
}

#[async_trait]
impl PipelineStage for MetricsStage {
    fn name(&self) -> &str {
        "metrics"
    }

    fn priority(&self) -> i32 {
        95
    }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let key = Self::endpoint_key(context);
        let success = response.is_success();
        let duration = response.timing.duration;

        self.collector.record(&key, duration, success);

        Ok(PipelineAction::Continue(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;
    use crate::response::ResponseTiming;
    use std::time::Instant;

    fn make_ctx() -> PipelineContext {
        PipelineContext::new(MtwRequest::get("http://example.com/api"))
    }

    fn resp_with_timing(status: u16, duration_ms: u64) -> MtwResponse {
        let mut resp = MtwResponse::new(status);
        resp.timing = ResponseTiming {
            started_at: Instant::now(),
            duration: Duration::from_millis(duration_ms),
            dns_time: None,
            connect_time: None,
        };
        resp
    }

    #[tokio::test]
    async fn test_records_metrics() {
        let collector = MetricsCollector::new();
        let stage = MetricsStage::new(collector.clone());
        let mut ctx = make_ctx();

        stage
            .process(resp_with_timing(200, 50), &mut ctx)
            .await
            .unwrap();
        stage
            .process(resp_with_timing(200, 100), &mut ctx)
            .await
            .unwrap();
        stage
            .process(resp_with_timing(500, 200), &mut ctx)
            .await
            .unwrap();

        let stats = collector
            .get_stats("GET http://example.com/api")
            .unwrap();
        assert_eq!(stats.request_count, 3);
        assert_eq!(stats.success_count, 2);
        assert_eq!(stats.failure_count, 1);
        assert_eq!(stats.min_duration_ms, 50);
        assert_eq!(stats.max_duration_ms, 200);
    }

    #[tokio::test]
    async fn test_success_rate() {
        let collector = MetricsCollector::new();
        let stage = MetricsStage::new(collector.clone());
        let mut ctx = make_ctx();

        for _ in 0..8 {
            stage
                .process(resp_with_timing(200, 10), &mut ctx)
                .await
                .unwrap();
        }
        for _ in 0..2 {
            stage
                .process(resp_with_timing(500, 10), &mut ctx)
                .await
                .unwrap();
        }

        let stats = collector
            .get_stats("GET http://example.com/api")
            .unwrap();
        assert!((stats.success_rate() - 0.8).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_avg_duration() {
        let collector = MetricsCollector::new();
        let stage = MetricsStage::new(collector.clone());
        let mut ctx = make_ctx();

        stage
            .process(resp_with_timing(200, 100), &mut ctx)
            .await
            .unwrap();
        stage
            .process(resp_with_timing(200, 200), &mut ctx)
            .await
            .unwrap();

        let stats = collector
            .get_stats("GET http://example.com/api")
            .unwrap();
        assert!((stats.avg_duration_ms() - 150.0).abs() < 0.001);
    }

    #[test]
    fn test_empty_collector() {
        let collector = MetricsCollector::new();
        assert!(collector.get_stats("nonexistent").is_none());
        let total = collector.total_stats();
        assert_eq!(total.request_count, 0);
    }
}
