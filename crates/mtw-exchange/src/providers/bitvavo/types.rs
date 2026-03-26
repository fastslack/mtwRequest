//! Bitvavo-specific API response types for serde deserialization.
//!
//! These map directly to the Bitvavo REST API v2 responses.
//! See: https://docs.bitvavo.com/

use serde::Deserialize;

/// GET /ticker24h response item
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BitvavoTicker24h {
    pub market: String,
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub last: Option<String>,
    pub volume: Option<String>,
    pub volume_quote: Option<String>,
    pub bid: Option<String>,
    pub bid_size: Option<String>,
    pub ask: Option<String>,
    pub ask_size: Option<String>,
    pub timestamp: Option<u64>,
}

/// GET /{market}/candles response is Vec<Vec<Value>>
/// Each candle: [timestamp, open, high, low, close, volume]
/// All values are returned as arrays of mixed types
#[derive(Debug, Deserialize)]
pub struct BitvavoCandle(pub Vec<serde_json::Value>);

impl BitvavoCandle {
    pub fn timestamp(&self) -> Option<u64> {
        self.0.first()?.as_u64()
    }

    pub fn open(&self) -> Option<f64> {
        parse_f64(self.0.get(1)?)
    }

    pub fn high(&self) -> Option<f64> {
        parse_f64(self.0.get(2)?)
    }

    pub fn low(&self) -> Option<f64> {
        parse_f64(self.0.get(3)?)
    }

    pub fn close(&self) -> Option<f64> {
        parse_f64(self.0.get(4)?)
    }

    pub fn volume(&self) -> Option<f64> {
        parse_f64(self.0.get(5)?)
    }
}

/// GET /{market}/book response
#[derive(Debug, Deserialize)]
pub struct BitvavoOrderBook {
    pub market: String,
    pub nonce: u64,
    pub bids: Vec<Vec<String>>,
    pub asks: Vec<Vec<String>>,
}

/// GET /balance response item
#[derive(Debug, Deserialize)]
pub struct BitvavoBalance {
    pub symbol: String,
    pub available: String,
    #[serde(rename = "inOrder")]
    pub in_order: String,
}

/// POST /order response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BitvavoOrder {
    pub order_id: String,
    pub market: String,
    pub side: String,
    pub order_type: String,
    pub status: String,
    pub amount: Option<String>,
    pub amount_remaining: Option<String>,
    pub price: Option<String>,
    pub amount_quote: Option<String>,
    pub filled_amount: Option<String>,
    pub filled_amount_quote: Option<String>,
    pub filled_price: Option<String>,
    pub fee_paid: Option<String>,
    pub fee_currency: Option<String>,
    pub created: Option<u64>,
    pub updated: Option<u64>,
}

/// GET /markets response item
#[derive(Debug, Deserialize)]
pub struct BitvavoMarket {
    pub market: String,
    pub status: String,
    pub base: String,
    pub quote: String,
}

/// Bitvavo WebSocket event wrapper
#[derive(Debug, Deserialize)]
pub struct BitvavoWsEvent {
    pub event: String,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default)]
    pub error: Option<String>,
}

/// Bitvavo WebSocket ticker24h event data
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BitvavoWsTicker {
    pub market: String,
    pub best_bid: Option<String>,
    pub best_bid_size: Option<String>,
    pub best_ask: Option<String>,
    pub best_ask_size: Option<String>,
    pub last: Option<String>,
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub volume: Option<String>,
    pub volume_quote: Option<String>,
    pub timestamp: Option<u64>,
}

/// Bitvavo WebSocket candle event data
#[derive(Debug, Deserialize)]
pub struct BitvavoWsCandle {
    pub market: String,
    pub interval: String,
    pub candle: Vec<serde_json::Value>,
}

/// Parse a JSON value as f64 (handles both string and number representations)
fn parse_f64(val: &serde_json::Value) -> Option<f64> {
    val.as_f64()
        .or_else(|| val.as_str().and_then(|s| s.parse::<f64>().ok()))
}

/// Parse a string reference to f64, defaulting to 0.0
pub fn parse_str_f64(s: &Option<String>) -> f64 {
    s.as_deref()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_str_f64() {
        assert_eq!(parse_str_f64(&Some("95000.50".into())), 95000.50);
        assert_eq!(parse_str_f64(&None), 0.0);
        assert_eq!(parse_str_f64(&Some("invalid".into())), 0.0);
    }

    #[test]
    fn test_ticker24h_deserialization() {
        let json = r#"{
            "market": "BTC-EUR",
            "open": "94000",
            "high": "96000",
            "low": "93500",
            "last": "95200",
            "volume": "1234.5",
            "volumeQuote": "117000000",
            "bid": "95190",
            "bidSize": "0.5",
            "ask": "95210",
            "askSize": "0.3",
            "timestamp": 1711000000000
        }"#;
        let ticker: BitvavoTicker24h = serde_json::from_str(json).unwrap();
        assert_eq!(ticker.market, "BTC-EUR");
        assert_eq!(ticker.last, Some("95200".into()));
    }

    #[test]
    fn test_candle_parsing() {
        let json = r#"[1711000000000, "95000", "95500", "94800", "95200", "100.5"]"#;
        let candle: BitvavoCandle = serde_json::from_str(json).unwrap();
        assert_eq!(candle.timestamp(), Some(1711000000000));
        assert_eq!(candle.open(), Some(95000.0));
        assert_eq!(candle.close(), Some(95200.0));
        assert_eq!(candle.volume(), Some(100.5));
    }

    #[test]
    fn test_balance_deserialization() {
        let json = r#"{"symbol": "BTC", "available": "1.5", "inOrder": "0.2"}"#;
        let balance: BitvavoBalance = serde_json::from_str(json).unwrap();
        assert_eq!(balance.symbol, "BTC");
        assert_eq!(balance.available, "1.5");
    }

    #[test]
    fn test_ws_event_deserialization() {
        let json = r#"{"event": "ticker24h", "data": {"market": "BTC-EUR"}}"#;
        let event: BitvavoWsEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event, "ticker24h");
        assert!(event.error.is_none());
    }
}
