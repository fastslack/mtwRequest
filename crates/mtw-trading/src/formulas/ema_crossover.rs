use std::collections::HashMap;
use crate::formula::{compute_ema, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

pub struct EmaCrossoverFormula;

impl SignalFormula for EmaCrossoverFormula {
    fn id(&self) -> &str { "ema_crossover" }
    fn name(&self) -> &str { "EMA Crossover (9/21)" }
    fn description(&self) -> &str { "Trend: short EMA crosses long EMA" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], _context: Option<&FormulaContext>) -> FormulaResult {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let short = compute_ema(&closes, 9);
        let long = compute_ema(&closes, 21);
        if short.len() < 2 || long.len() < 2 { return FormulaResult::neutral("insufficient data"); }
        let n = short.len() - 1;
        let cross_up = short[n - 1] <= long[n - 1] && short[n] > long[n];
        let cross_down = short[n - 1] >= long[n - 1] && short[n] < long[n];
        let mut indicators = HashMap::new();
        indicators.insert("ema9".into(), (short[n] * 100.0).round() / 100.0);
        indicators.insert("ema21".into(), (long[n] * 100.0).round() / 100.0);
        FormulaResult {
            side: if cross_up { Some(OrderSide::Buy) } else if cross_down { Some(OrderSide::Sell) } else { None },
            confidence: if cross_up || cross_down { 65.0 } else { 0.0 },
            indicators,
            reasoning: format!("EMA9 {:.2} vs EMA21 {:.2}", short[n], long[n]),
        }
    }
}
