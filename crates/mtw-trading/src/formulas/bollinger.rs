use std::collections::HashMap;
use crate::formula::{compute_sma, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

pub struct BollingerFormula;

impl SignalFormula for BollingerFormula {
    fn id(&self) -> &str { "bollinger" }
    fn name(&self) -> &str { "Bollinger Bands (20, 2.0)" }
    fn description(&self) -> &str { "Mean-reversion: buy below lower band, sell above upper" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], _context: Option<&FormulaContext>) -> FormulaResult {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let period = 20;
        let mult = 2.0;
        let sma = compute_sma(&closes, period);
        if sma.is_empty() { return FormulaResult::neutral("insufficient data"); }
        let n = closes.len() - 1;
        let mean = sma[n];
        let variance: f64 = closes[n + 1 - period..=n].iter().map(|x| (x - mean).powi(2)).sum::<f64>() / period as f64;
        let std_dev = variance.sqrt();
        let upper = mean + mult * std_dev;
        let lower = mean - mult * std_dev;
        let price = closes[n];
        let below = price < lower;
        let above = price > upper;
        let mut indicators = HashMap::new();
        indicators.insert("upper".into(), (upper * 100.0).round() / 100.0);
        indicators.insert("middle".into(), (mean * 100.0).round() / 100.0);
        indicators.insert("lower".into(), (lower * 100.0).round() / 100.0);
        FormulaResult {
            side: if below { Some(OrderSide::Buy) } else if above { Some(OrderSide::Sell) } else { None },
            confidence: if below || above { 70.0 } else { 0.0 },
            indicators,
            reasoning: format!("BB price {:.2} lower {:.2} upper {:.2}", price, lower, upper),
        }
    }
}
