use std::collections::HashMap;
use crate::formula::{compute_ema, compute_atr, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// ATR Channel — dynamic volatility bands around EMA.
pub struct AtrChannelFormula;

impl SignalFormula for AtrChannelFormula {
    fn id(&self) -> &str { "atr_channel" }
    fn name(&self) -> &str { "ATR Channel" }
    fn description(&self) -> &str { "Volatility-adaptive bands: breakout signals with dynamic SL/TP sizing" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let atr_period = context.and_then(|c| c.params.get("atr_period")).copied().unwrap_or(14.0) as usize;
        let ema_period = context.and_then(|c| c.params.get("ema_period")).copied().unwrap_or(20.0) as usize;
        let mult = context.and_then(|c| c.params.get("mult")).copied().unwrap_or(2.0);

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let atr = compute_atr(candles, atr_period);
        let ema = compute_ema(&closes, ema_period);
        let i = closes.len() - 1;

        if atr[i] == 0.0 || ema[i] == 0.0 {
            return FormulaResult::neutral("insufficient data");
        }

        let price = closes[i];
        let mid = ema[i];
        let upper = mid + mult * atr[i];
        let lower = mid - mult * atr[i];
        let atr_pct = (atr[i] / price) * 100.0;

        let prev_atr = if i >= 5 && atr[i - 5] != 0.0 { atr[i - 5] } else { atr[i] };
        let vol_expanding = atr[i] > prev_atr * 1.1;

        let recent_slope = if i >= 5 && closes[i - 5] != 0.0 {
            (closes[i] - closes[i - 5]) / closes[i - 5] * 100.0
        } else { 0.0 };

        let above_upper = price > upper;
        let below_lower = price < lower;

        let (side, confidence) = if above_upper && recent_slope > 0.0 && vol_expanding {
            (Some(OrderSide::Buy), (45.0 + atr_pct * 5.0 + recent_slope.abs() * 3.0).min(80.0))
        } else if below_lower && recent_slope < 0.0 && vol_expanding {
            (Some(OrderSide::Sell), (45.0 + atr_pct * 5.0 + recent_slope.abs() * 3.0).min(80.0))
        } else if above_upper && recent_slope > 0.0 {
            (Some(OrderSide::Buy), (35.0 + atr_pct * 3.0).min(65.0))
        } else if below_lower && recent_slope < 0.0 {
            (Some(OrderSide::Sell), (35.0 + atr_pct * 3.0).min(65.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("atr_pct".into(), (atr_pct * 100.0).round() / 100.0);
        indicators.insert("upper".into(), (upper * 100.0).round() / 100.0);
        indicators.insert("lower".into(), (lower * 100.0).round() / 100.0);
        indicators.insert("vol_expanding".into(), if vol_expanding { 1.0 } else { 0.0 });

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("ATR Channel {:.1}%{}", atr_pct, if vol_expanding { " vol expanding" } else { "" }),
        }
    }
}
