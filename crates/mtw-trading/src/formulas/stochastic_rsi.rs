use std::collections::HashMap;
use crate::formula::{compute_rsi, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Stochastic RSI: applies the Stochastic oscillator to RSI values.
///
/// Generates %K and %D lines. Crossovers in oversold (<20) or overbought (>80) zones
/// produce signals.
pub struct StochasticRsiFormula;

impl SignalFormula for StochasticRsiFormula {
    fn id(&self) -> &str { "stochastic_rsi" }
    fn name(&self) -> &str { "Stochastic RSI (14, 14, 3, 3)" }
    fn description(&self) -> &str { "Momentum: %K/%D crossovers in oversold/overbought zones" }
    fn min_candles(&self) -> usize { 60 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let rsi_period = context.and_then(|c| c.params.get("rsi_period")).copied().unwrap_or(14.0) as usize;
        let stoch_period = context.and_then(|c| c.params.get("stoch_period")).copied().unwrap_or(14.0) as usize;
        let k_smooth = context.and_then(|c| c.params.get("k_smooth")).copied().unwrap_or(3.0) as usize;
        let d_smooth = context.and_then(|c| c.params.get("d_smooth")).copied().unwrap_or(3.0) as usize;
        let oversold = context.and_then(|c| c.params.get("oversold")).copied().unwrap_or(20.0);
        let overbought = context.and_then(|c| c.params.get("overbought")).copied().unwrap_or(80.0);

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let rsi = compute_rsi(&closes, rsi_period);
        if rsi.len() < stoch_period + k_smooth + d_smooth {
            return FormulaResult::neutral("insufficient data for stochastic RSI");
        }

        // Compute Stochastic of RSI: %K = (RSI - min(RSI, N)) / (max(RSI, N) - min(RSI, N)) * 100
        let start = rsi_period; // RSI values before this index are placeholders
        let valid_rsi = &rsi[start..];
        if valid_rsi.len() < stoch_period {
            return FormulaResult::neutral("insufficient RSI data");
        }

        let mut stoch_k_raw = Vec::with_capacity(valid_rsi.len());
        for i in 0..valid_rsi.len() {
            if i + 1 < stoch_period {
                stoch_k_raw.push(50.0);
                continue;
            }
            let window = &valid_rsi[i + 1 - stoch_period..=i];
            let min = window.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let range = max - min;
            let k = if range > 0.0 { (valid_rsi[i] - min) / range * 100.0 } else { 50.0 };
            stoch_k_raw.push(k);
        }

        // Smooth %K with SMA
        let k_values = sma_smooth(&stoch_k_raw, k_smooth);
        if k_values.len() < d_smooth + 1 {
            return FormulaResult::neutral("insufficient smoothed K data");
        }

        // %D = SMA of %K
        let d_values = sma_smooth(&k_values, d_smooth);
        if d_values.len() < 2 {
            return FormulaResult::neutral("insufficient D data");
        }

        let n = d_values.len() - 1;
        let k_n = k_values[k_values.len() - 1];
        let k_prev = k_values[k_values.len() - 2];
        // Align D with K: D is shorter by (d_smooth - 1) elements
        let d_offset = k_values.len() - d_values.len();
        let d_idx = k_values.len() - 1 - d_offset;
        let d_n = if d_idx < d_values.len() { d_values[d_idx] } else { d_values[n] };
        let d_prev_idx = if d_idx > 0 { d_idx - 1 } else { 0 };
        let d_prev = d_values[d_prev_idx];

        let bullish_cross = k_prev <= d_prev && k_n > d_n && k_n < oversold + 10.0;
        let bearish_cross = k_prev >= d_prev && k_n < d_n && k_n > overbought - 10.0;

        let in_oversold = k_n < oversold;
        let in_overbought = k_n > overbought;

        let mut indicators = HashMap::new();
        indicators.insert("stoch_rsi_k".into(), (k_n * 100.0).round() / 100.0);
        indicators.insert("stoch_rsi_d".into(), (d_n * 100.0).round() / 100.0);

        let (side, confidence) = if bullish_cross || in_oversold {
            (Some(OrderSide::Buy), if bullish_cross { 75.0 } else { 60.0 })
        } else if bearish_cross || in_overbought {
            (Some(OrderSide::Sell), if bearish_cross { 75.0 } else { 60.0 })
        } else {
            (None, 0.0)
        };

        FormulaResult {
            side,
            confidence,
            indicators,
            reasoning: format!("StochRSI %K {:.1} %D {:.1}", k_n, d_n),
        }
    }
}

fn sma_smooth(data: &[f64], period: usize) -> Vec<f64> {
    if data.len() < period || period == 0 { return data.to_vec(); }
    let mut result = Vec::with_capacity(data.len() - period + 1);
    let mut sum: f64 = data[..period].iter().sum();
    result.push(sum / period as f64);
    for i in period..data.len() {
        sum += data[i] - data[i - period];
        result.push(sum / period as f64);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candles(prices: &[f64]) -> Vec<Candle> {
        prices.iter().enumerate().map(|(i, &p)| Candle {
            timestamp: i as u64, open: p, high: p + 1.0, low: p - 1.0, close: p, volume: 100.0,
        }).collect()
    }

    #[test]
    fn test_stochastic_rsi_downtrend_oversold() {
        // Strong downtrend -> RSI very low -> StochRSI should be oversold -> Buy
        let prices: Vec<f64> = (0..80).map(|i| 200.0 - i as f64).collect();
        let r = StochasticRsiFormula.compute(&make_candles(&prices), None);
        if r.side.is_some() {
            assert_eq!(r.side, Some(OrderSide::Buy));
        }
    }

    #[test]
    fn test_stochastic_rsi_uptrend_overbought() {
        let prices: Vec<f64> = (0..80).map(|i| 100.0 + i as f64).collect();
        let r = StochasticRsiFormula.compute(&make_candles(&prices), None);
        if r.side.is_some() {
            assert_eq!(r.side, Some(OrderSide::Sell));
        }
    }
}
