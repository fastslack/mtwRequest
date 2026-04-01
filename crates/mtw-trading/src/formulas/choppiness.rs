use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Choppiness Index — measures if market is trending or choppy.
pub struct ChoppinessFormula;

impl SignalFormula for ChoppinessFormula {
    fn id(&self) -> &str { "choppiness" }
    fn name(&self) -> &str { "Choppiness Index" }
    fn description(&self) -> &str { "Trend quality filter: high = ranging (avoid), low = trending (trade)" }
    fn min_candles(&self) -> usize { 20 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(14.0) as usize;
        let i = candles.len() - 1;
        if i < period { return FormulaResult::neutral("insufficient data"); }

        let mut atr_sum = 0.0_f64;
        let mut hh = f64::NEG_INFINITY;
        let mut ll = f64::INFINITY;
        for j in (i + 1 - period)..=i {
            let tr = (candles[j].high - candles[j].low)
                .max((candles[j].high - candles[j - 1].close).abs())
                .max((candles[j].low - candles[j - 1].close).abs());
            atr_sum += tr;
            if candles[j].high > hh { hh = candles[j].high; }
            if candles[j].low < ll { ll = candles[j].low; }
        }

        let range = hh - ll;
        let ci = if range > 0.0 { 100.0 * (atr_sum / range).log10() / (period as f64).log10() } else { 50.0 };

        let choppy = ci > 61.8;
        let trending = ci < 38.2;
        let direction = if candles[i].close > candles[i - period].close { "up" } else { "down" };

        let (side, confidence) = if trending && direction == "up" {
            (Some(OrderSide::Buy), (35.0 + (38.2 - ci) * 2.0).min(70.0))
        } else if trending && direction == "down" {
            (Some(OrderSide::Sell), (35.0 + (38.2 - ci) * 2.0).min(70.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("ci".into(), (ci * 10.0).round() / 10.0);
        indicators.insert("choppy".into(), if choppy { 1.0 } else { 0.0 });
        indicators.insert("trending".into(), if trending { 1.0 } else { 0.0 });

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: if choppy { format!("Choppy (CI:{:.0} > 61.8)", ci) }
                       else if trending { format!("Trending {} (CI:{:.0})", direction, ci) }
                       else { format!("Moderate CI:{:.0}", ci) },
        }
    }
}
