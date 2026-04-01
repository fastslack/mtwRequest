use std::collections::HashMap;
use crate::formula::{compute_ema, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Elder Ray Index — measures buyer vs seller power relative to EMA.
pub struct ElderRayFormula;

impl SignalFormula for ElderRayFormula {
    fn id(&self) -> &str { "elder_ray" }
    fn name(&self) -> &str { "Elder Ray" }
    fn description(&self) -> &str { "Bull/Bear power: buyer vs seller force relative to EMA" }
    fn min_candles(&self) -> usize { 20 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(13.0) as usize;
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let ema = compute_ema(&closes, period);
        let i = closes.len() - 1;

        if i < 1 || ema[i] == 0.0 || ema[i - 1] == 0.0 {
            return FormulaResult::neutral("insufficient data");
        }

        let bull_power = candles[i].high - ema[i];
        let bear_power = candles[i].low - ema[i];
        let prev_bull = candles[i - 1].high - ema[i - 1];
        let prev_bear = candles[i - 1].low - ema[i - 1];

        let ema_rising = ema[i] > ema[i - 1];
        let ema_falling = ema[i] < ema[i - 1];
        let bear_rising = bear_power > prev_bear;
        let bull_falling = bull_power < prev_bull;
        let bull_rising = bull_power > prev_bull;

        let (side, confidence) = if ema_rising && bear_power < 0.0 && bear_rising {
            let strength = bear_power.abs() / ema[i] * 100.0;
            (Some(OrderSide::Buy), (40.0 + strength * 10.0 + if bull_rising { 15.0 } else { 0.0 }).min(75.0))
        } else if ema_falling && bull_power > 0.0 && bull_falling {
            let strength = bull_power / ema[i] * 100.0;
            (Some(OrderSide::Sell), (40.0 + strength * 10.0 + if !bear_rising { 15.0 } else { 0.0 }).min(75.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("bull_power".into(), (bull_power * 10000.0).round() / 10000.0);
        indicators.insert("bear_power".into(), (bear_power * 10000.0).round() / 10000.0);
        indicators.insert("ema_dir".into(), if ema_rising { 1.0 } else if ema_falling { -1.0 } else { 0.0 });

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("Elder bull:{:.4} bear:{:.4}", bull_power, bear_power),
        }
    }
}
