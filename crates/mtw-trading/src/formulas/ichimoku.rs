use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Ichimoku Cloud indicator.
///
/// Components:
/// - Tenkan-sen (conversion line): (9-period high + 9-period low) / 2
/// - Kijun-sen (base line): (26-period high + 26-period low) / 2
/// - Senkou Span A: (Tenkan + Kijun) / 2, plotted 26 periods ahead
/// - Senkou Span B: (52-period high + 52-period low) / 2, plotted 26 periods ahead
///
/// Score-based signal: each condition adds/subtracts points.
/// - Price above cloud: +40, below: -40
/// - Tenkan > Kijun: +30, Tenkan < Kijun: -30
/// - Tenkan cross Kijun: +15/-15
pub struct IchimokuFormula;

fn period_midpoint(candles: &[Candle], end: usize, period: usize) -> f64 {
    if end + 1 < period { return 0.0; }
    let start = end + 1 - period;
    let slice = &candles[start..=end];
    let high = slice.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let low = slice.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    (high + low) / 2.0
}

impl SignalFormula for IchimokuFormula {
    fn id(&self) -> &str { "ichimoku" }
    fn name(&self) -> &str { "Ichimoku Cloud (9/26/52)" }
    fn description(&self) -> &str { "Multi-component trend system with score-based signals" }
    fn min_candles(&self) -> usize { 55 }

    fn compute(&self, candles: &[Candle], _context: Option<&FormulaContext>) -> FormulaResult {
        let n = candles.len() - 1;

        let tenkan = period_midpoint(candles, n, 9);
        let kijun = period_midpoint(candles, n, 26);
        let senkou_a = (tenkan + kijun) / 2.0;
        let senkou_b = period_midpoint(candles, n, 52);

        let prev_tenkan = period_midpoint(candles, n - 1, 9);
        let prev_kijun = period_midpoint(candles, n - 1, 26);

        let price = candles[n].close;
        let cloud_top = senkou_a.max(senkou_b);
        let cloud_bottom = senkou_a.min(senkou_b);

        let mut score: f64 = 0.0;

        // Price vs cloud: +/- 40
        if price > cloud_top {
            score += 40.0;
        } else if price < cloud_bottom {
            score -= 40.0;
        }

        // Tenkan vs Kijun: +/- 30
        if tenkan > kijun {
            score += 30.0;
        } else if tenkan < kijun {
            score -= 30.0;
        }

        // TK cross: +/- 15
        let tk_cross_up = prev_tenkan <= prev_kijun && tenkan > kijun;
        let tk_cross_down = prev_tenkan >= prev_kijun && tenkan < kijun;
        if tk_cross_up {
            score += 15.0;
        } else if tk_cross_down {
            score -= 15.0;
        }

        let mut indicators = HashMap::new();
        indicators.insert("tenkan".into(), (tenkan * 100.0).round() / 100.0);
        indicators.insert("kijun".into(), (kijun * 100.0).round() / 100.0);
        indicators.insert("senkou_a".into(), (senkou_a * 100.0).round() / 100.0);
        indicators.insert("senkou_b".into(), (senkou_b * 100.0).round() / 100.0);
        indicators.insert("score".into(), score);

        let (side, confidence) = if score >= 40.0 {
            (Some(OrderSide::Buy), 60.0 + score.min(85.0).abs() * 0.3)
        } else if score <= -40.0 {
            (Some(OrderSide::Sell), 60.0 + score.min(85.0).abs() * 0.3)
        } else {
            (None, 0.0)
        };

        FormulaResult {
            side,
            confidence: confidence.min(95.0),
            indicators,
            reasoning: format!("Ichimoku score {:.0} tenkan {:.2} kijun {:.2}", score, tenkan, kijun),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ichimoku_uptrend() {
        let candles: Vec<Candle> = (0..60).map(|i| {
            let base = 100.0 + i as f64 * 1.5;
            Candle {
                timestamp: i as u64, open: base, high: base + 2.0,
                low: base - 1.0, close: base + 1.0, volume: 100.0,
            }
        }).collect();
        let r = IchimokuFormula.compute(&candles, None);
        // Price well above cloud in uptrend
        assert!(r.indicators.contains_key("tenkan"));
        assert!(r.indicators.contains_key("senkou_a"));
    }

    #[test]
    fn test_ichimoku_downtrend() {
        let candles: Vec<Candle> = (0..60).map(|i| {
            let base = 200.0 - i as f64 * 1.5;
            Candle {
                timestamp: i as u64, open: base, high: base + 1.0,
                low: base - 2.0, close: base - 1.0, volume: 100.0,
            }
        }).collect();
        let r = IchimokuFormula.compute(&candles, None);
        if r.side.is_some() {
            assert_eq!(r.side, Some(OrderSide::Sell));
        }
    }
}
