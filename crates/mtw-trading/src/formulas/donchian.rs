use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Donchian Channels — highest high / lowest low breakout (Turtle Trading).
pub struct DonchianFormula;

impl SignalFormula for DonchianFormula {
    fn id(&self) -> &str { "donchian" }
    fn name(&self) -> &str { "Donchian Channels" }
    fn description(&self) -> &str { "Turtle Trading breakout: buy at new highs, sell at new lows" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("entry_period")).copied().unwrap_or(20.0) as usize;
        let n = candles.len();
        let i = n - 1;
        if i < period { return FormulaResult::neutral("insufficient data"); }

        // Previous bar's channel (avoid look-ahead)
        let window = &candles[(i - period)..i];
        let upper = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let lower = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let mid = (upper + lower) / 2.0;
        let width_pct = if mid > 0.0 { ((upper - lower) / mid) * 100.0 } else { 0.0 };

        let breakout_up = candles[i].high > upper;
        let breakout_down = candles[i].low < lower;

        // Volume confirmation
        let avg_vol: f64 = candles[i.saturating_sub(20)..i].iter().map(|c| c.volume).sum::<f64>()
            / 20.0_f64.min(i as f64).max(1.0);
        let vol_confirm = candles[i].volume > avg_vol * 1.2;

        let (side, confidence) = if breakout_up {
            (Some(OrderSide::Buy), (40.0 + width_pct * 3.0 + if vol_confirm { 15.0 } else { 0.0 }).min(80.0))
        } else if breakout_down {
            (Some(OrderSide::Sell), (40.0 + width_pct * 3.0 + if vol_confirm { 15.0 } else { 0.0 }).min(80.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("upper".into(), (upper * 100.0).round() / 100.0);
        indicators.insert("lower".into(), (lower * 100.0).round() / 100.0);
        indicators.insert("width_pct".into(), (width_pct * 100.0).round() / 100.0);

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("Donchian upper:{:.2} lower:{:.2} width:{:.1}%", upper, lower, width_pct),
        }
    }
}
