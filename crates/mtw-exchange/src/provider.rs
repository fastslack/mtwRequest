use async_trait::async_trait;

use crate::error::ExchangeError;
use crate::types::*;

/// Capabilities advertised by a provider
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    /// Supports batch ticker fetch (single API call for multiple symbols)
    pub batch_tickers: bool,
    /// Supports WebSocket streaming
    pub websocket: bool,
    /// Supports order book streaming
    pub order_book_stream: bool,
    /// Supports candle streaming
    pub candle_stream: bool,
    /// Supports stop loss as a native order type
    pub native_stop_loss: bool,
    /// Supports take profit as a native order type
    pub native_take_profit: bool,
    /// Whether sandbox/testnet mode is available
    pub sandbox: bool,
}

/// Abstract interface for any exchange or broker.
///
/// Implementations handle authentication, rate limiting, and API-specific details.
/// The trait is designed to be object-safe for use with `Arc<dyn ExchangeProvider>`.
#[async_trait]
pub trait ExchangeProvider: Send + Sync {
    /// Exchange identifier
    fn id(&self) -> &ExchangeId;

    /// Human-readable exchange name
    fn name(&self) -> &str;

    /// Advertised capabilities
    fn capabilities(&self) -> &ProviderCapabilities;

    // -- Market Data --

    /// Fetch a single ticker
    async fn get_ticker(&self, symbol: &str) -> Result<Ticker, ExchangeError>;

    /// Fetch multiple tickers (batch or parallel depending on exchange capabilities)
    async fn get_tickers(&self, symbols: &[String]) -> Result<Vec<Ticker>, ExchangeError>;

    /// Fetch OHLCV candles
    async fn get_candles(
        &self,
        symbol: &str,
        timeframe: &str,
        limit: usize,
    ) -> Result<Vec<Candle>, ExchangeError>;

    /// Fetch order book
    async fn get_order_book(
        &self,
        symbol: &str,
        limit: usize,
    ) -> Result<OrderBook, ExchangeError>;

    /// Fetch available trading pairs/markets
    async fn get_markets(&self) -> Result<Vec<String>, ExchangeError>;

    // -- Account --

    /// Fetch account balances
    async fn get_balances(&self) -> Result<Vec<Balance>, ExchangeError>;

    // -- Trading --

    /// Create an order
    async fn create_order(&self, params: CreateOrderParams) -> Result<Order, ExchangeError>;

    /// Cancel an order by ID
    async fn cancel_order(&self, order_id: &str, symbol: &str) -> Result<(), ExchangeError>;

    /// Get open orders, optionally filtered by symbol
    async fn get_open_orders(&self, symbol: Option<&str>) -> Result<Vec<Order>, ExchangeError>;

    /// Get a specific order by ID
    async fn get_order(&self, order_id: &str, symbol: &str) -> Result<Order, ExchangeError>;

    // -- Lifecycle --

    /// Graceful shutdown (close connections, flush buffers)
    async fn shutdown(&self) -> Result<(), ExchangeError>;
}
