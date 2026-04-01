use std::collections::HashMap;
use crate::formula::{compute_ema, compute_sma, compute_atr, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Keltner Channels — EMA ± ATR multiplier with Bollinger squeeze detection.
pub struct KeltnerFormula;

fn stddev(data: &[f64], period: usize) -> Vec<f64> {
    let sma = compute_sma(data, period);
    let mut sd = vec![0.0; data.len()];
    for i in (period - 1)..data.len() {
        let mean = sma[i];
        let var: f64 = data[(i + 1 - period)..=i].iter().map(|x| (x - mean).powi(2)).sum::<f64>() / period as f64;
        sd[i] = var.sqrt();
    }
    sd
}

impl SignalFormula for KeltnerFormula {
    fn id(&self) -> &str { "keltner" }
    fn name(&self) -> &str { "Keltner Channels" }
    fn description(&self) -> &str { "ATR-based channels with Bollinger squeeze detection" }
    fn min_candles(&self) -> usize { 30 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let ema_period = context.and_then(|c| c.params.get("ema_period")).copied().unwrap_or(20.0) as usize;
        let atr_period = context.and_then(|c| c.params.get("atr_period")).copied().unwrap_or(10.0) as usize;
        let kc_mult = context.and_then(|c| c.params.get("kc_mult")).copied().unwrap_or(1.5);
        let bb_period = context.and_then(|c| c.params.get("bb_period")).copied().unwrap_or(20.0) as usize;
        let bb_mult = context.and_then(|c| c.params.get("bb_mult")).copied().unwrap_or(2.0);

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let ema = compute_ema(&closes, ema_period);
        let atr = compute_atr(candles, atr_period);
        let sd = stddev(&closes, bb_period);
        let i = closes.len() - 1;

        if ema[i] == 0.0 || atr[i] == 0.0 || sd[i] == 0.0 {
            return FormulaResult::neutral("insufficient data");
        }

        let mid = ema[i];
        let k_upper = mid + kc_mult * atr[i];
        let k_lower = mid - kc_mult * atr[i];
        let bb_upper = mid + bb_mult * sd[i];
        let bb_lower = mid - bb_mult * sd[i];

        let squeeze = bb_lower > k_lower && bb_upper < k_upper;
        let prev_squeeze = if i > 0 && ema[i - 1] != 0.0 && atr[i - 1] != 0.0 && sd[i - 1] != 0.0 {
            let pm = ema[i - 1];
            (pm - bb_mult * sd[i - 1]) > (pm - kc_mult * atr[i - 1]) &&
            (pm + bb_mult * sd[i - 1]) < (pm + kc_mult * atr[i - 1])
        } else { false };
        let squeeze_release = prev_squeeze && !squeeze;

        let momentum = if i >= 3 && ema[i - 3] != 0.0 {
            ((ema[i] - ema[i - 3]) / ema[i - 3]) * 100.0
        } else { 0.0 };
        let bullish = momentum > 0.0;
        let price = closes[i];

        let (side, confidence) = if squeeze_release {
            (if bullish { Some(OrderSide::Buy) } else { Some(OrderSide::Sell) },
             (55.0 + momentum.abs() * 15.0).min(85.0))
        } else if price > k_upper && bullish {
            (Some(OrderSide::Buy), (40.0 + ((price - k_upper) / k_upper) * 500.0).min(70.0))
        } else if price < k_lower && !bullish {
            (Some(OrderSide::Sell), (40.0 + ((k_lower - price) / k_lower) * 500.0).min(70.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("k_upper".into(), (k_upper * 100.0).round() / 100.0);
        indicators.insert("k_lower".into(), (k_lower * 100.0).round() / 100.0);
        indicators.insert("squeeze".into(), if squeeze { 1.0 } else { 0.0 });
        indicators.insert("squeeze_release".into(), if squeeze_release { 1.0 } else { 0.0 });

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: if squeeze_release { format!("Squeeze RELEASED — {} move", if bullish { "bullish" } else { "bearish" }) }
                       else if squeeze { "Volatility squeeze active".into() }
                       else { format!("Keltner momentum:{:.2}%", momentum) },
        }
    }
}
