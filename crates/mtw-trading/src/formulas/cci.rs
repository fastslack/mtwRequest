use std::collections::HashMap;
use crate::formula::{compute_sma, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// CCI (Commodity Channel Index) — measures price deviation from statistical mean.
pub struct CciFormula;

impl SignalFormula for CciFormula {
    fn id(&self) -> &str { "cci" }
    fn name(&self) -> &str { "CCI (20)" }
    fn description(&self) -> &str { "Cycle detection: deviation from mean, trend shifts at +/-100" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(20.0) as usize;
        let n = candles.len();
        let i = n - 1;

        let tp: Vec<f64> = candles.iter().map(|c| (c.high + c.low + c.close) / 3.0).collect();
        let tp_sma = compute_sma(&tp, period);
        if tp_sma.is_empty() || tp_sma[i] == 0.0 { return FormulaResult::neutral("insufficient data"); }

        let mean_dev: f64 = tp[(i + 1 - period)..=i].iter().map(|v| (v - tp_sma[i]).abs()).sum::<f64>() / period as f64;
        let cci = if mean_dev > 0.0 { (tp[i] - tp_sma[i]) / (0.015 * mean_dev) } else { 0.0 };

        // Previous CCI
        let prev_cci = if i >= 1 && i >= period && tp_sma[i - 1] != 0.0 {
            let prev_md: f64 = tp[(i - period)..i].iter().map(|v| (v - tp_sma[i - 1]).abs()).sum::<f64>() / period as f64;
            if prev_md > 0.0 { (tp[i - 1] - tp_sma[i - 1]) / (0.015 * prev_md) } else { 0.0 }
        } else { 0.0 };

        let overbought = cci > 100.0;
        let oversold = cci < -100.0;
        let cross_above_100 = prev_cci <= 100.0 && cci > 100.0;
        let cross_below_100 = prev_cci >= -100.0 && cci < -100.0;
        let cross_above_zero = prev_cci <= 0.0 && cci > 0.0;
        let cross_below_zero = prev_cci >= 0.0 && cci < 0.0;

        let (side, confidence) = if cross_below_100 || (oversold && cci > prev_cci) {
            (Some(OrderSide::Buy), (40.0 + (cci + 100.0).abs() * 0.3).min(75.0))
        } else if cross_above_100 || (overbought && cci < prev_cci) {
            (Some(OrderSide::Sell), (40.0 + (cci - 100.0).abs() * 0.3).min(75.0))
        } else if cross_above_zero {
            (Some(OrderSide::Buy), (30.0 + cci.abs() * 0.3).min(55.0))
        } else if cross_below_zero {
            (Some(OrderSide::Sell), (30.0 + cci.abs() * 0.3).min(55.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("cci".into(), (cci * 10.0).round() / 10.0);
        indicators.insert("prev_cci".into(), (prev_cci * 10.0).round() / 10.0);

        FormulaResult {
            side, confidence: confidence.round(),
            indicators,
            reasoning: format!("CCI {:.0} (prev {:.0})", cci, prev_cci),
        }
    }
}
