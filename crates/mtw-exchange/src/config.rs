use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level exchange configuration (embedded in mtw.toml under [[modules]])
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeConfig {
    /// Configured exchange providers
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Default poll interval in seconds (for REST fallback when WS unavailable)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Maximum concurrent requests across all providers
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_requests: usize,
}

fn default_poll_interval() -> u64 {
    10
}

fn default_max_concurrent() -> usize {
    50
}

impl Default for ExchangeConfig {
    fn default() -> Self {
        Self {
            providers: vec![],
            poll_interval_secs: default_poll_interval(),
            max_concurrent_requests: default_max_concurrent(),
        }
    }
}

/// Per-provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Exchange identifier (e.g., "bitvavo", "kraken")
    pub exchange_id: String,
    /// API key (supports ${ENV_VAR} expansion via mtw-core)
    pub api_key: String,
    /// API secret
    pub api_secret: String,
    /// Optional passphrase (KuCoin, etc.)
    pub passphrase: Option<String>,
    /// Use sandbox/testnet mode
    #[serde(default)]
    pub sandbox: bool,
    /// Symbols to subscribe to via WebSocket
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Timeframes for candle subscriptions (e.g., ["1m", "5m", "1h"])
    #[serde(default)]
    pub timeframes: Vec<String>,
    /// Enable WebSocket streaming (if exchange supports it)
    #[serde(default = "default_true")]
    pub websocket_enabled: bool,
    /// Rate limit configuration (overrides exchange defaults)
    pub rate_limit: Option<RateLimitConfig>,
    /// Extra provider-specific settings
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Strategy: "token_bucket" or "sliding_window"
    pub strategy: String,
    /// Max requests (bucket size or window max)
    pub max_requests: u32,
    /// Window/refill duration in milliseconds
    pub window_ms: u64,
}

/// Credentials for authenticating with an exchange
#[derive(Debug, Clone)]
pub struct ExchangeCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
}

impl From<&ProviderConfig> for ExchangeCredentials {
    fn from(config: &ProviderConfig) -> Self {
        Self {
            api_key: config.api_key.clone(),
            api_secret: config.api_secret.clone(),
            passphrase: config.passphrase.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ExchangeConfig::default();
        assert_eq!(config.poll_interval_secs, 10);
        assert_eq!(config.max_concurrent_requests, 50);
        assert!(config.providers.is_empty());
    }

    #[test]
    fn test_provider_config_deserialization() {
        let toml_str = r#"
            exchange_id = "bitvavo"
            api_key = "${BITVAVO_API_KEY}"
            api_secret = "${BITVAVO_API_SECRET}"
            sandbox = false
            symbols = ["BTC-EUR", "ETH-EUR"]
            timeframes = ["1m", "5m", "1h"]
            websocket_enabled = true
        "#;
        let config: ProviderConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.exchange_id, "bitvavo");
        assert_eq!(config.symbols.len(), 2);
        assert!(config.websocket_enabled);
    }

    #[test]
    fn test_rate_limit_config() {
        let toml_str = r#"
            strategy = "token_bucket"
            max_requests = 10
            window_ms = 1000
        "#;
        let config: RateLimitConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.strategy, "token_bucket");
        assert_eq!(config.max_requests, 10);
    }
}
