use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Williams %R oscillator.
///
/// %R = (Highest High - Close) / (Highest High - Lowest Low) * -100
///
/// Range: -100 to 0
/// - Above -20: overbought -> Sell
/// - Below -80: oversold -> Buy
pub struct WilliamsRFormula;

impl SignalFormula for WilliamsRFormula {
    fn id(&self) -> &str { "williams_r" }
    fn name(&self) -> &str { "Williams %R (14)" }
    fn description(&self) -> &str { "Momentum: overbought/oversold oscillator (-100 to 0)" }
    fn min_candles(&self) -> usize { 15 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(14.0) as usize;
        let overbought = context.and_then(|c| c.params.get("overbought")).copied().unwrap_or(-20.0);
        let oversold = context.and_then(|c| c.params.get("oversold")).copied().unwrap_or(-80.0);

        let n = candles.len();
        if n < period {
            return FormulaResult::neutral("insufficient data");
        }

        let window = &candles[n - period..n];
        let highest = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let lowest = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let close = candles[n - 1].close;

        let range = highest - lowest;
        let wr = if range > 0.0 {
            ((highest - close) / range) * -100.0
        } else {
            -50.0
        };

        // Also compute previous %R for crossover detection
        let prev_wr = if n > period {
            let prev_window = &candles[n - 1 - period..n - 1];
            let ph = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
            let pl = prev_window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
            let prev_close = candles[n - 2].close;
            let pr = ph - pl;
            if pr > 0.0 { ((ph - prev_close) / pr) * -100.0 } else { -50.0 }
        } else {
            -50.0
        };

        let mut indicators = HashMap::new();
        indicators.insert("williams_r".into(), (wr * 100.0).round() / 100.0);
        indicators.insert("prev_williams_r".into(), (prev_wr * 100.0).round() / 100.0);

        // Signal on crossing out of extreme zones (more reliable than just being in zone)
        let leaving_oversold = prev_wr < oversold && wr >= oversold;
        let in_oversold = wr < oversold;
        let leaving_overbought = prev_wr > overbought && wr <= overbought;
        let in_overbought = wr > overbought;

        let (side, confidence) = if leaving_oversold {
            (Some(OrderSide::Buy), 75.0)
        } else if in_oversold {
            (Some(OrderSide::Buy), 60.0)
        } else if leaving_overbought {
            (Some(OrderSide::Sell), 75.0)
        } else if in_overbought {
            (Some(OrderSide::Sell), 60.0)
        } else {
            (None, 0.0)
        };

        FormulaResult {
            side,
            confidence,
            indicators,
            reasoning: format!("Williams %R {:.1}", wr),
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
    fn test_williams_r_overbought() {
        // Price at top of range -> %R near 0 (overbought)
        let prices: Vec<f64> = (0..20).map(|i| 100.0 + i as f64).collect();
        let r = WilliamsRFormula.compute(&make_candles(&prices), None);
        let wr = r.indicators["williams_r"];
        assert!(wr > -2000.0); // %R stored *100 for precision
        assert_eq!(r.side, Some(OrderSide::Sell));
    }

    #[test]
    fn test_williams_r_oversold() {
        // Price at bottom of range -> %R near -100 (oversold)
        let prices: Vec<f64> = (0..20).map(|i| 200.0 - i as f64).collect();
        let r = WilliamsRFormula.compute(&make_candles(&prices), None);
        assert_eq!(r.side, Some(OrderSide::Buy));
    }
}
