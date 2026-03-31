use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::OrderSide;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub id: String, pub strategy_id: String, pub symbol: String,
    pub side: OrderSide, pub confidence: f64,
    pub analysis: String, pub indicators: HashMap<String, f64>,
    pub acted_on: bool, pub trade_id: Option<String>, pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConsensus {
    pub symbol: String, pub side: Option<OrderSide>,
    pub avg_confidence: f64, pub formula_count: u32,
    pub formulas: Vec<String>, pub total_checked: u32,
}

impl SignalConsensus {
    pub fn meets_threshold(&self, min_consensus: u32, min_confidence: f64) -> bool {
        self.formula_count >= min_consensus && self.avg_confidence >= min_confidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_consensus_threshold() {
        let c = SignalConsensus {
            symbol: "BTC/USDT".into(), side: Some(OrderSide::Buy),
            avg_confidence: 75.0, formula_count: 4,
            formulas: vec!["rsi".into(), "macd".into(), "ema".into(), "bb".into()],
            total_checked: 6,
        };
        assert!(c.meets_threshold(3, 70.0));
        assert!(!c.meets_threshold(5, 70.0));
        assert!(!c.meets_threshold(3, 80.0));
    }
}
