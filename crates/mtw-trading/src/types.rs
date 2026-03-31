use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass { Crypto, Stock, Etf, Forex, Future, Option, Cfd, Bond }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide { Buy, Sell }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType { Market, Limit, Stop, StopLimit }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeStatus { Pending, Open, Filled, Partial, Cancelled, Failed }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyStatus { Active, Paused, Stopped, Error }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Timeframe { M1, M5, M15, H1, H4, D1, W1 }

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::M1 => write!(f, "1m"), Self::M5 => write!(f, "5m"),
            Self::M15 => write!(f, "15m"), Self::H1 => write!(f, "1h"),
            Self::H4 => write!(f, "4h"), Self::D1 => write!(f, "1d"),
            Self::W1 => write!(f, "1w"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticker {
    pub symbol: String, pub bid: f64, pub ask: f64, pub last: f64,
    pub volume: f64, pub change_24h: f64, pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub currency: String, pub free: f64, pub used: f64, pub total: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: u64, pub open: f64, pub high: f64,
    pub low: f64, pub close: f64, pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String, pub strategy_id: String, pub account_id: String,
    pub exchange_id: String, pub symbol: String, pub side: OrderSide,
    pub order_type: OrderType, pub status: TradeStatus,
    pub amount: f64, pub price: f64, pub filled_amount: f64, pub filled_price: f64,
    pub cost: f64, pub fee: f64, pub fee_currency: String,
    pub stop_loss: Option<f64>, pub take_profit: Option<f64>,
    pub trailing_stop_pct: Option<f64>, pub highest_price: Option<f64>,
    pub exchange_order_id: String, pub pnl: Option<f64>,
    pub signal_info: String, pub error: String, pub session_id: String,
    pub opened_at: String, pub closed_at: Option<String>, pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub id: String, pub name: String, pub description: String,
    pub account_id: String, pub agent_id: String,
    pub symbols: Vec<String>, pub timeframe: Timeframe,
    pub max_position_pct: f64, pub max_open_trades: u32,
    pub stop_loss_pct: f64, pub take_profit_pct: f64,
    pub trailing_stop_pct: f64, pub cooldown_ms: u64,
    pub status: StrategyStatus, pub last_signal_at: Option<String>,
    pub total_trades: u32, pub total_pnl_cents: i64, pub win_rate: f64,
    pub min_consensus: u32, pub min_confidence: f64, pub max_drawdown_pct: f64,
    pub preset: String, pub created_at: String, pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_timeframe_display() {
        assert_eq!(Timeframe::M1.to_string(), "1m");
        assert_eq!(Timeframe::H4.to_string(), "4h");
        assert_eq!(Timeframe::D1.to_string(), "1d");
    }

    #[test]
    fn test_candle_copy() {
        let c = Candle { timestamp: 0, open: 1.0, high: 2.0, low: 0.5, close: 1.5, volume: 100.0 };
        let c2 = c;
        assert_eq!(c2.close, 1.5);
    }
}
