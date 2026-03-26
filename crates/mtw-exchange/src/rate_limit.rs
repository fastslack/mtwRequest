use async_trait::async_trait;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Rate limiter trait — `acquire()` waits until a request can proceed.
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Acquire permission to make one request. Blocks until allowed.
    /// Returns the number of milliseconds waited (0 if immediate).
    async fn acquire(&self) -> u64;

    /// Current remaining capacity (approximate, non-blocking).
    fn remaining(&self) -> usize;
}

/// Token bucket rate limiter.
///
/// Tokens refill at a constant rate. Each request consumes one token.
/// Use for APIs with per-second limits (e.g., IBKR: 10 req/s).
pub struct TokenBucket {
    inner: Mutex<TokenBucketInner>,
}

struct TokenBucketInner {
    tokens: f64,
    max_tokens: f64,
    refill_interval: Duration,
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new token bucket.
    ///
    /// - `max_tokens`: burst capacity (e.g., 10)
    /// - `refill_interval`: duration to refill all tokens (e.g., 1 second)
    pub fn new(max_tokens: u32, refill_interval: Duration) -> Self {
        Self {
            inner: Mutex::new(TokenBucketInner {
                tokens: max_tokens as f64,
                max_tokens: max_tokens as f64,
                refill_interval,
                last_refill: Instant::now(),
            }),
        }
    }
}

#[async_trait]
impl RateLimiter for TokenBucket {
    async fn acquire(&self) -> u64 {
        let mut total_waited: u64 = 0;
        loop {
            let wait_ms = {
                let mut inner = self.inner.lock().await;
                let now = Instant::now();
                let elapsed = now.duration_since(inner.last_refill);
                let tokens_to_add = (elapsed.as_secs_f64()
                    / inner.refill_interval.as_secs_f64())
                    * inner.max_tokens;
                inner.tokens = (inner.tokens + tokens_to_add).min(inner.max_tokens);
                inner.last_refill = now;

                if inner.tokens >= 1.0 {
                    inner.tokens -= 1.0;
                    return total_waited;
                }

                let tokens_per_ms =
                    inner.max_tokens / inner.refill_interval.as_millis() as f64;
                ((1.0 - inner.tokens) / tokens_per_ms).ceil() as u64
            };

            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            total_waited += wait_ms;
        }
    }

    fn remaining(&self) -> usize {
        self.inner
            .try_lock()
            .map(|inner| inner.tokens as usize)
            .unwrap_or(0)
    }
}

/// Sliding window rate limiter.
///
/// Tracks timestamps of recent requests within a rolling window.
/// Use for APIs with per-minute limits (e.g., Saxo: 120 req/min).
pub struct SlidingWindowLimiter {
    inner: Mutex<SlidingWindowInner>,
}

struct SlidingWindowInner {
    timestamps: VecDeque<Instant>,
    max_requests: usize,
    window: Duration,
}

impl SlidingWindowLimiter {
    /// Create a new sliding window limiter.
    ///
    /// - `max_requests`: maximum requests within the window (e.g., 120)
    /// - `window`: rolling window duration (e.g., 60 seconds)
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            inner: Mutex::new(SlidingWindowInner {
                timestamps: VecDeque::new(),
                max_requests,
                window,
            }),
        }
    }
}

#[async_trait]
impl RateLimiter for SlidingWindowLimiter {
    async fn acquire(&self) -> u64 {
        let mut total_waited: u64 = 0;
        loop {
            let wait_ms = {
                let mut inner = self.inner.lock().await;
                let now = Instant::now();
                let cutoff = now - inner.window;

                // Prune expired timestamps
                while inner
                    .timestamps
                    .front()
                    .map_or(false, |&t| t <= cutoff)
                {
                    inner.timestamps.pop_front();
                }

                if inner.timestamps.len() < inner.max_requests {
                    inner.timestamps.push_back(now);
                    return total_waited;
                }

                // Window is full — wait until oldest entry expires
                let oldest = inner.timestamps[0];
                let expires_at = oldest + inner.window;
                expires_at.duration_since(now).as_millis() as u64 + 1
            };

            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            total_waited += wait_ms;
        }
    }

    fn remaining(&self) -> usize {
        self.inner
            .try_lock()
            .map(|inner| {
                let now = Instant::now();
                let cutoff = now - inner.window;
                let active = inner
                    .timestamps
                    .iter()
                    .filter(|&&t| t > cutoff)
                    .count();
                inner.max_requests.saturating_sub(active)
            })
            .unwrap_or(0)
    }
}

/// Create a rate limiter from config
pub fn from_config(config: &crate::config::RateLimitConfig) -> Box<dyn RateLimiter> {
    match config.strategy.as_str() {
        "sliding_window" => Box::new(SlidingWindowLimiter::new(
            config.max_requests as usize,
            Duration::from_millis(config.window_ms),
        )),
        _ => Box::new(TokenBucket::new(
            config.max_requests,
            Duration::from_millis(config.window_ms),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_bucket_immediate() {
        let bucket = TokenBucket::new(10, Duration::from_secs(1));
        // First 10 requests should be immediate
        for _ in 0..10 {
            let waited = bucket.acquire().await;
            assert_eq!(waited, 0);
        }
    }

    #[tokio::test]
    async fn test_token_bucket_remaining() {
        let bucket = TokenBucket::new(5, Duration::from_secs(1));
        assert_eq!(bucket.remaining(), 5);
        bucket.acquire().await;
        assert_eq!(bucket.remaining(), 4);
        bucket.acquire().await;
        assert_eq!(bucket.remaining(), 3);
    }

    #[tokio::test]
    async fn test_token_bucket_refill() {
        let bucket = TokenBucket::new(2, Duration::from_millis(100));
        // Exhaust tokens
        bucket.acquire().await;
        bucket.acquire().await;
        assert_eq!(bucket.remaining(), 0);

        // Wait for refill
        tokio::time::sleep(Duration::from_millis(120)).await;
        let waited = bucket.acquire().await;
        assert_eq!(waited, 0);
    }

    #[tokio::test]
    async fn test_sliding_window_immediate() {
        let limiter = SlidingWindowLimiter::new(5, Duration::from_secs(1));
        for _ in 0..5 {
            let waited = limiter.acquire().await;
            assert_eq!(waited, 0);
        }
    }

    #[tokio::test]
    async fn test_sliding_window_remaining() {
        let limiter = SlidingWindowLimiter::new(3, Duration::from_secs(1));
        assert_eq!(limiter.remaining(), 3);
        limiter.acquire().await;
        assert_eq!(limiter.remaining(), 2);
    }

    #[tokio::test]
    async fn test_sliding_window_expiry() {
        let limiter = SlidingWindowLimiter::new(2, Duration::from_millis(100));
        limiter.acquire().await;
        limiter.acquire().await;
        assert_eq!(limiter.remaining(), 0);

        // Wait for window to expire
        tokio::time::sleep(Duration::from_millis(120)).await;
        let waited = limiter.acquire().await;
        assert_eq!(waited, 0);
    }

    #[test]
    fn test_from_config_token_bucket() {
        let config = crate::config::RateLimitConfig {
            strategy: "token_bucket".into(),
            max_requests: 10,
            window_ms: 1000,
        };
        let limiter = from_config(&config);
        assert_eq!(limiter.remaining(), 10);
    }

    #[test]
    fn test_from_config_sliding_window() {
        let config = crate::config::RateLimitConfig {
            strategy: "sliding_window".into(),
            max_requests: 120,
            window_ms: 60000,
        };
        let limiter = from_config(&config);
        assert_eq!(limiter.remaining(), 120);
    }
}
