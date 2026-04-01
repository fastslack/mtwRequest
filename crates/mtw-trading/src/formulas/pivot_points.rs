use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Classic + Fibonacci Pivot Points — daily levels used by institutional traders.
pub struct PivotPointsFormula;

impl SignalFormula for PivotPointsFormula {
    fn id(&self) -> &str { "pivot_points" }
    fn name(&self) -> &str { "Pivot Points" }
    fn description(&self) -> &str { "Institutional daily levels: classic + Fibonacci pivots as S/R" }
    fn min_candles(&self) -> usize { 15 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let n = candles.len();
        let i = n - 1;
        let lookback = context.and_then(|c| c.params.get("lookback")).copied().unwrap_or(24.0) as usize;
        let lookback = lookback.min(i);

        let close = candles[i - 1].close;
        let mut high = f64::NEG_INFINITY;
        let mut low = f64::INFINITY;
        for j in (i - lookback)..i {
            if candles[j].high > high { high = candles[j].high; }
            if candles[j].low < low { low = candles[j].low; }
        }

        let pp = (high + low + close) / 3.0;
        let r1 = 2.0 * pp - low;
        let s1 = 2.0 * pp - high;
        let r2 = pp + (high - low);
        let s2 = pp - (high - low);
        let range = high - low;
        let fr1 = pp + range * 0.382;
        let fs1 = pp - range * 0.382;
        let fr2 = pp + range * 0.618;
        let fs2 = pp - range * 0.618;

        let price = candles[i].close;
        let mut levels = vec![
            ("S2", s2), ("FS2", fs2), ("S1", s1), ("FS1", fs1), ("PP", pp),
            ("FR1", fr1), ("R1", r1), ("FR2", fr2), ("R2", r2),
        ];
        levels.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        let mut below = levels[0];
        let mut above = levels[levels.len() - 1];
        for w in levels.windows(2) {
            if price >= w[0].1 && price < w[1].1 {
                below = w[0];
                above = w[1];
                break;
            }
        }

        let dist_below = if price > 0.0 { (price - below.1) / price * 100.0 } else { 0.0 };
        let dist_above = if price > 0.0 { (above.1 - price) / price * 100.0 } else { 0.0 };

        let (side, confidence) = if dist_below < 0.5 && (below.0.starts_with('S') || below.0.starts_with("FS")) {
            (Some(OrderSide::Buy), (30.0 + (0.5 - dist_below) * 60.0).min(65.0))
        } else if dist_above < 0.5 && (above.0.starts_with('R') || above.0.starts_with("FR")) {
            (Some(OrderSide::Sell), (30.0 + (0.5 - dist_above) * 60.0).min(65.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("pp".into(), (pp * 100.0).round() / 100.0);
        indicators.insert("r1".into(), (r1 * 100.0).round() / 100.0);
        indicators.insert("s1".into(), (s1 * 100.0).round() / 100.0);
        indicators.insert("r2".into(), (r2 * 100.0).round() / 100.0);
        indicators.insert("s2".into(), (s2 * 100.0).round() / 100.0);

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("Between {} ({:.2}) and {} ({:.2})", below.0, below.1, above.0, above.1),
        }
    }
}
