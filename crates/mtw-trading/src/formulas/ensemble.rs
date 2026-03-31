use std::collections::HashMap;
use crate::formula::{compute_ema, compute_rsi, compute_sma, compute_atr, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Ensemble Vote: a meta-formula that combines 12 micro-rules with weights.
///
/// Each micro-rule produces a vote in [-1, +1] (sell to buy), multiplied by its weight.
/// The final score is the weighted sum, normalized. This gives a consensus signal
/// from multiple independent technical perspectives.
pub struct EnsembleFormula;

struct MicroRule {
    name: &'static str,
    weight: f64,
}

impl EnsembleFormula {
    fn rules() -> Vec<MicroRule> {
        vec![
            MicroRule { name: "ema_trend_9_21", weight: 1.0 },
            MicroRule { name: "ema_trend_21_50", weight: 1.2 },
            MicroRule { name: "rsi_14", weight: 1.0 },
            MicroRule { name: "rsi_7", weight: 0.8 },
            MicroRule { name: "macd_hist", weight: 1.1 },
            MicroRule { name: "price_vs_sma50", weight: 1.0 },
            MicroRule { name: "price_vs_sma200", weight: 1.3 },
            MicroRule { name: "volume_surge", weight: 0.7 },
            MicroRule { name: "higher_highs", weight: 0.9 },
            MicroRule { name: "higher_lows", weight: 0.9 },
            MicroRule { name: "atr_expansion", weight: 0.6 },
            MicroRule { name: "close_position", weight: 0.5 },
        ]
    }

    fn evaluate_rule(name: &str, candles: &[Candle], closes: &[f64]) -> f64 {
        let n = closes.len();
        if n < 2 { return 0.0; }

        match name {
            "ema_trend_9_21" => {
                let ema9 = compute_ema(closes, 9);
                let ema21 = compute_ema(closes, 21);
                if ema9[n - 1] > ema21[n - 1] { 1.0 } else { -1.0 }
            }
            "ema_trend_21_50" => {
                if n < 50 { return 0.0; }
                let ema21 = compute_ema(closes, 21);
                let ema50 = compute_ema(closes, 50);
                if ema21[n - 1] > ema50[n - 1] { 1.0 } else { -1.0 }
            }
            "rsi_14" => {
                let rsi = compute_rsi(closes, 14);
                if rsi.is_empty() { return 0.0; }
                let val = rsi[rsi.len() - 1];
                if val < 30.0 { 1.0 }
                else if val > 70.0 { -1.0 }
                else { (50.0 - val) / 50.0 } // linear scale
            }
            "rsi_7" => {
                let rsi = compute_rsi(closes, 7);
                if rsi.is_empty() { return 0.0; }
                let val = rsi[rsi.len() - 1];
                if val < 25.0 { 1.0 }
                else if val > 75.0 { -1.0 }
                else { (50.0 - val) / 50.0 }
            }
            "macd_hist" => {
                if n < 35 { return 0.0; }
                let ema12 = compute_ema(closes, 12);
                let ema26 = compute_ema(closes, 26);
                let macd_line: Vec<f64> = ema12.iter().zip(ema26.iter()).map(|(a, b)| a - b).collect();
                let signal = compute_ema(&macd_line, 9);
                let hist = macd_line[n - 1] - signal[n - 1];
                hist.signum()
            }
            "price_vs_sma50" => {
                if n < 50 { return 0.0; }
                let sma = compute_sma(closes, 50);
                if sma[n - 1] == 0.0 { return 0.0; }
                if closes[n - 1] > sma[n - 1] { 1.0 } else { -1.0 }
            }
            "price_vs_sma200" => {
                if n < 200 { return 0.0; }
                let sma = compute_sma(closes, 200);
                if sma[n - 1] == 0.0 { return 0.0; }
                if closes[n - 1] > sma[n - 1] { 1.0 } else { -1.0 }
            }
            "volume_surge" => {
                if n < 20 { return 0.0; }
                let avg_vol: f64 = candles[n - 20..n - 1].iter().map(|c| c.volume).sum::<f64>() / 19.0;
                let cur_vol = candles[n - 1].volume;
                if avg_vol == 0.0 { return 0.0; }
                let ratio = cur_vol / avg_vol;
                if ratio > 2.0 {
                    // Volume surge in direction of price move
                    if closes[n - 1] > closes[n - 2] { 0.8 } else { -0.8 }
                } else {
                    0.0
                }
            }
            "higher_highs" => {
                if n < 10 { return 0.0; }
                let recent_high = candles[n - 5..n].iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let prev_high = candles[n - 10..n - 5].iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                if recent_high > prev_high { 1.0 } else { -1.0 }
            }
            "higher_lows" => {
                if n < 10 { return 0.0; }
                let recent_low = candles[n - 5..n].iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
                let prev_low = candles[n - 10..n - 5].iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
                if recent_low > prev_low { 1.0 } else { -1.0 }
            }
            "atr_expansion" => {
                if n < 20 { return 0.0; }
                let atr = compute_atr(candles, 14);
                if atr.len() < 2 { return 0.0; }
                let cur = atr[atr.len() - 1];
                let prev = atr[atr.len() - 6.min(atr.len())];
                if cur > prev * 1.2 {
                    // ATR expanding -> trend strengthening
                    if closes[n - 1] > closes[n - 6.min(n)] { 0.5 } else { -0.5 }
                } else {
                    0.0
                }
            }
            "close_position" => {
                // Where price closed in the candle range
                let c = &candles[n - 1];
                let range = c.high - c.low;
                if range == 0.0 { return 0.0; }
                let position = (c.close - c.low) / range; // 0..1
                (position - 0.5) * 2.0 // -1..1
            }
            _ => 0.0,
        }
    }
}

impl SignalFormula for EnsembleFormula {
    fn id(&self) -> &str { "ensemble" }
    fn name(&self) -> &str { "Ensemble Vote (12 rules)" }
    fn description(&self) -> &str { "Weighted consensus from 12 technical micro-rules" }
    fn min_candles(&self) -> usize { 55 }

    fn compute(&self, candles: &[Candle], _context: Option<&FormulaContext>) -> FormulaResult {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let rules = Self::rules();

        let mut total_weight = 0.0;
        let mut weighted_sum = 0.0;
        let mut active_rules = 0;
        let mut indicators = HashMap::new();

        for rule in &rules {
            let vote = Self::evaluate_rule(rule.name, candles, &closes);
            if vote != 0.0 {
                active_rules += 1;
            }
            weighted_sum += vote * rule.weight;
            total_weight += rule.weight;
            indicators.insert(format!("rule_{}", rule.name), (vote * 100.0).round() / 100.0);
        }

        let score = if total_weight > 0.0 { weighted_sum / total_weight } else { 0.0 };
        indicators.insert("ensemble_score".into(), (score * 10000.0).round() / 10000.0);
        indicators.insert("active_rules".into(), active_rules as f64);

        // Threshold: score > 0.3 for buy, < -0.3 for sell
        let threshold = 0.3;
        let (side, confidence) = if score > threshold {
            let conf = 50.0 + score.min(1.0) * 40.0;
            (Some(OrderSide::Buy), conf)
        } else if score < -threshold {
            let conf = 50.0 + score.abs().min(1.0) * 40.0;
            (Some(OrderSide::Sell), conf)
        } else {
            (None, 0.0)
        };

        FormulaResult {
            side,
            confidence: confidence.min(95.0),
            indicators,
            reasoning: format!("Ensemble score {:.3} ({} active rules)", score, active_rules),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trending_candles(up: bool, count: usize) -> Vec<Candle> {
        (0..count).map(|i| {
            let base = if up { 100.0 + i as f64 * 1.0 } else { 200.0 - i as f64 * 1.0 };
            Candle {
                timestamp: i as u64, open: base - 0.3, high: base + 1.0,
                low: base - 0.5, close: base + 0.5, volume: 1000.0,
            }
        }).collect()
    }

    #[test]
    fn test_ensemble_uptrend() {
        let candles = trending_candles(true, 60);
        let r = EnsembleFormula.compute(&candles, None);
        // Most rules should agree on Buy in a clean uptrend
        let score = r.indicators["ensemble_score"];
        assert!(score > 0.0, "Expected positive ensemble score, got {}", score);
    }

    #[test]
    fn test_ensemble_downtrend() {
        let candles = trending_candles(false, 60);
        let r = EnsembleFormula.compute(&candles, None);
        let score = r.indicators["ensemble_score"];
        assert!(score < 0.0, "Expected negative ensemble score, got {}", score);
    }
}
