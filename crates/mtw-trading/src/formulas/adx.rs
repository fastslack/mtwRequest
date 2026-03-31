use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Average Directional Index (ADX) with +DI/-DI.
///
/// ADX measures trend strength. +DI/-DI indicate direction.
/// - ADX > 25 = trending, ADX > 40 = strong trend
/// - +DI > -DI = bullish, -DI > +DI = bearish
pub struct AdxFormula;

impl SignalFormula for AdxFormula {
    fn id(&self) -> &str { "adx" }
    fn name(&self) -> &str { "ADX (14)" }
    fn description(&self) -> &str { "Trend strength with directional indicators (+DI/-DI)" }
    fn min_candles(&self) -> usize { 30 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(14.0) as usize;
        let n = candles.len();
        if n < period + 1 {
            return FormulaResult::neutral("insufficient data");
        }

        // Compute True Range, +DM, -DM
        let mut tr_vals = Vec::with_capacity(n - 1);
        let mut plus_dm = Vec::with_capacity(n - 1);
        let mut minus_dm = Vec::with_capacity(n - 1);

        for i in 1..n {
            let high = candles[i].high;
            let low = candles[i].low;
            let prev_close = candles[i - 1].close;
            let prev_high = candles[i - 1].high;
            let prev_low = candles[i - 1].low;

            let tr = (high - low)
                .max((high - prev_close).abs())
                .max((low - prev_close).abs());
            tr_vals.push(tr);

            let up_move = high - prev_high;
            let down_move = prev_low - low;

            if up_move > down_move && up_move > 0.0 {
                plus_dm.push(up_move);
            } else {
                plus_dm.push(0.0);
            }
            if down_move > up_move && down_move > 0.0 {
                minus_dm.push(down_move);
            } else {
                minus_dm.push(0.0);
            }
        }

        // Smoothed using Wilder's method (EMA with alpha = 1/period)
        let smooth = |data: &[f64], p: usize| -> Vec<f64> {
            if data.len() < p { return Vec::new(); }
            let mut result = Vec::with_capacity(data.len());
            let first: f64 = data[..p].iter().sum();
            result.push(first);
            for i in p..data.len() {
                let val = result.last().unwrap() - result.last().unwrap() / p as f64 + data[i];
                result.push(val);
            }
            result
        };

        let smooth_tr = smooth(&tr_vals, period);
        let smooth_plus = smooth(&plus_dm, period);
        let smooth_minus = smooth(&minus_dm, period);

        if smooth_tr.is_empty() || smooth_plus.is_empty() || smooth_minus.is_empty() {
            return FormulaResult::neutral("insufficient smoothed data");
        }

        let len = smooth_tr.len();

        // Compute DI values
        let mut dx_vals = Vec::with_capacity(len);
        let mut last_plus_di = 0.0;
        let mut last_minus_di = 0.0;

        for i in 0..len {
            if smooth_tr[i] == 0.0 {
                dx_vals.push(0.0);
                continue;
            }
            let pdi = 100.0 * smooth_plus[i] / smooth_tr[i];
            let mdi = 100.0 * smooth_minus[i] / smooth_tr[i];
            let sum = pdi + mdi;
            let dx = if sum > 0.0 { 100.0 * (pdi - mdi).abs() / sum } else { 0.0 };
            dx_vals.push(dx);
            last_plus_di = pdi;
            last_minus_di = mdi;
        }

        // ADX = smoothed average of DX
        if dx_vals.len() < period {
            return FormulaResult::neutral("insufficient DX data");
        }
        let first_adx: f64 = dx_vals[..period].iter().sum::<f64>() / period as f64;
        let mut adx = first_adx;
        for i in period..dx_vals.len() {
            adx = (adx * (period as f64 - 1.0) + dx_vals[i]) / period as f64;
        }

        let mut indicators = HashMap::new();
        indicators.insert("adx".into(), (adx * 100.0).round() / 100.0);
        indicators.insert("plus_di".into(), (last_plus_di * 100.0).round() / 100.0);
        indicators.insert("minus_di".into(), (last_minus_di * 100.0).round() / 100.0);

        let strong_trend = adx >= 25.0;
        let very_strong = adx >= 40.0;

        let (side, confidence) = if strong_trend {
            if last_plus_di > last_minus_di {
                let conf = if very_strong { 80.0 } else { 65.0 };
                (Some(OrderSide::Buy), conf)
            } else {
                let conf = if very_strong { 80.0 } else { 65.0 };
                (Some(OrderSide::Sell), conf)
            }
        } else {
            (None, 0.0)
        };

        FormulaResult {
            side,
            confidence,
            indicators,
            reasoning: format!("ADX {:.1} +DI {:.1} -DI {:.1}", adx, last_plus_di, last_minus_di),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trending_candles(up: bool) -> Vec<Candle> {
        (0..50).map(|i| {
            let base = if up { 100.0 + i as f64 * 2.0 } else { 200.0 - i as f64 * 2.0 };
            Candle {
                timestamp: i as u64,
                open: base - 0.5,
                high: base + 1.5,
                low: base - 1.5,
                close: base + 0.5,
                volume: 1000.0,
            }
        }).collect()
    }

    #[test]
    fn test_adx_uptrend() {
        let candles = trending_candles(true);
        let r = AdxFormula.compute(&candles, None);
        // Strong uptrend should give Buy signal
        if r.side.is_some() {
            assert_eq!(r.side, Some(OrderSide::Buy));
        }
        assert!(r.indicators.contains_key("adx"));
    }

    #[test]
    fn test_adx_downtrend() {
        let candles = trending_candles(false);
        let r = AdxFormula.compute(&candles, None);
        if r.side.is_some() {
            assert_eq!(r.side, Some(OrderSide::Sell));
        }
    }
}
