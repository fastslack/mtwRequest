use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::{CacheInfo, MtwResponse};

/// Configuration for the cache stage.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of cached entries.
    pub max_entries: usize,
    /// Default TTL for cached responses.
    pub default_ttl: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            default_ttl: Duration::from_secs(300),
        }
    }
}

#[derive(Clone)]
struct CacheEntry {
    response: MtwResponse,
    etag: Option<String>,
    last_modified: Option<String>,
    expires_at: Instant,
}

/// Pipeline stage providing in-memory response caching with ETag/Last-Modified support.
pub struct CacheStage {
    config: CacheConfig,
    cache: Arc<DashMap<String, CacheEntry>>,
}

impl CacheStage {
    pub fn new() -> Self {
        Self {
            config: CacheConfig::default(),
            cache: Arc::new(DashMap::new()),
        }
    }

    pub fn with_config(config: CacheConfig) -> Self {
        Self {
            config,
            cache: Arc::new(DashMap::new()),
        }
    }

    fn cache_key(ctx: &PipelineContext) -> String {
        format!("{}:{}", ctx.request.method, ctx.request.url)
    }

    fn parse_max_age(cache_control: &str) -> Option<u64> {
        for directive in cache_control.split(',') {
            let directive = directive.trim();
            if let Some(value) = directive.strip_prefix("max-age=") {
                return value.trim().parse().ok();
            }
        }
        None
    }

    fn should_not_cache(headers: &std::collections::HashMap<String, String>) -> bool {
        if let Some(cc) = headers.get("cache-control") {
            let cc = cc.to_lowercase();
            if cc.contains("no-store") || cc.contains("no-cache") {
                return true;
            }
        }
        false
    }

    /// Get the number of entries currently cached (for testing/metrics).
    pub fn entry_count(&self) -> usize {
        self.cache.len()
    }

    /// Clear all cache entries.
    pub fn clear(&self) {
        self.cache.clear();
    }
}

impl Default for CacheStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for CacheStage {
    fn name(&self) -> &str {
        "cache"
    }

    fn priority(&self) -> i32 {
        30
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let key = Self::cache_key(context);

        // On 304 Not Modified, return the cached response
        if response.status == 304 {
            if let Some(entry) = self.cache.get(&key) {
                let mut cached = entry.response.clone();
                cached.cache_info = Some(CacheInfo {
                    etag: entry.etag.clone(),
                    last_modified: entry.last_modified.clone(),
                    cache_control: response.headers.get("cache-control").cloned(),
                    max_age: response
                        .headers
                        .get("cache-control")
                        .and_then(|cc| Self::parse_max_age(cc)),
                    is_cached: true,
                });
                return Ok(PipelineAction::Cached(cached));
            }
        }

        // Extract cache-related headers
        let etag = response.headers.get("etag").cloned();
        let last_modified = response.headers.get("last-modified").cloned();
        let cache_control = response.headers.get("cache-control").cloned();
        let max_age = cache_control.as_ref().and_then(|cc| Self::parse_max_age(cc));

        // Populate CacheInfo on the response
        response.cache_info = Some(CacheInfo {
            etag: etag.clone(),
            last_modified: last_modified.clone(),
            cache_control: cache_control.clone(),
            max_age,
            is_cached: false,
        });

        // Cache the response if cacheable
        if response.is_success() && !Self::should_not_cache(&response.headers) {
            // Evict if at capacity
            if self.cache.len() >= self.config.max_entries {
                // Simple eviction: remove expired entries, then oldest if needed
                self.cache.retain(|_, v| v.expires_at > Instant::now());
                if self.cache.len() >= self.config.max_entries {
                    // Remove first entry found
                    if let Some(first_key) = self.cache.iter().next().map(|e| e.key().clone()) {
                        self.cache.remove(&first_key);
                    }
                }
            }

            let ttl = max_age
                .map(Duration::from_secs)
                .unwrap_or(self.config.default_ttl);

            self.cache.insert(
                key,
                CacheEntry {
                    response: response.clone(),
                    etag,
                    last_modified,
                    expires_at: Instant::now() + ttl,
                },
            );
        }

        Ok(PipelineAction::Continue(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;

    fn make_ctx() -> PipelineContext {
        PipelineContext::new(MtwRequest::get("http://example.com/data"))
    }

    #[tokio::test]
    async fn test_caches_successful_response() {
        let stage = CacheStage::new();
        let resp = MtwResponse::new(200)
            .with_header("etag", "\"abc123\"")
            .with_body(b"cached data".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
        assert_eq!(stage.entry_count(), 1);
    }

    #[tokio::test]
    async fn test_304_returns_cached_response() {
        let stage = CacheStage::new();
        let mut ctx = make_ctx();

        // First: cache a successful response
        let resp = MtwResponse::new(200)
            .with_header("etag", "\"abc\"")
            .with_body(b"original data".to_vec());
        stage.process(resp, &mut ctx).await.unwrap();

        // Second: 304 should return cached
        let resp304 = MtwResponse::new(304);
        let result = stage.process(resp304, &mut ctx).await.unwrap();
        if let PipelineAction::Cached(resp) = result {
            assert_eq!(resp.status, 200);
            assert!(resp.cache_info.as_ref().unwrap().is_cached);
        } else {
            panic!("expected Cached action");
        }
    }

    #[tokio::test]
    async fn test_no_store_not_cached() {
        let stage = CacheStage::new();
        let resp = MtwResponse::new(200)
            .with_header("cache-control", "no-store")
            .with_body(b"secret".to_vec());
        let mut ctx = make_ctx();

        stage.process(resp, &mut ctx).await.unwrap();
        assert_eq!(stage.entry_count(), 0);
    }

    #[tokio::test]
    async fn test_max_age_parsing() {
        assert_eq!(CacheStage::parse_max_age("max-age=3600"), Some(3600));
        assert_eq!(
            CacheStage::parse_max_age("public, max-age=600"),
            Some(600)
        );
        assert_eq!(CacheStage::parse_max_age("no-cache"), None);
    }

    #[tokio::test]
    async fn test_cache_info_populated() {
        let stage = CacheStage::new();
        let resp = MtwResponse::new(200)
            .with_header("etag", "\"xyz\"")
            .with_header("last-modified", "Mon, 01 Jan 2024 00:00:00 GMT")
            .with_header("cache-control", "max-age=600")
            .with_body(b"data".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let info = resp.cache_info.unwrap();
            assert_eq!(info.etag.as_deref(), Some("\"xyz\""));
            assert!(info.last_modified.is_some());
            assert_eq!(info.max_age, Some(600));
            assert!(!info.is_cached);
        } else {
            panic!("expected Continue");
        }
    }
}
