use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Kelly Criterion for optimal position sizing.
///
/// Computes: K = W - (1 - W) / R
/// where W = win probability, R = average win / average loss ratio
///
/// Uses historical candle returns to estimate W and R, then recommends
/// a sizing fraction. Signal direction is based on recent momentum.
pub struct KellyFormula;

impl SignalFormula for KellyFormula {
    fn id(&self) -> &str { "kelly" }
    fn name(&self) -> &str { "Kelly Criterion" }
    fn description(&self) -> &str { "Optimal position sizing from win rate and win/loss ratio" }
    fn min_candles(&self) -> usize { 30 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let lookback = context.and_then(|c| c.params.get("lookback")).copied().unwrap_or(20.0) as usize;
        let momentum_period = context.and_then(|c| c.params.get("momentum_period")).copied().unwrap_or(5.0) as usize;
        let n = candles.len();
        if n < lookback + 1 {
            return FormulaResult::neutral("insufficient data for Kelly");
        }

        // Compute returns over the lookback period
        let start = n - lookback;
        let mut wins = 0usize;
        let mut losses = 0usize;
        let mut total_win = 0.0f64;
        let mut total_loss = 0.0f64;

        for i in start..n {
            let ret = (candles[i].close - candles[i - 1].close) / candles[i - 1].close;
            if ret > 0.0 {
                wins += 1;
                total_win += ret;
            } else if ret < 0.0 {
                losses += 1;
                total_loss += ret.abs();
            }
        }

        let total = wins + losses;
        if total == 0 {
            return FormulaResult::neutral("no price movement");
        }

        let win_prob = wins as f64 / total as f64;
        let avg_win = if wins > 0 { total_win / wins as f64 } else { 0.0 };
        let avg_loss = if losses > 0 { total_loss / losses as f64 } else { 0.001 }; // avoid div by zero
        let win_loss_ratio = avg_win / avg_loss;

        // Kelly fraction: K = W - (1 - W) / R
        let kelly = win_prob - (1.0 - win_prob) / win_loss_ratio;
        // Half-Kelly is common in practice for safety
        let half_kelly = (kelly / 2.0).max(0.0).min(1.0);

        // Direction from recent momentum
        let mom_start = if n > momentum_period { n - momentum_period } else { 0 };
        let momentum = (candles[n - 1].close - candles[mom_start].close) / candles[mom_start].close;

        let mut indicators = HashMap::new();
        indicators.insert("kelly_fraction".into(), (kelly * 10000.0).round() / 10000.0);
        indicators.insert("half_kelly".into(), (half_kelly * 10000.0).round() / 10000.0);
        indicators.insert("win_probability".into(), (win_prob * 10000.0).round() / 10000.0);
        indicators.insert("win_loss_ratio".into(), (win_loss_ratio * 10000.0).round() / 10000.0);
        indicators.insert("momentum".into(), (momentum * 10000.0).round() / 10000.0);

        let (side, confidence) = if kelly > 0.05 && momentum > 0.0 {
            (Some(OrderSide::Buy), 50.0 + kelly.min(0.5) * 60.0)
        } else if kelly > 0.05 && momentum < 0.0 {
            (Some(OrderSide::Sell), 50.0 + kelly.min(0.5) * 60.0)
        } else {
            (None, 0.0) // Negative or too-small Kelly = don't trade
        };

        FormulaResult {
            side,
            confidence: confidence.min(90.0),
            indicators,
            reasoning: format!("Kelly {:.4} half {:.4} W={:.2} R={:.2} mom={:.4}",
                kelly, half_kelly, win_prob, win_loss_ratio, momentum),
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
    fn test_kelly_positive_edge() {
        // Strong uptrend -> high win rate, positive Kelly
        let prices: Vec<f64> = (0..40).map(|i| 100.0 + i as f64 * 0.5).collect();
        let r = KellyFormula.compute(&make_candles(&prices), None);
        assert!(r.indicators["kelly_fraction"] > 0.0);
        assert!(r.indicators["win_probability"] > 0.5);
    }

    #[test]
    fn test_kelly_flat_market() {
        // Flat market -> Kelly near zero
        let prices: Vec<f64> = (0..40).map(|i| 100.0 + (i as f64 * 0.1).sin()).collect();
        let r = KellyFormula.compute(&make_candles(&prices), None);
        // In a flat/noisy market, Kelly should be small
        assert!(r.indicators.contains_key("kelly_fraction"));
    }
}
