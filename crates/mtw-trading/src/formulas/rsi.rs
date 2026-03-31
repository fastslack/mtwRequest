use std::collections::HashMap;
use crate::formula::{compute_rsi, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

pub struct RsiFormula;

impl SignalFormula for RsiFormula {
    fn id(&self) -> &str { "rsi" }
    fn name(&self) -> &str { "RSI (14)" }
    fn description(&self) -> &str { "Mean-reversion: buy oversold, sell overbought" }
    fn min_candles(&self) -> usize { 55 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(14.0) as usize;
        let rsi = compute_rsi(&closes, period);
        if rsi.is_empty() { return FormulaResult::neutral("insufficient data"); }
        let val = rsi[rsi.len() - 1];
        let oversold = val < 25.0;
        let overbought = val > 75.0;
        let mut indicators = HashMap::new();
        indicators.insert("rsi".to_string(), (val * 100.0).round() / 100.0);
        FormulaResult {
            side: if oversold { Some(OrderSide::Buy) } else if overbought { Some(OrderSide::Sell) } else { None },
            confidence: if oversold || overbought { 75.0 } else { 0.0 },
            indicators,
            reasoning: format!("RSI({}) {:.1}", period, val),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_candles(prices: &[f64]) -> Vec<Candle> {
        prices.iter().enumerate().map(|(i, &p)| Candle {
            timestamp: i as u64, open: p, high: p + 1.0, low: p - 1.0, close: p, volume: 100.0,
        }).collect()
    }

    #[test]
    fn test_rsi_uptrend() {
        let prices: Vec<f64> = (0..60).map(|i| 100.0 + i as f64).collect();
        let r = RsiFormula.compute(&make_candles(&prices), None);
        assert_eq!(r.side, Some(OrderSide::Sell)); // overbought
    }

    #[test]
    fn test_rsi_downtrend() {
        let prices: Vec<f64> = (0..60).map(|i| 200.0 - i as f64).collect();
        let r = RsiFormula.compute(&make_candles(&prices), None);
        assert_eq!(r.side, Some(OrderSide::Buy)); // oversold
    }
}
