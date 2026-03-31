use std::collections::HashMap;
use crate::types::{Candle, OrderSide, Timeframe};
use crate::signal::SignalConsensus;

#[derive(Debug, Clone)]
pub struct FormulaResult {
    pub side: Option<OrderSide>,
    pub confidence: f64,
    pub indicators: HashMap<String, f64>,
    pub reasoning: String,
}

impl FormulaResult {
    pub fn neutral(reasoning: impl Into<String>) -> Self {
        Self { side: None, confidence: 0.0, indicators: HashMap::new(), reasoning: reasoning.into() }
    }
}

#[derive(Debug, Clone)]
pub struct FormulaContext {
    pub params: HashMap<String, f64>,
    pub timeframe: Option<Timeframe>,
}

impl Default for FormulaContext {
    fn default() -> Self { Self { params: HashMap::new(), timeframe: None } }
}

pub trait SignalFormula: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn min_candles(&self) -> usize;
    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult;
}

pub struct FormulaRegistry {
    formulas: Vec<Box<dyn SignalFormula>>,
}

impl FormulaRegistry {
    pub fn new() -> Self { Self { formulas: Vec::new() } }

    pub fn register(&mut self, formula: Box<dyn SignalFormula>) {
        self.formulas.push(formula);
    }

    pub fn get(&self, id: &str) -> Option<&dyn SignalFormula> {
        self.formulas.iter().find(|f| f.id() == id).map(|f| f.as_ref())
    }

    pub fn list(&self) -> Vec<(&str, &str)> {
        self.formulas.iter().map(|f| (f.id(), f.name())).collect()
    }

    pub fn compute_all(&self, candles: &[Candle], ctx: Option<&FormulaContext>) -> Vec<(String, FormulaResult)> {
        self.formulas.iter()
            .filter(|f| candles.len() >= f.min_candles())
            .map(|f| (f.id().to_string(), f.compute(candles, ctx)))
            .collect()
    }

    pub fn consensus(&self, symbol: &str, candles: &[Candle], ctx: Option<&FormulaContext>) -> SignalConsensus {
        let results = self.compute_all(candles, ctx);
        let total_checked = results.len() as u32;
        let mut buy_formulas = Vec::new();
        let mut sell_formulas = Vec::new();
        let mut buy_conf = 0.0;
        let mut sell_conf = 0.0;

        for (id, r) in &results {
            match r.side {
                Some(OrderSide::Buy) => { buy_formulas.push(id.clone()); buy_conf += r.confidence; }
                Some(OrderSide::Sell) => { sell_formulas.push(id.clone()); sell_conf += r.confidence; }
                None => {}
            }
        }

        let (side, formulas, avg_confidence, count) = if buy_formulas.len() >= sell_formulas.len() && !buy_formulas.is_empty() {
            let avg = buy_conf / buy_formulas.len() as f64;
            (Some(OrderSide::Buy), buy_formulas.clone(), avg, buy_formulas.len() as u32)
        } else if !sell_formulas.is_empty() {
            let avg = sell_conf / sell_formulas.len() as f64;
            (Some(OrderSide::Sell), sell_formulas.clone(), avg, sell_formulas.len() as u32)
        } else {
            (None, Vec::new(), 0.0, 0)
        };

        SignalConsensus { symbol: symbol.to_string(), side, avg_confidence, formula_count: count, formulas, total_checked }
    }
}

impl Default for FormulaRegistry { fn default() -> Self { Self::new() } }

pub fn compute_ema(data: &[f64], period: usize) -> Vec<f64> {
    if data.is_empty() || period == 0 { return Vec::new(); }
    let k = 2.0 / (period as f64 + 1.0);
    let mut ema = vec![0.0; data.len()];
    ema[0] = data[0];
    for i in 1..data.len() { ema[i] = data[i] * k + ema[i - 1] * (1.0 - k); }
    ema
}

pub fn compute_sma(data: &[f64], period: usize) -> Vec<f64> {
    if data.len() < period { return Vec::new(); }
    let mut sma = vec![0.0; data.len()];
    let mut sum: f64 = data[..period].iter().sum();
    sma[period - 1] = sum / period as f64;
    for i in period..data.len() {
        sum += data[i] - data[i - period];
        sma[i] = sum / period as f64;
    }
    sma
}

pub fn compute_rsi(closes: &[f64], period: usize) -> Vec<f64> {
    if closes.len() < period + 1 { return Vec::new(); }
    let mut rsi = vec![50.0; closes.len()];
    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;
    for i in 1..=period {
        let diff = closes[i] - closes[i - 1];
        if diff > 0.0 { avg_gain += diff; } else { avg_loss -= diff; }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;
    if avg_loss == 0.0 { rsi[period] = 100.0; }
    else { let rs = avg_gain / avg_loss; rsi[period] = 100.0 - (100.0 / (1.0 + rs)); }
    for i in (period + 1)..closes.len() {
        let diff = closes[i] - closes[i - 1];
        let (gain, loss) = if diff > 0.0 { (diff, 0.0) } else { (0.0, -diff) };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        if avg_loss == 0.0 { rsi[i] = 100.0; }
        else { let rs = avg_gain / avg_loss; rsi[i] = 100.0 - (100.0 / (1.0 + rs)); }
    }
    rsi
}

pub fn compute_atr(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < 2 { return Vec::new(); }
    let mut trs = Vec::with_capacity(candles.len());
    trs.push(candles[0].high - candles[0].low);
    for i in 1..candles.len() {
        let c = &candles[i];
        let prev_close = candles[i - 1].close;
        let tr = (c.high - c.low).max((c.high - prev_close).abs()).max((c.low - prev_close).abs());
        trs.push(tr);
    }
    compute_ema(&trs, period)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ema() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let ema = compute_ema(&data, 3);
        assert_eq!(ema.len(), 5);
        assert!((ema[0] - 1.0).abs() < 0.01);
        assert!(ema[4] > ema[0]); // uptrend
    }

    #[test]
    fn test_rsi() {
        let mut closes = Vec::new();
        for i in 0..30 { closes.push(100.0 + i as f64); } // uptrend
        let rsi = compute_rsi(&closes, 14);
        assert!(rsi[29] > 70.0); // should be overbought
    }

    #[test]
    fn test_sma() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let sma = compute_sma(&data, 3);
        assert!((sma[2] - 2.0).abs() < 0.01);
        assert!((sma[4] - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_registry_consensus() {
        let reg = FormulaRegistry::new();
        let candles = vec![Candle { timestamp: 0, open: 1.0, high: 2.0, low: 0.5, close: 1.5, volume: 100.0 }];
        let c = reg.consensus("BTC", &candles, None);
        assert_eq!(c.total_checked, 0);
        assert!(c.side.is_none());
    }
}
