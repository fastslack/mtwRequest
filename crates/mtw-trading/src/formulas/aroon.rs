use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Aroon — identifies trend start/end by measuring time since highs/lows.
pub struct AroonFormula;

impl SignalFormula for AroonFormula {
    fn id(&self) -> &str { "aroon" }
    fn name(&self) -> &str { "Aroon (25)" }
    fn description(&self) -> &str { "Early trend detection: time since highs/lows catches new trends" }
    fn min_candles(&self) -> usize { 30 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(25.0) as usize;
        let n = candles.len();
        let i = n - 1;
        if i < period { return FormulaResult::neutral("insufficient data"); }

        let calc_aroon = |end: usize| -> (f64, f64) {
            let mut high_idx = 0usize;
            let mut low_idx = 0usize;
            let mut max_h = f64::NEG_INFINITY;
            let mut min_l = f64::INFINITY;
            for j in 0..=period {
                let idx = end - period + j;
                if candles[idx].high > max_h { max_h = candles[idx].high; high_idx = j; }
                if candles[idx].low < min_l { min_l = candles[idx].low; low_idx = j; }
            }
            ((high_idx as f64 / period as f64) * 100.0, (low_idx as f64 / period as f64) * 100.0)
        };

        let (aroon_up, aroon_down) = calc_aroon(i);
        let (prev_up, prev_down) = if i > period { calc_aroon(i - 1) } else { (50.0, 50.0) };

        let oscillator = aroon_up - aroon_down;
        let cross_up = prev_up <= prev_down && aroon_up > aroon_down;
        let cross_down = prev_up >= prev_down && aroon_up < aroon_down;
        let strong_up = aroon_up > 70.0 && aroon_down < 30.0;
        let strong_down = aroon_down > 70.0 && aroon_up < 30.0;

        let (side, confidence) = if cross_up || strong_up {
            (Some(OrderSide::Buy), (35.0 + oscillator.abs() * 0.4 + if cross_up { 15.0 } else { 0.0 }).min(75.0))
        } else if cross_down || strong_down {
            (Some(OrderSide::Sell), (35.0 + oscillator.abs() * 0.4 + if cross_down { 15.0 } else { 0.0 }).min(75.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("aroon_up".into(), aroon_up.round());
        indicators.insert("aroon_down".into(), aroon_down.round());
        indicators.insert("oscillator".into(), oscillator.round());

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("Aroon Up:{:.0} Down:{:.0}", aroon_up, aroon_down),
        }
    }
}
