use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Money Flow Index — volume-weighted RSI. More accurate than RSI or OBV alone.
pub struct MfiFormula;

impl SignalFormula for MfiFormula {
    fn id(&self) -> &str { "mfi" }
    fn name(&self) -> &str { "Money Flow Index" }
    fn description(&self) -> &str { "Volume-weighted RSI: overbought/oversold with volume confirmation" }
    fn min_candles(&self) -> usize { 20 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(14.0) as usize;
        let n = candles.len();
        let i = n - 1;
        if i < period + 1 { return FormulaResult::neutral("insufficient data"); }

        let calc_mfi = |start: usize, end: usize| -> f64 {
            let mut pos = 0.0_f64;
            let mut neg = 0.0_f64;
            for j in start..=end {
                let tp = (candles[j].high + candles[j].low + candles[j].close) / 3.0;
                let prev_tp = (candles[j - 1].high + candles[j - 1].low + candles[j - 1].close) / 3.0;
                let mf = tp * candles[j].volume.max(1.0);
                if tp > prev_tp { pos += mf; } else if tp < prev_tp { neg += mf; }
            }
            if neg > 0.0 { 100.0 - 100.0 / (1.0 + pos / neg) } else { 100.0 }
        };

        let mfi = calc_mfi(i + 1 - period, i);
        let prev_mfi = if i >= period + 1 { calc_mfi(i - period, i - 1) } else { mfi };

        let oversold = mfi < 20.0;
        let overbought = mfi > 80.0;
        let rising = mfi > prev_mfi;

        let (side, confidence) = if oversold && rising {
            (Some(OrderSide::Buy), (45.0 + (20.0 - mfi) * 2.0).min(80.0))
        } else if overbought && !rising {
            (Some(OrderSide::Sell), (45.0 + (mfi - 80.0) * 2.0).min(80.0))
        } else if mfi < 30.0 && rising {
            (Some(OrderSide::Buy), (30.0 + 30.0 - mfi).min(60.0))
        } else if mfi > 70.0 && !rising {
            (Some(OrderSide::Sell), (30.0 + mfi - 70.0).min(60.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("mfi".into(), (mfi * 10.0).round() / 10.0);
        indicators.insert("prev_mfi".into(), (prev_mfi * 10.0).round() / 10.0);

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("MFI {:.0}{}", mfi, if rising { " rising" } else { " falling" }),
        }
    }
}
