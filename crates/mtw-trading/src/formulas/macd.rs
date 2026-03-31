use std::collections::HashMap;
use crate::formula::{compute_ema, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

pub struct MacdFormula;

impl SignalFormula for MacdFormula {
    fn id(&self) -> &str { "macd" }
    fn name(&self) -> &str { "MACD (12/26/9)" }
    fn description(&self) -> &str { "Trend following: bullish/bearish crossover" }
    fn min_candles(&self) -> usize { 35 }

    fn compute(&self, candles: &[Candle], _context: Option<&FormulaContext>) -> FormulaResult {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let ema12 = compute_ema(&closes, 12);
        let ema26 = compute_ema(&closes, 26);
        if ema12.len() < 2 || ema26.len() < 2 { return FormulaResult::neutral("insufficient data"); }
        let macd_line: Vec<f64> = ema12.iter().zip(ema26.iter()).map(|(a, b)| a - b).collect();
        let signal_line = compute_ema(&macd_line, 9);
        let n = macd_line.len() - 1;
        let macd = macd_line[n];
        let signal = signal_line[n];
        let prev_macd = macd_line[n - 1];
        let prev_signal = signal_line[n - 1];
        let bullish_cross = prev_macd <= prev_signal && macd > signal;
        let bearish_cross = prev_macd >= prev_signal && macd < signal;
        let histogram = macd - signal;
        let mut indicators = HashMap::new();
        indicators.insert("macd".into(), (macd * 100.0).round() / 100.0);
        indicators.insert("signal".into(), (signal * 100.0).round() / 100.0);
        indicators.insert("histogram".into(), (histogram * 100.0).round() / 100.0);
        FormulaResult {
            side: if bullish_cross { Some(OrderSide::Buy) } else if bearish_cross { Some(OrderSide::Sell) } else { None },
            confidence: if bullish_cross || bearish_cross { 70.0 } else { 0.0 },
            indicators,
            reasoning: format!("MACD {:.2} signal {:.2} hist {:.2}", macd, signal, histogram),
        }
    }
}
