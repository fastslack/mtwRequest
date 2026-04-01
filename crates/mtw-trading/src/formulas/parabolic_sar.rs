use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Parabolic SAR — trailing stop & reversal system (Wilder).
pub struct ParabolicSarFormula;

impl SignalFormula for ParabolicSarFormula {
    fn id(&self) -> &str { "parabolic_sar" }
    fn name(&self) -> &str { "Parabolic SAR" }
    fn description(&self) -> &str { "Trailing stop & reversal: dots track trend, flip signals entries/exits" }
    fn min_candles(&self) -> usize { 10 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let n = candles.len();
        if n < 5 { return FormulaResult::neutral("insufficient data"); }

        let af_start = context.and_then(|c| c.params.get("af_start")).copied().unwrap_or(0.02);
        let af_step = context.and_then(|c| c.params.get("af_step")).copied().unwrap_or(0.02);
        let af_max = context.and_then(|c| c.params.get("af_max")).copied().unwrap_or(0.20);

        let mut bullish = candles[1].close > candles[0].close;
        let mut af = af_start;
        let mut ep = if bullish { candles[0].high } else { candles[0].low };
        let mut sar = if bullish { candles[0].low } else { candles[0].high };
        let mut directions = vec![if bullish { 1i8 } else { -1 }];

        for i in 1..n {
            sar = sar + af * (ep - sar);

            if bullish {
                if i >= 2 { sar = sar.min(candles[i - 1].low).min(candles[i - 2].low); }
                else { sar = sar.min(candles[i - 1].low); }

                if candles[i].low < sar {
                    bullish = false; sar = ep; ep = candles[i].low; af = af_start;
                } else if candles[i].high > ep {
                    ep = candles[i].high; af = (af + af_step).min(af_max);
                }
            } else {
                if i >= 2 { sar = sar.max(candles[i - 1].high).max(candles[i - 2].high); }
                else { sar = sar.max(candles[i - 1].high); }

                if candles[i].high > sar {
                    bullish = true; sar = ep; ep = candles[i].high; af = af_start;
                } else if candles[i].low < ep {
                    ep = candles[i].low; af = (af + af_step).min(af_max);
                }
            }
            directions.push(if bullish { 1 } else { -1 });
        }

        let last = n - 1;
        let dir = directions[last];
        let prev_dir = if last > 0 { directions[last - 1] } else { dir };
        let flipped = dir != prev_dir;
        let price = candles[last].close;
        let distance = ((price - sar).abs() / price) * 100.0;

        let (side, confidence) = if flipped && dir == 1 {
            (Some(OrderSide::Buy), (50.0 + distance * 8.0).min(80.0))
        } else if flipped && dir == -1 {
            (Some(OrderSide::Sell), (50.0 + distance * 8.0).min(80.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("sar".into(), (sar * 10000.0).round() / 10000.0);
        indicators.insert("direction".into(), dir as f64);
        indicators.insert("flipped".into(), if flipped { 1.0 } else { 0.0 });

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("SAR {:.4} dir:{} flipped:{}", sar, dir, flipped),
        }
    }
}
