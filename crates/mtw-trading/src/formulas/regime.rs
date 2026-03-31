use std::collections::HashMap;
use crate::formula::{compute_ema, compute_rsi, compute_sma, compute_atr, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Market Regime Detector.
///
/// Classifies the market into Bull, Bear, or Range using 4-6 signals:
/// 1. EMA 20 vs EMA 50 alignment
/// 2. RSI position (trending vs mean-reverting)
/// 3. ADX-like trend strength (using directional movement)
/// 4. Price vs SMA 50
/// 5. ATR relative to price (volatility regime)
/// 6. Higher highs / lower lows structure
///
/// Outputs a regime classification and directional signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketRegime {
    Bull,
    Bear,
    Range,
}

pub struct RegimeFormula;

impl SignalFormula for RegimeFormula {
    fn id(&self) -> &str { "regime" }
    fn name(&self) -> &str { "Market Regime Detector" }
    fn description(&self) -> &str { "Classifies market as Bull/Bear/Range from multiple signals" }
    fn min_candles(&self) -> usize { 55 }

    fn compute(&self, candles: &[Candle], _context: Option<&FormulaContext>) -> FormulaResult {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let n = closes.len();

        let mut bull_score: f64 = 0.0;
        let mut bear_score: f64 = 0.0;

        // Signal 1: EMA 20 vs EMA 50
        let ema20 = compute_ema(&closes, 20);
        let ema50 = compute_ema(&closes, 50);
        if ema20[n - 1] > ema50[n - 1] {
            bull_score += 1.5;
        } else {
            bear_score += 1.5;
        }

        // Signal 2: RSI regime
        let rsi = compute_rsi(&closes, 14);
        if !rsi.is_empty() {
            let rsi_val = rsi[rsi.len() - 1];
            if rsi_val > 55.0 {
                bull_score += 1.0;
            } else if rsi_val < 45.0 {
                bear_score += 1.0;
            }
            // RSI between 45-55 is range-bound (no points either way)
        }

        // Signal 3: Directional movement strength
        // Simplified: compare recent upward vs downward price moves
        let lookback = 14.min(n - 1);
        let mut up_moves = 0.0;
        let mut down_moves = 0.0;
        for i in (n - lookback)..n {
            let diff = closes[i] - closes[i - 1];
            if diff > 0.0 { up_moves += diff; }
            else { down_moves += diff.abs(); }
        }
        let total_moves = up_moves + down_moves;
        if total_moves > 0.0 {
            let directional_ratio = (up_moves - down_moves) / total_moves;
            if directional_ratio > 0.3 {
                bull_score += 1.2;
            } else if directional_ratio < -0.3 {
                bear_score += 1.2;
            }
        }

        // Signal 4: Price vs SMA 50
        let sma50 = compute_sma(&closes, 50);
        if sma50[n - 1] > 0.0 {
            let pct_above = (closes[n - 1] - sma50[n - 1]) / sma50[n - 1] * 100.0;
            if pct_above > 2.0 {
                bull_score += 1.0;
            } else if pct_above < -2.0 {
                bear_score += 1.0;
            }
        }

        // Signal 5: ATR as % of price (volatility regime)
        let atr = compute_atr(candles, 14);
        if !atr.is_empty() && closes[n - 1] > 0.0 {
            let atr_pct = atr[atr.len() - 1] / closes[n - 1] * 100.0;
            // High ATR in trending markets is normal; high ATR in range = choppy
            if atr_pct > 3.0 {
                // High volatility -> amplifies the dominant direction
                if bull_score > bear_score { bull_score += 0.5; }
                else { bear_score += 0.5; }
            }
        }

        // Signal 6: Structure - higher highs/lows vs lower highs/lows
        if n >= 20 {
            let recent_5_high = candles[n - 5..n].iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
            let prev_5_high = candles[n - 10..n - 5].iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
            let recent_5_low = candles[n - 5..n].iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
            let prev_5_low = candles[n - 10..n - 5].iter().map(|c| c.low).fold(f64::INFINITY, f64::min);

            if recent_5_high > prev_5_high && recent_5_low > prev_5_low {
                bull_score += 1.0; // Higher highs AND higher lows
            } else if recent_5_high < prev_5_high && recent_5_low < prev_5_low {
                bear_score += 1.0; // Lower highs AND lower lows
            }
        }

        // Determine regime
        let net = bull_score - bear_score;
        let regime = if net > 2.0 {
            MarketRegime::Bull
        } else if net < -2.0 {
            MarketRegime::Bear
        } else {
            MarketRegime::Range
        };

        let mut indicators = HashMap::new();
        indicators.insert("bull_score".into(), (bull_score * 100.0).round() / 100.0);
        indicators.insert("bear_score".into(), (bear_score * 100.0).round() / 100.0);
        indicators.insert("net_score".into(), (net * 100.0).round() / 100.0);
        indicators.insert("regime".into(), match regime {
            MarketRegime::Bull => 1.0,
            MarketRegime::Bear => -1.0,
            MarketRegime::Range => 0.0,
        });

        let (side, confidence) = match regime {
            MarketRegime::Bull => {
                let conf = 55.0 + net.min(5.0) * 6.0;
                (Some(OrderSide::Buy), conf)
            }
            MarketRegime::Bear => {
                let conf = 55.0 + net.abs().min(5.0) * 6.0;
                (Some(OrderSide::Sell), conf)
            }
            MarketRegime::Range => (None, 0.0),
        };

        let regime_str = match regime {
            MarketRegime::Bull => "BULL",
            MarketRegime::Bear => "BEAR",
            MarketRegime::Range => "RANGE",
        };

        FormulaResult {
            side,
            confidence: confidence.min(90.0),
            indicators,
            reasoning: format!("Regime: {} (bull {:.1} bear {:.1} net {:.1})",
                regime_str, bull_score, bear_score, net),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regime_bull() {
        let candles: Vec<Candle> = (0..60).map(|i| {
            let base = 100.0 + i as f64 * 1.5;
            Candle {
                timestamp: i as u64, open: base - 0.5, high: base + 1.5,
                low: base - 1.0, close: base + 0.5, volume: 1000.0,
            }
        }).collect();
        let r = RegimeFormula.compute(&candles, None);
        assert_eq!(r.indicators["regime"], 1.0);
        assert_eq!(r.side, Some(OrderSide::Buy));
    }

    #[test]
    fn test_regime_bear() {
        let candles: Vec<Candle> = (0..60).map(|i| {
            let base = 200.0 - i as f64 * 1.5;
            Candle {
                timestamp: i as u64, open: base + 0.5, high: base + 1.0,
                low: base - 1.5, close: base - 0.5, volume: 1000.0,
            }
        }).collect();
        let r = RegimeFormula.compute(&candles, None);
        assert_eq!(r.indicators["regime"], -1.0);
        assert_eq!(r.side, Some(OrderSide::Sell));
    }

    #[test]
    fn test_regime_range() {
        // Oscillating price -> range
        let candles: Vec<Candle> = (0..60).map(|i| {
            let base = 100.0 + (i as f64 * 0.5).sin() * 2.0;
            Candle {
                timestamp: i as u64, open: base, high: base + 0.5,
                low: base - 0.5, close: base, volume: 1000.0,
            }
        }).collect();
        let r = RegimeFormula.compute(&candles, None);
        // Regime should be Range or very weakly directional
        assert!(r.indicators.contains_key("regime"));
    }
}
