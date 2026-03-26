//! Exchange and broker connectivity for mtwRequest.
//!
//! This crate provides a trait-based abstraction (`ExchangeProvider`) for connecting to
//! cryptocurrency exchanges and brokers. It includes:
//!
//! - **REST API clients** for market data, account info, and order management
//! - **WebSocket streaming** for real-time ticker, candle, and order book updates
//! - **Rate limiting** (token bucket and sliding window) built into each provider
//! - **Parallel fetching** for multiple symbols across exchanges
//!
//! # Supported Exchanges
//!
//! - **Bitvavo** — full REST + WebSocket implementation
//! - Kraken, Coinbase, Bybit, Binance, KuCoin, Bitget, MEXC — stubs (planned)
//!
//! # Example
//!
//! ```rust,no_run
//! use mtw_exchange::providers::bitvavo::BitvavoProvider;
//! use mtw_exchange::provider::ExchangeProvider;
//! use mtw_exchange::config::ExchangeCredentials;
//! use mtw_exchange::rate_limit::TokenBucket;
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let creds = ExchangeCredentials {
//!     api_key: "your-key".into(),
//!     api_secret: "your-secret".into(),
//!     passphrase: None,
//! };
//! let limiter = Arc::new(TokenBucket::new(10, Duration::from_secs(1)));
//! let provider = BitvavoProvider::new(creds, limiter);
//!
//! let ticker = provider.get_ticker("BTC-EUR").await?;
//! println!("BTC-EUR last: {}", ticker.last);
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;
pub mod manager;
pub mod module;
pub mod provider;
pub mod providers;
pub mod rate_limit;
pub mod types;

pub use config::{ExchangeConfig, ExchangeCredentials, ProviderConfig};
pub use error::ExchangeError;
pub use manager::ExchangeManager;
pub use module::ExchangeModule;
pub use provider::{ExchangeProvider, ProviderCapabilities};
pub use types::*;
