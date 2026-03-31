use std::collections::HashMap;
use crate::formula::{compute_atr, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

pub struct SuperTrendFormula;

impl SignalFormula for SuperTrendFormula {
    fn id(&self) -> &str { "supertrend" }
    fn name(&self) -> &str { "SuperTrend (10, 3.0)" }
    fn description(&self) -> &str { "Trend: ATR-based trend flip detection" }
    fn min_candles(&self) -> usize { 20 }

    fn compute(&self, candles: &[Candle], _context: Option<&FormulaContext>) -> FormulaResult {
        let period = 10;
        let multiplier = 3.0;
        let atr = compute_atr(candles, period);
        if atr.len() < 2 { return FormulaResult::neutral("insufficient data"); }
        let n = candles.len() - 1;
        let hl2 = (candles[n].high + candles[n].low) / 2.0;
        let upper_band = hl2 + multiplier * atr[n];
        let lower_band = hl2 - multiplier * atr[n];
        let prev_hl2 = (candles[n - 1].high + candles[n - 1].low) / 2.0;
        let prev_upper = prev_hl2 + multiplier * atr[n - 1];
        let prev_lower = prev_hl2 - multiplier * atr[n - 1];
        let close = candles[n].close;
        let prev_close = candles[n - 1].close;
        // Simplified: if close crosses above upper_band -> buy, below lower -> sell
        let flip_up = prev_close <= prev_upper && close > lower_band && close > prev_close;
        let flip_down = prev_close >= prev_lower && close < upper_band && close < prev_close;
        let mut indicators = HashMap::new();
        indicators.insert("atr".into(), (atr[n] * 100.0).round() / 100.0);
        indicators.insert("upper".into(), (upper_band * 100.0).round() / 100.0);
        indicators.insert("lower".into(), (lower_band * 100.0).round() / 100.0);
        FormulaResult {
            side: if flip_up { Some(OrderSide::Buy) } else if flip_down { Some(OrderSide::Sell) } else { None },
            confidence: if flip_up || flip_down { 65.0 } else { 0.0 },
            indicators,
            reasoning: format!("SuperTrend ATR {:.2} bands [{:.2}, {:.2}]", atr[n], lower_band, upper_band),
        }
    }
}
