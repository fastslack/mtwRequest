use std::collections::HashMap;
use crate::formula::{compute_sma, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Stochastic Oscillator (%K / %D) — momentum oscillator comparing close to high-low range.
pub struct StochasticFormula;

impl SignalFormula for StochasticFormula {
    fn id(&self) -> &str { "stochastic" }
    fn name(&self) -> &str { "Stochastic (14,3,3)" }
    fn description(&self) -> &str { "Momentum oscillator: %K/%D crossover in overbought/oversold zones" }
    fn min_candles(&self) -> usize { 55 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(14.0) as usize;
        let smooth_k = context.and_then(|c| c.params.get("smooth_k")).copied().unwrap_or(3.0) as usize;
        let smooth_d = context.and_then(|c| c.params.get("smooth_d")).copied().unwrap_or(3.0) as usize;
        let n = candles.len();

        // Raw %K values
        let mut raw_k: Vec<f64> = Vec::with_capacity(n);
        for i in 0..n {
            if i < period - 1 { raw_k.push(f64::NAN); continue; }
            let mut lowest = f64::INFINITY;
            let mut highest = f64::NEG_INFINITY;
            for j in (i + 1 - period)..=i {
                if candles[j].low < lowest { lowest = candles[j].low; }
                if candles[j].high > highest { highest = candles[j].high; }
            }
            let range = highest - lowest;
            raw_k.push(if range > 0.0 { ((candles[i].close - lowest) / range) * 100.0 } else { 50.0 });
        }

        // Filter valid values and smooth
        let valid_raw: Vec<f64> = raw_k.iter().copied().filter(|v| v.is_finite()).collect();
        let k_line = compute_sma(&valid_raw, smooth_k);
        let valid_k: Vec<f64> = k_line.iter().copied().filter(|&v| v != 0.0 || k_line.len() < smooth_k).collect();
        let d_line = compute_sma(&valid_k, smooth_d);

        if valid_k.len() < 3 || d_line.len() < 2 {
            return FormulaResult::neutral("insufficient data for Stochastic");
        }

        let k = valid_k[valid_k.len() - 1];
        let k_prev = valid_k[valid_k.len() - 2];
        let d = d_line[d_line.len() - 1];
        let d_prev = d_line[d_line.len() - 2];

        let bullish_cross = k_prev <= d_prev && k > d;
        let bearish_cross = k_prev >= d_prev && k < d;
        let oversold = k < 15.0 && k_prev < 20.0;
        let overbought = k > 85.0 && k_prev > 80.0;

        let (side, confidence) = if bullish_cross && oversold {
            (Some(OrderSide::Buy), (50.0 + (20.0 - k.min(k_prev)) * 1.5).min(80.0))
        } else if bearish_cross && overbought {
            (Some(OrderSide::Sell), (50.0 + (k.max(k_prev) - 80.0) * 1.5).min(80.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("k".into(), (k * 100.0).round() / 100.0);
        indicators.insert("d".into(), (d * 100.0).round() / 100.0);

        FormulaResult {
            side, confidence: confidence.round(),
            indicators,
            reasoning: format!("Stochastic %K:{:.1} %D:{:.1}", k, d),
        }
    }
}
