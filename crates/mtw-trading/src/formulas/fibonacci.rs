use std::collections::HashMap;
use crate::formula::{compute_sma, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Fibonacci Retracement — key support/resistance levels from swing high/low.
pub struct FibonacciFormula;

impl SignalFormula for FibonacciFormula {
    fn id(&self) -> &str { "fibonacci" }
    fn name(&self) -> &str { "Fibonacci Retracement" }
    fn description(&self) -> &str { "S/R levels from swing high/low: buy at golden ratio pullbacks" }
    fn min_candles(&self) -> usize { 30 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let lookback = context.and_then(|c| c.params.get("lookback")).copied().unwrap_or(50.0) as usize;
        let n = candles.len();
        let i = n - 1;
        let window = lookback.min(n);

        let mut swing_high = f64::NEG_INFINITY;
        let mut swing_low = f64::INFINITY;
        let mut hi_idx = i;
        let mut lo_idx = i;
        for j in (i + 1 - window)..=i {
            if candles[j].high > swing_high { swing_high = candles[j].high; hi_idx = j; }
            if candles[j].low < swing_low { swing_low = candles[j].low; lo_idx = j; }
        }

        let range = swing_high - swing_low;
        if range <= 0.0 { return FormulaResult::neutral("no price range"); }

        let fib_levels: [(f64, f64); 5] = [
            (0.236, swing_high - range * 0.236),
            (0.382, swing_high - range * 0.382),
            (0.500, swing_high - range * 0.500),
            (0.618, swing_high - range * 0.618),
            (0.786, swing_high - range * 0.786),
        ];

        let price = candles[i].close;
        let uptrend = hi_idx > lo_idx;

        // Find nearest fib level
        let (nearest_level, nearest_price) = fib_levels.iter()
            .min_by(|a, b| (price - a.1).abs().partial_cmp(&(price - b.1).abs()).unwrap())
            .map(|&(l, p)| (l, p)).unwrap();

        let proximity = ((price - nearest_price).abs() / price) * 100.0;
        let at_fib = proximity < 1.5;
        let golden_zone = nearest_level >= 0.382 && nearest_level <= 0.618;

        // Detect trend via SMA slope
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let sma20 = compute_sma(&closes, 20);
        let trend_up = sma20.len() > 5 && sma20[i] > sma20[i.saturating_sub(5)];

        let (side, confidence) = if at_fib && uptrend && price <= nearest_price && trend_up {
            (Some(OrderSide::Buy), (35.0 + if golden_zone { 25.0 } else { 10.0 } + (1.5 - proximity) * 15.0).min(75.0))
        } else if at_fib && !uptrend && price >= nearest_price && !trend_up {
            (Some(OrderSide::Sell), (35.0 + if golden_zone { 25.0 } else { 10.0 } + (1.5 - proximity) * 15.0).min(75.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("nearest_level".into(), nearest_level);
        indicators.insert("proximity".into(), (proximity * 100.0).round() / 100.0);
        indicators.insert("swing_high".into(), (swing_high * 100.0).round() / 100.0);
        indicators.insert("swing_low".into(), (swing_low * 100.0).round() / 100.0);
        for &(l, p) in &fib_levels {
            indicators.insert(format!("fib_{}", (l * 1000.0) as u32), (p * 100.0).round() / 100.0);
        }

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("Fib {:.1}% (proximity:{:.1}%{})", nearest_level * 100.0, proximity,
                if golden_zone { " golden zone" } else { "" }),
        }
    }
}
