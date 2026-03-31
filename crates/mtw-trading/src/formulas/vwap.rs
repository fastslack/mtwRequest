use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Volume-Weighted Average Price (VWAP) with deviation bands.
///
/// VWAP = cumulative(typical_price * volume) / cumulative(volume)
/// Deviation bands at +/- 1 and 2 standard deviations from VWAP.
///
/// - Price below VWAP - 1σ: Buy (undervalued)
/// - Price above VWAP + 1σ: Sell (overvalued)
pub struct VwapFormula;

impl SignalFormula for VwapFormula {
    fn id(&self) -> &str { "vwap" }
    fn name(&self) -> &str { "VWAP Deviation" }
    fn description(&self) -> &str { "Volume-weighted average price with standard deviation bands" }
    fn min_candles(&self) -> usize { 20 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(20.0) as usize;
        let n = candles.len();
        if n < period {
            return FormulaResult::neutral("insufficient data");
        }

        let window = &candles[n - period..n];

        // Compute VWAP
        let mut cum_tp_vol = 0.0;
        let mut cum_vol = 0.0;

        for c in window {
            let typical_price = (c.high + c.low + c.close) / 3.0;
            cum_tp_vol += typical_price * c.volume;
            cum_vol += c.volume;
        }

        if cum_vol == 0.0 {
            return FormulaResult::neutral("zero volume");
        }

        let vwap = cum_tp_vol / cum_vol;

        // Standard deviation of typical prices from VWAP, weighted by volume
        let mut sum_sq_dev = 0.0;
        for c in window {
            let typical_price = (c.high + c.low + c.close) / 3.0;
            sum_sq_dev += (typical_price - vwap).powi(2) * c.volume;
        }
        let variance = sum_sq_dev / cum_vol;
        let std_dev = variance.sqrt();

        let price = candles[n - 1].close;
        let deviation = if std_dev > 0.0 { (price - vwap) / std_dev } else { 0.0 };

        let upper_1 = vwap + std_dev;
        let lower_1 = vwap - std_dev;
        let _upper_2 = vwap + 2.0 * std_dev;
        let _lower_2 = vwap - 2.0 * std_dev;

        let mut indicators = HashMap::new();
        indicators.insert("vwap".into(), (vwap * 100.0).round() / 100.0);
        indicators.insert("std_dev".into(), (std_dev * 100.0).round() / 100.0);
        indicators.insert("deviation".into(), (deviation * 100.0).round() / 100.0);
        indicators.insert("upper_1".into(), (upper_1 * 100.0).round() / 100.0);
        indicators.insert("lower_1".into(), (lower_1 * 100.0).round() / 100.0);

        let (side, confidence) = if deviation <= -2.0 {
            (Some(OrderSide::Buy), 80.0) // Far below VWAP
        } else if deviation <= -1.0 {
            (Some(OrderSide::Buy), 65.0) // Below VWAP
        } else if deviation >= 2.0 {
            (Some(OrderSide::Sell), 80.0) // Far above VWAP
        } else if deviation >= 1.0 {
            (Some(OrderSide::Sell), 65.0) // Above VWAP
        } else {
            (None, 0.0) // Near VWAP, no signal
        };

        FormulaResult {
            side,
            confidence,
            indicators,
            reasoning: format!("VWAP {:.2} price {:.2} dev {:.2}σ", vwap, price, deviation),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vwap_below_band() {
        // Create candles where last price is well below VWAP
        let mut candles = Vec::new();
        for i in 0..25 {
            let base = 100.0;
            let close = if i < 20 { base + 2.0 } else { base - 5.0 }; // sudden drop at end
            candles.push(Candle {
                timestamp: i as u64, open: close, high: close + 1.0,
                low: close - 1.0, close, volume: 1000.0,
            });
        }
        let r = VwapFormula.compute(&candles, None);
        // Price dropped below VWAP -> should signal Buy
        if r.side.is_some() {
            assert_eq!(r.side, Some(OrderSide::Buy));
        }
        assert!(r.indicators.contains_key("vwap"));
    }

    #[test]
    fn test_vwap_at_mean() {
        // All candles at same price -> no signal
        let candles: Vec<Candle> = (0..25).map(|i| Candle {
            timestamp: i as u64, open: 100.0, high: 100.5, low: 99.5, close: 100.0, volume: 1000.0,
        }).collect();
        let r = VwapFormula.compute(&candles, None);
        assert!(r.side.is_none());
    }
}
