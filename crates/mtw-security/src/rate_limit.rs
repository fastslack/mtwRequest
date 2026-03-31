use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Configuration for the rate limiter
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_requests: u32,
    pub window_duration: Duration,
    pub block_duration: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 1000,
            window_duration: Duration::from_secs(60),
            block_duration: Duration::from_secs(300),
        }
    }
}

/// State for a single rate limit entry
#[derive(Debug, Clone)]
struct RateLimitEntry {
    count: u32,
    window_start: Instant,
    blocked: bool,
    blocked_until: Option<Instant>,
}

/// Rate limit status for a key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitStatus {
    pub key: String,
    pub count: u32,
    pub limit: u32,
    pub blocked: bool,
    pub remaining: u32,
}

/// Token bucket rate limiter using DashMap for concurrent access
pub struct RateLimiter {
    entries: DashMap<String, RateLimitEntry>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            entries: DashMap::new(),
            config,
        }
    }

    /// Check if a key is currently allowed (not blocked, not over limit)
    pub fn check(&self, key: &str) -> bool {
        if let Some(entry) = self.entries.get(key) {
            if entry.blocked {
                if let Some(until) = entry.blocked_until {
                    if Instant::now() < until {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            let elapsed = entry.window_start.elapsed();
            if elapsed < self.config.window_duration {
                return entry.count < self.config.max_requests;
            }
        }
        true
    }

    /// Record a request and consume a token. Returns error if rate limited.
    pub fn consume(&self, key: &str) -> Result<(), MtwError> {
        let now = Instant::now();

        let mut entry = self.entries.entry(key.to_string()).or_insert_with(|| {
            RateLimitEntry {
                count: 0,
                window_start: now,
                blocked: false,
                blocked_until: None,
            }
        });

        // Unblock if block duration expired
        if entry.blocked {
            if let Some(until) = entry.blocked_until {
                if now >= until {
                    entry.blocked = false;
                    entry.blocked_until = None;
                    entry.count = 0;
                    entry.window_start = now;
                } else {
                    return Err(MtwError::Auth(format!("rate limited: {}", key)));
                }
            }
        }

        // Reset window if expired
        if entry.window_start.elapsed() >= self.config.window_duration {
            entry.count = 0;
            entry.window_start = now;
        }

        entry.count += 1;

        // Block if over limit
        if entry.count > self.config.max_requests {
            entry.blocked = true;
            entry.blocked_until = Some(now + self.config.block_duration);
            return Err(MtwError::Auth(format!("rate limited: {}", key)));
        }

        Ok(())
    }

    /// Reset a key's rate limit state
    pub fn reset(&self, key: &str) {
        self.entries.remove(key);
    }

    /// Manually unblock a key
    pub fn unblock(&self, key: &str) {
        if let Some(mut entry) = self.entries.get_mut(key) {
            entry.blocked = false;
            entry.blocked_until = None;
            entry.count = 0;
            entry.window_start = Instant::now();
        }
    }

    /// Get status for a key
    pub fn get_status(&self, key: &str) -> RateLimitStatus {
        if let Some(entry) = self.entries.get(key) {
            let count = if entry.window_start.elapsed() >= self.config.window_duration {
                0
            } else {
                entry.count
            };
            RateLimitStatus {
                key: key.to_string(),
                count,
                limit: self.config.max_requests,
                blocked: entry.blocked
                    && entry
                        .blocked_until
                        .map(|u| Instant::now() < u)
                        .unwrap_or(true),
                remaining: self.config.max_requests.saturating_sub(count),
            }
        } else {
            RateLimitStatus {
                key: key.to_string(),
                count: 0,
                limit: self.config.max_requests,
                blocked: false,
                remaining: self.config.max_requests,
            }
        }
    }

    /// Get all currently blocked keys
    pub fn get_blocked_keys(&self) -> Vec<String> {
        let now = Instant::now();
        self.entries
            .iter()
            .filter(|e| {
                e.blocked && e.blocked_until.map(|u| now < u).unwrap_or(true)
            })
            .map(|e| e.key().clone())
            .collect()
    }

    /// Remove expired entries
    pub fn cleanup_expired(&self) {
        let now = Instant::now();
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|e| {
                let window_expired = e.window_start.elapsed() >= self.config.window_duration * 2;
                let block_expired = e
                    .blocked_until
                    .map(|u| now >= u)
                    .unwrap_or(true);
                window_expired && block_expired && !e.blocked
            })
            .map(|e| e.key().clone())
            .collect();

        for key in expired {
            self.entries.remove(&key);
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(RateLimitConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_rate_limiting() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 3,
            window_duration: Duration::from_secs(60),
            block_duration: Duration::from_secs(5),
        });

        assert!(limiter.consume("user1").is_ok());
        assert!(limiter.consume("user1").is_ok());
        assert!(limiter.consume("user1").is_ok());
        assert!(limiter.consume("user1").is_err()); // 4th should fail
    }

    #[test]
    fn test_check() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 2,
            window_duration: Duration::from_secs(60),
            block_duration: Duration::from_secs(5),
        });

        assert!(limiter.check("user1"));
        limiter.consume("user1").unwrap();
        limiter.consume("user1").unwrap();
        assert!(!limiter.check("user1"));
    }

    #[test]
    fn test_reset() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 1,
            ..Default::default()
        });

        limiter.consume("user1").unwrap();
        assert!(limiter.consume("user1").is_err());

        limiter.reset("user1");
        assert!(limiter.consume("user1").is_ok());
    }

    #[test]
    fn test_unblock() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 1,
            block_duration: Duration::from_secs(300),
            ..Default::default()
        });

        limiter.consume("user1").unwrap();
        let _ = limiter.consume("user1"); // triggers block

        assert!(!limiter.check("user1"));
        limiter.unblock("user1");
        assert!(limiter.check("user1"));
    }

    #[test]
    fn test_status() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 10,
            ..Default::default()
        });

        let status = limiter.get_status("user1");
        assert_eq!(status.count, 0);
        assert_eq!(status.remaining, 10);
        assert!(!status.blocked);

        limiter.consume("user1").unwrap();
        let status = limiter.get_status("user1");
        assert_eq!(status.count, 1);
        assert_eq!(status.remaining, 9);
    }

    #[test]
    fn test_independent_keys() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 1,
            ..Default::default()
        });

        limiter.consume("user1").unwrap();
        assert!(limiter.consume("user2").is_ok()); // separate key
    }
}
