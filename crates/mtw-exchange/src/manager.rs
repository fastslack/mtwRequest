use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::ExchangeConfig;
use crate::error::ExchangeError;
use crate::provider::ExchangeProvider;
use crate::types::*;

/// Manages all exchange providers, routes events, and handles parallel fetching.
///
/// The manager maintains a registry of providers (keyed by a user-defined string, typically
/// the account ID or exchange name) and an unbounded event channel that aggregates events
/// from both REST polling and WebSocket streams.
pub struct ExchangeManager {
    /// Registered providers by key (e.g., "bitvavo-main", "kraken-trading")
    providers: DashMap<String, Arc<dyn ExchangeProvider>>,
    /// Event channel — receives from all providers (REST + WS)
    event_tx: mpsc::UnboundedSender<ExchangeEvent>,
    event_rx: Option<mpsc::UnboundedReceiver<ExchangeEvent>>,
    /// Latest ticker cache (for fast lookups without hitting the exchange)
    ticker_cache: DashMap<String, Ticker>,
    /// Configuration
    config: ExchangeConfig,
}

impl ExchangeManager {
    /// Create a new exchange manager.
    pub fn new(config: ExchangeConfig) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            providers: DashMap::new(),
            event_tx,
            event_rx: Some(event_rx),
            ticker_cache: DashMap::new(),
            config,
        }
    }

    /// Take the event receiver (consumed by ExchangeModule to forward events to channels).
    /// Can only be called once.
    pub fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<ExchangeEvent>> {
        self.event_rx.take()
    }

    /// Get the event sender (passed to WS clients and polling tasks).
    pub fn event_sender(&self) -> mpsc::UnboundedSender<ExchangeEvent> {
        self.event_tx.clone()
    }

    /// Register a provider with a key.
    pub fn register_provider(&self, key: String, provider: Arc<dyn ExchangeProvider>) {
        tracing::info!(key = %key, exchange = %provider.name(), "registered exchange provider");
        self.providers.insert(key, provider);
    }

    /// Get a provider by key.
    pub fn get_provider(&self, key: &str) -> Option<Arc<dyn ExchangeProvider>> {
        self.providers.get(key).map(|p| p.clone())
    }

    /// List all registered provider keys.
    pub fn list_providers(&self) -> Vec<String> {
        self.providers.iter().map(|e| e.key().clone()).collect()
    }

    /// Fetch tickers for multiple symbols in parallel via a specific provider.
    pub async fn fetch_tickers_parallel(
        &self,
        provider_key: &str,
        symbols: &[String],
    ) -> Result<Vec<Ticker>, ExchangeError> {
        let provider = self
            .providers
            .get(provider_key)
            .ok_or_else(|| ExchangeError::NotFound(provider_key.to_string()))?;

        let tickers = provider.get_tickers(symbols).await?;

        // Update cache
        for ticker in &tickers {
            let cache_key = format!("{}:{}", provider_key, ticker.symbol);
            self.ticker_cache.insert(cache_key, ticker.clone());
        }

        Ok(tickers)
    }

    /// Fetch candles for multiple symbols in parallel via tokio tasks.
    pub async fn fetch_candles_parallel(
        &self,
        provider_key: &str,
        symbols: &[String],
        timeframe: &str,
        limit: usize,
    ) -> Result<Vec<(String, Vec<Candle>)>, ExchangeError> {
        let provider = self
            .providers
            .get(provider_key)
            .ok_or_else(|| ExchangeError::NotFound(provider_key.to_string()))?
            .clone();

        let max_concurrent = self.config.max_concurrent_requests;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

        let mut handles = Vec::with_capacity(symbols.len());
        for symbol in symbols {
            let provider = provider.clone();
            let symbol = symbol.clone();
            let timeframe = timeframe.to_string();
            let sem = semaphore.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let candles = provider.get_candles(&symbol, &timeframe, limit).await?;
                Ok::<_, ExchangeError>((symbol, candles))
            }));
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(Ok(result)) => results.push(result),
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "failed to fetch candles for symbol");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "task panicked during candle fetch");
                }
            }
        }

        Ok(results)
    }

    /// Get a cached ticker (no exchange call).
    pub fn cached_ticker(&self, provider_key: &str, symbol: &str) -> Option<Ticker> {
        let cache_key = format!("{}:{}", provider_key, symbol);
        self.ticker_cache.get(&cache_key).map(|t| t.clone())
    }

    /// Update the ticker cache (called by WS event handlers).
    pub fn update_ticker_cache(&self, provider_key: &str, ticker: Ticker) {
        let cache_key = format!("{}:{}", provider_key, ticker.symbol);
        self.ticker_cache.insert(cache_key, ticker);
    }

    /// Get all cached tickers.
    pub fn all_cached_tickers(&self) -> Vec<(String, Ticker)> {
        self.ticker_cache
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// Provider count.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// Configuration reference.
    pub fn config(&self) -> &ExchangeConfig {
        &self.config
    }

    /// Shutdown all providers.
    pub async fn shutdown(&self) {
        // Collect providers to avoid holding DashMap iterator across await
        let providers: Vec<(String, Arc<dyn ExchangeProvider>)> = self
            .providers
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();

        for (key, provider) in providers {
            if let Err(e) = provider.shutdown().await {
                tracing::error!(
                    provider = %key,
                    error = %e,
                    "error shutting down exchange provider"
                );
            }
        }
        tracing::info!("all exchange providers shut down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExchangeConfig;

    #[test]
    fn test_manager_creation() {
        let manager = ExchangeManager::new(ExchangeConfig::default());
        assert_eq!(manager.provider_count(), 0);
        assert!(manager.list_providers().is_empty());
    }

    #[test]
    fn test_ticker_cache() {
        let manager = ExchangeManager::new(ExchangeConfig::default());
        let ticker = Ticker {
            symbol: "BTC-EUR".into(),
            bid: 95000.0,
            ask: 95010.0,
            last: 95005.0,
            volume: 1234.5,
            change_24h: 2.5,
            timestamp: 1711000000000,
        };
        manager.update_ticker_cache("bitvavo", ticker.clone());
        let cached = manager.cached_ticker("bitvavo", "BTC-EUR");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().last, 95005.0);
    }

    #[test]
    fn test_event_receiver_take_once() {
        let mut manager = ExchangeManager::new(ExchangeConfig::default());
        assert!(manager.take_event_receiver().is_some());
        assert!(manager.take_event_receiver().is_none());
    }
}
