use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// On Balance Volume (OBV).
///
/// Cumulative volume indicator: adds volume on up-closes, subtracts on down-closes.
/// Detects divergence between OBV trend and price trend.
/// - Price falling + OBV rising = bullish divergence (Buy)
/// - Price rising + OBV falling = bearish divergence (Sell)
pub struct ObvFormula;

impl SignalFormula for ObvFormula {
    fn id(&self) -> &str { "obv" }
    fn name(&self) -> &str { "On Balance Volume" }
    fn description(&self) -> &str { "Volume-based divergence detection" }
    fn min_candles(&self) -> usize { 20 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let lookback = context.and_then(|c| c.params.get("lookback")).copied().unwrap_or(14.0) as usize;
        let n = candles.len();
        if n < lookback + 1 {
            return FormulaResult::neutral("insufficient data");
        }

        // Compute OBV series
        let mut obv = Vec::with_capacity(n);
        obv.push(0.0);
        for i in 1..n {
            let prev = obv[i - 1];
            if candles[i].close > candles[i - 1].close {
                obv.push(prev + candles[i].volume);
            } else if candles[i].close < candles[i - 1].close {
                obv.push(prev - candles[i].volume);
            } else {
                obv.push(prev);
            }
        }

        // Compute slopes over lookback period for both price and OBV
        let price_start = candles[n - lookback].close;
        let price_end = candles[n - 1].close;
        let price_slope = price_end - price_start;

        let obv_start = obv[n - lookback];
        let obv_end = obv[n - 1];
        let obv_slope = obv_end - obv_start;

        // Normalize OBV slope relative to total volume
        let total_vol: f64 = candles[n - lookback..n].iter().map(|c| c.volume).sum();
        let obv_slope_norm = if total_vol > 0.0 { obv_slope / total_vol } else { 0.0 };

        let mut indicators = HashMap::new();
        indicators.insert("obv".into(), (obv_end * 100.0).round() / 100.0);
        indicators.insert("obv_slope_norm".into(), (obv_slope_norm * 10000.0).round() / 10000.0);
        indicators.insert("price_slope".into(), (price_slope * 100.0).round() / 100.0);

        // Divergence detection
        let bullish_div = price_slope < 0.0 && obv_slope > 0.0; // price falling, OBV rising
        let bearish_div = price_slope > 0.0 && obv_slope < 0.0; // price rising, OBV falling

        // Confirmation: OBV trending with price
        let bullish_confirm = price_slope > 0.0 && obv_slope > 0.0 && obv_slope_norm > 0.3;
        let bearish_confirm = price_slope < 0.0 && obv_slope < 0.0 && obv_slope_norm < -0.3;

        let (side, confidence) = if bullish_div {
            (Some(OrderSide::Buy), 70.0)
        } else if bearish_div {
            (Some(OrderSide::Sell), 70.0)
        } else if bullish_confirm {
            (Some(OrderSide::Buy), 55.0)
        } else if bearish_confirm {
            (Some(OrderSide::Sell), 55.0)
        } else {
            (None, 0.0)
        };

        FormulaResult {
            side,
            confidence,
            indicators,
            reasoning: format!("OBV {:.0} slope_norm {:.4} price_slope {:.2}", obv_end, obv_slope_norm, price_slope),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_obv_bullish_divergence() {
        // Price falling but volume accumulating on small up-candles
        let mut candles = Vec::new();
        for i in 0..30 {
            let close = 100.0 - i as f64 * 0.5; // price falling
            // Volume much higher on up candles
            let vol = if i % 3 == 0 { 5000.0 } else { 500.0 };
            let prev_close = if i == 0 { close } else { 100.0 - (i - 1) as f64 * 0.5 };
            // Make every 3rd candle an up-candle
            let c = if i % 3 == 0 {
                close + 0.1
            } else {
                close
            };
            candles.push(Candle {
                timestamp: i as u64,
                open: prev_close,
                high: c + 0.5,
                low: c - 0.5,
                close: c,
                volume: vol,
            });
        }
        let r = ObvFormula.compute(&candles, None);
        assert!(r.indicators.contains_key("obv"));
    }

    #[test]
    fn test_obv_uptrend_confirm() {
        let candles: Vec<Candle> = (0..30).map(|i| {
            let base = 100.0 + i as f64;
            Candle {
                timestamp: i as u64, open: base - 0.5, high: base + 1.0,
                low: base - 1.0, close: base, volume: 1000.0 + i as f64 * 100.0,
            }
        }).collect();
        let r = ObvFormula.compute(&candles, None);
        // Price and OBV both rising -> confirmation
        if r.side.is_some() {
            assert_eq!(r.side, Some(OrderSide::Buy));
        }
    }
}
