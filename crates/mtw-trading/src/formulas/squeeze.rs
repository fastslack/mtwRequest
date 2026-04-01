use std::collections::HashMap;
use crate::formula::{compute_sma, compute_ema, compute_atr, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// TTM Squeeze Momentum — Bollinger inside Keltner = squeeze, release = explosive move.
pub struct SqueezeFormula;

fn stddev_at(data: &[f64], sma: &[f64], period: usize, idx: usize) -> f64 {
    if idx < period - 1 { return 0.0; }
    let mean = sma[idx];
    let var: f64 = data[(idx + 1 - period)..=idx].iter().map(|x| (x - mean).powi(2)).sum::<f64>() / period as f64;
    var.sqrt()
}

impl SignalFormula for SqueezeFormula {
    fn id(&self) -> &str { "squeeze" }
    fn name(&self) -> &str { "Squeeze Momentum" }
    fn description(&self) -> &str { "TTM Squeeze: Bollinger inside Keltner detects coiling, release = explosion" }
    fn min_candles(&self) -> usize { 30 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let len = context.and_then(|c| c.params.get("period")).copied().unwrap_or(20.0) as usize;
        let bb_mult = context.and_then(|c| c.params.get("bb_mult")).copied().unwrap_or(2.0);
        let kc_mult = context.and_then(|c| c.params.get("kc_mult")).copied().unwrap_or(1.5);
        let atr_len = context.and_then(|c| c.params.get("atr_period")).copied().unwrap_or(10.0) as usize;

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let sma = compute_sma(&closes, len);
        let ema = compute_ema(&closes, len);
        let atr = compute_atr(candles, atr_len);
        let i = closes.len() - 1;

        if sma.is_empty() || sma[i] == 0.0 || ema[i] == 0.0 || atr[i] == 0.0 {
            return FormulaResult::neutral("insufficient data");
        }

        let is_squeeze = |idx: usize| -> bool {
            let sd = stddev_at(&closes, &sma, len, idx);
            if sd == 0.0 || ema[idx] == 0.0 || atr[idx] == 0.0 { return false; }
            let bb_l = sma[idx] - bb_mult * sd;
            let bb_u = sma[idx] + bb_mult * sd;
            let kc_l = ema[idx] - kc_mult * atr[idx];
            let kc_u = ema[idx] + kc_mult * atr[idx];
            bb_l > kc_l && bb_u < kc_u
        };

        let sqz_on = is_squeeze(i);
        let prev_sqz = if i > 0 { is_squeeze(i - 1) } else { false };
        let sqz_fired = prev_sqz && !sqz_on;

        // Count consecutive squeeze bars
        let mut sqz_count = 0u32;
        for j in (0..=i).rev() {
            if is_squeeze(j) { sqz_count += 1; } else { break; }
        }

        let momentum = closes[i] - sma[i];
        let prev_mom = if i > 0 && sma[i - 1] != 0.0 { closes[i - 1] - sma[i - 1] } else { 0.0 };
        let mom_rising = momentum > prev_mom;

        let (side, confidence) = if sqz_fired {
            let s = if momentum > 0.0 { Some(OrderSide::Buy) } else { Some(OrderSide::Sell) };
            (s, (50.0 + sqz_count as f64 * 3.0 + (momentum / closes[i]).abs() * 500.0).min(85.0))
        } else if sqz_on && sqz_count >= 4 {
            if mom_rising && momentum > 0.0 {
                (Some(OrderSide::Buy), (30.0 + sqz_count as f64 * 2.0).min(60.0))
            } else if !mom_rising && momentum < 0.0 {
                (Some(OrderSide::Sell), (30.0 + sqz_count as f64 * 2.0).min(60.0))
            } else { (None, 0.0) }
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("sqz_on".into(), if sqz_on { 1.0 } else { 0.0 });
        indicators.insert("sqz_fired".into(), if sqz_fired { 1.0 } else { 0.0 });
        indicators.insert("sqz_count".into(), sqz_count as f64);
        indicators.insert("momentum".into(), (momentum / closes[i] * 100.0 * 1000.0).round() / 1000.0);

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: if sqz_fired { format!("SQUEEZE FIRED after {} bars — {}", sqz_count, if momentum > 0.0 { "bullish" } else { "bearish" }) }
                       else if sqz_on { format!("Squeeze active ({} bars)", sqz_count) }
                       else { format!("No squeeze (mom:{:.2}%)", momentum / closes[i] * 100.0) },
        }
    }
}
