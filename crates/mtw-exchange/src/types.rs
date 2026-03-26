use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Supported exchange identifiers
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeId {
    Bitvavo,
    Kraken,
    Coinbase,
    Bybit,
    Binance,
    KuCoin,
    Bitget,
    Mexc,
    Custom(String),
}

impl fmt::Display for ExchangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom(name) => write!(f, "{}", name),
            other => write!(f, "{}", format!("{:?}", other).to_lowercase()),
        }
    }
}

/// Asset class
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    Crypto,
    Stock,
    Etf,
    Forex,
    Future,
    Option,
    Cfd,
    Bond,
}

/// Ticker data snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticker {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume: f64,
    pub change_24h: f64,
    pub timestamp: u64,
}

/// OHLCV candle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Account balance per currency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub currency: String,
    pub free: f64,
    pub used: f64,
    pub total: f64,
}

/// Order book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub symbol: String,
    pub bids: Vec<OrderBookLevel>,
    pub asks: Vec<OrderBookLevel>,
    pub timestamp: u64,
}

/// Single price level in the order book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookLevel {
    pub price: f64,
    pub quantity: f64,
}

/// Order side
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

impl fmt::Display for OrderSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buy => write!(f, "buy"),
            Self::Sell => write!(f, "sell"),
        }
    }
}

/// Order type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
}

/// Order status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Open,
    Closed,
    Canceled,
    Expired,
    Rejected,
}

/// Order parameters for creating an order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrderParams {
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub amount: f64,
    pub price: Option<f64>,
    pub stop_price: Option<f64>,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

/// Unified order result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub status: OrderStatus,
    pub amount: f64,
    pub filled: f64,
    pub average: f64,
    pub cost: f64,
    pub fee: Option<Fee>,
    pub timestamp: u64,
}

/// Trading fee
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fee {
    pub cost: f64,
    pub currency: String,
}

/// Exchange event — emitted by providers and WebSocket streams
#[derive(Debug, Clone)]
pub enum ExchangeEvent {
    TickerUpdate {
        exchange: ExchangeId,
        ticker: Ticker,
    },
    CandleUpdate {
        exchange: ExchangeId,
        symbol: String,
        timeframe: String,
        candle: Candle,
    },
    OrderBookUpdate {
        exchange: ExchangeId,
        book: OrderBook,
    },
    OrderUpdate {
        exchange: ExchangeId,
        order: Order,
    },
    Connected {
        exchange: ExchangeId,
    },
    Disconnected {
        exchange: ExchangeId,
        reason: String,
    },
    Error {
        exchange: ExchangeId,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exchange_id_display() {
        assert_eq!(ExchangeId::Bitvavo.to_string(), "bitvavo");
        assert_eq!(ExchangeId::Kraken.to_string(), "kraken");
        assert_eq!(
            ExchangeId::Custom("myexchange".into()).to_string(),
            "myexchange"
        );
    }

    #[test]
    fn test_ticker_serialization() {
        let ticker = Ticker {
            symbol: "BTC-EUR".into(),
            bid: 95000.0,
            ask: 95010.0,
            last: 95005.0,
            volume: 1234.5,
            change_24h: 2.5,
            timestamp: 1711000000000,
        };
        let json = serde_json::to_string(&ticker).unwrap();
        let parsed: Ticker = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.symbol, "BTC-EUR");
        assert_eq!(parsed.last, 95005.0);
    }

    #[test]
    fn test_candle_serialization() {
        let candle = Candle {
            timestamp: 1711000000000,
            open: 95000.0,
            high: 95500.0,
            low: 94800.0,
            close: 95200.0,
            volume: 100.5,
        };
        let json = serde_json::to_string(&candle).unwrap();
        let parsed: Candle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.close, 95200.0);
    }

    #[test]
    fn test_order_side_display() {
        assert_eq!(OrderSide::Buy.to_string(), "buy");
        assert_eq!(OrderSide::Sell.to_string(), "sell");
    }

    #[test]
    fn test_create_order_params() {
        let params = CreateOrderParams {
            symbol: "BTC-EUR".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            amount: 0.01,
            price: None,
            stop_price: None,
            params: HashMap::new(),
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["side"], "buy");
        assert_eq!(json["order_type"], "market");
    }
}
