use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Circuit breaker states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation, requests pass through.
    Closed,
    /// Circuit is tripped, requests are rejected.
    Open,
    /// Testing if the service has recovered.
    HalfOpen,
}

/// Configuration for the circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit.
    pub failure_threshold: u32,
    /// How long to wait before transitioning from Open to HalfOpen.
    pub recovery_timeout: Duration,
    /// Max requests allowed in HalfOpen state.
    pub half_open_max_requests: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            half_open_max_requests: 1,
        }
    }
}

struct CircuitData {
    state: CircuitState,
    failure_count: u32,
    last_failure_at: Option<Instant>,
    half_open_requests: u32,
}

impl CircuitData {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure_at: None,
            half_open_requests: 0,
        }
    }
}

/// Pipeline stage implementing the circuit breaker pattern per host.
pub struct CircuitBreakerStage {
    config: CircuitBreakerConfig,
    circuits: Arc<DashMap<String, Arc<RwLock<CircuitData>>>>,
}

impl CircuitBreakerStage {
    pub fn new() -> Self {
        Self {
            config: CircuitBreakerConfig::default(),
            circuits: Arc::new(DashMap::new()),
        }
    }

    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            circuits: Arc::new(DashMap::new()),
        }
    }

    fn extract_host(url: &str) -> String {
        url.split("//")
            .nth(1)
            .unwrap_or(url)
            .split('/')
            .next()
            .unwrap_or(url)
            .to_string()
    }

    fn get_circuit(&self, host: &str) -> Arc<RwLock<CircuitData>> {
        self.circuits
            .entry(host.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(CircuitData::new())))
            .clone()
    }

    /// Get the current state of the circuit for a given host (for testing/monitoring).
    pub async fn state_for(&self, host: &str) -> CircuitState {
        let circuit = self.get_circuit(host);
        let guard = circuit.read().await;
        let state = guard.state.clone();
        drop(guard);
        state
    }
}

impl Default for CircuitBreakerStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for CircuitBreakerStage {
    fn name(&self) -> &str {
        "circuit_breaker"
    }

    fn priority(&self) -> i32 {
        2
    }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let host = Self::extract_host(&context.request.url);
        let circuit = self.get_circuit(&host);
        let mut data = circuit.write().await;

        // Check if we should transition from Open to HalfOpen
        if data.state == CircuitState::Open {
            if let Some(last_failure) = data.last_failure_at {
                if last_failure.elapsed() >= self.config.recovery_timeout {
                    tracing::info!(host = %host, "circuit breaker transitioning to half-open");
                    data.state = CircuitState::HalfOpen;
                    data.half_open_requests = 0;
                } else {
                    return Ok(PipelineAction::Error(MtwError::Transport(format!(
                        "circuit breaker open for host: {}",
                        host
                    ))));
                }
            }
        }

        // In HalfOpen, limit the number of requests
        if data.state == CircuitState::HalfOpen {
            if data.half_open_requests >= self.config.half_open_max_requests {
                return Ok(PipelineAction::Error(MtwError::Transport(format!(
                    "circuit breaker half-open limit reached for host: {}",
                    host
                ))));
            }
            data.half_open_requests += 1;
        }

        // Process the response
        if response.is_server_error() {
            data.failure_count += 1;
            data.last_failure_at = Some(Instant::now());

            if data.failure_count >= self.config.failure_threshold {
                tracing::warn!(
                    host = %host,
                    failures = data.failure_count,
                    "circuit breaker opening"
                );
                data.state = CircuitState::Open;
            }
        } else {
            // Success: reset in closed state, close in half-open state
            if data.state == CircuitState::HalfOpen {
                tracing::info!(host = %host, "circuit breaker closing (recovery successful)");
                data.state = CircuitState::Closed;
            }
            data.failure_count = 0;
        }

        Ok(PipelineAction::Continue(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;

    fn make_ctx() -> PipelineContext {
        PipelineContext::new(MtwRequest::get("http://api.example.com/data"))
    }

    #[tokio::test]
    async fn test_closed_state_passes_through() {
        let stage = CircuitBreakerStage::new();
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
        assert_eq!(
            stage.state_for("api.example.com").await,
            CircuitState::Closed
        );
    }

    #[tokio::test]
    async fn test_failures_open_circuit() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
            half_open_max_requests: 1,
        };
        let stage = CircuitBreakerStage::with_config(config);
        let mut ctx = make_ctx();

        // 3 failures should open the circuit
        for _ in 0..3 {
            let resp = MtwResponse::new(500);
            stage.process(resp, &mut ctx).await.unwrap();
        }

        assert_eq!(
            stage.state_for("api.example.com").await,
            CircuitState::Open
        );

        // Next request should be rejected
        let resp = MtwResponse::new(200);
        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Error(_)));
    }

    #[tokio::test]
    async fn test_success_resets_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let stage = CircuitBreakerStage::with_config(config);
        let mut ctx = make_ctx();

        // 2 failures then a success
        for _ in 0..2 {
            let resp = MtwResponse::new(500);
            stage.process(resp, &mut ctx).await.unwrap();
        }
        let resp = MtwResponse::new(200);
        stage.process(resp, &mut ctx).await.unwrap();

        assert_eq!(
            stage.state_for("api.example.com").await,
            CircuitState::Closed
        );
    }

    #[tokio::test]
    async fn test_half_open_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            recovery_timeout: Duration::from_millis(10),
            half_open_max_requests: 1,
        };
        let stage = CircuitBreakerStage::with_config(config);
        let mut ctx = make_ctx();

        // Trip the breaker
        for _ in 0..2 {
            let resp = MtwResponse::new(500);
            stage.process(resp, &mut ctx).await.unwrap();
        }
        assert_eq!(
            stage.state_for("api.example.com").await,
            CircuitState::Open
        );

        // Wait for recovery timeout
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Next request should go through (transitions to HalfOpen then succeeds -> Closed)
        let resp = MtwResponse::new(200);
        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
        assert_eq!(
            stage.state_for("api.example.com").await,
            CircuitState::Closed
        );
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            CircuitBreakerStage::extract_host("https://api.example.com/path"),
            "api.example.com"
        );
        assert_eq!(
            CircuitBreakerStage::extract_host("http://localhost:8080/test"),
            "localhost:8080"
        );
    }
}
