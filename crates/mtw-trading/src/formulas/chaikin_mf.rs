use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Chaikin Money Flow — volume-weighted accumulation/distribution.
pub struct ChaikinMfFormula;

impl SignalFormula for ChaikinMfFormula {
    fn id(&self) -> &str { "chaikin_mf" }
    fn name(&self) -> &str { "Chaikin Money Flow" }
    fn description(&self) -> &str { "Accumulation/distribution: volume-weighted close position reveals smart money" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(20.0) as usize;
        let i = candles.len() - 1;
        if i < period { return FormulaResult::neutral("insufficient data"); }

        let calc_cmf = |start: usize, end: usize| -> f64 {
            let mut sum_mfv = 0.0_f64;
            let mut sum_vol = 0.0_f64;
            for j in start..=end {
                let hl = candles[j].high - candles[j].low;
                let mfm = if hl > 0.0 { ((candles[j].close - candles[j].low) - (candles[j].high - candles[j].close)) / hl } else { 0.0 };
                sum_mfv += mfm * candles[j].volume;
                sum_vol += candles[j].volume;
            }
            if sum_vol > 0.0 { sum_mfv / sum_vol } else { 0.0 }
        };

        let cmf = calc_cmf(i + 1 - period, i);
        let prev_cmf = if i >= period { calc_cmf(i - period, i - 1) } else { 0.0 };

        let cross_above = prev_cmf <= -0.05 && cmf > 0.05;
        let cross_below = prev_cmf >= 0.05 && cmf < -0.05;
        let strong_buy = cmf > 0.25;
        let strong_sell = cmf < -0.25;

        let (side, confidence) = if cross_above || strong_buy {
            (Some(OrderSide::Buy), (35.0 + cmf.abs() * 200.0 + if cross_above { 15.0 } else { 0.0 }).min(75.0))
        } else if cross_below || strong_sell {
            (Some(OrderSide::Sell), (35.0 + cmf.abs() * 200.0 + if cross_below { 15.0 } else { 0.0 }).min(75.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("cmf".into(), (cmf * 10000.0).round() / 10000.0);
        indicators.insert("prev_cmf".into(), (prev_cmf * 10000.0).round() / 10000.0);

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("CMF {:.3} (prev {:.3})", cmf, prev_cmf),
        }
    }
}
