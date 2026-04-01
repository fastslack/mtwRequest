use std::collections::HashMap;
use crate::formula::{compute_sma, FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// VWMA Trend — Volume-Weighted Moving Average vs SMA divergence.
pub struct VwmaTrendFormula;

fn compute_vwma(closes: &[f64], volumes: &[f64], period: usize) -> Vec<f64> {
    let n = closes.len();
    let mut vwma = vec![0.0; n];
    if n < period { return vwma; }
    for i in (period - 1)..n {
        let mut sum_cv = 0.0;
        let mut sum_v = 0.0;
        for j in (i + 1 - period)..=i {
            sum_cv += closes[j] * volumes[j];
            sum_v += volumes[j];
        }
        vwma[i] = if sum_v > 0.0 { sum_cv / sum_v } else { closes[i] };
    }
    vwma
}

impl SignalFormula for VwmaTrendFormula {
    fn id(&self) -> &str { "vwma_trend" }
    fn name(&self) -> &str { "VWMA Trend" }
    fn description(&self) -> &str { "Volume-weighted trend: VWMA vs SMA divergence reveals smart money" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(20.0) as usize;
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let volumes: Vec<f64> = candles.iter().map(|c| c.volume).collect();
        let i = closes.len() - 1;

        let vwma = compute_vwma(&closes, &volumes, period);
        let sma = compute_sma(&closes, period);

        if vwma[i] == 0.0 || sma[i] == 0.0 {
            return FormulaResult::neutral("insufficient data");
        }

        let spread = ((vwma[i] - sma[i]) / sma[i]) * 100.0;
        let price = closes[i];

        // Volume ratio (recent 5 vs avg 20)
        let recent: f64 = volumes[i.saturating_sub(4)..=i].iter().sum::<f64>() / 5.0;
        let avg: f64 = volumes[i.saturating_sub(19)..=i].iter().sum::<f64>() / 20.0_f64.min(i as f64 + 1.0);
        let vol_ratio = if avg > 0.0 { recent / avg } else { 1.0 };
        let high_vol = vol_ratio > 1.3;

        let prev_spread = if i > 0 && vwma[i - 1] != 0.0 && sma[i - 1] != 0.0 {
            ((vwma[i - 1] - sma[i - 1]) / sma[i - 1]) * 100.0
        } else { spread };

        let cross_up = prev_spread <= 0.0 && spread > 0.0;
        let cross_down = prev_spread >= 0.0 && spread < 0.0;
        let above_vwma = price > vwma[i];

        let (side, confidence) = if cross_up && high_vol {
            (Some(OrderSide::Buy), (50.0 + vol_ratio * 10.0 + spread.abs() * 15.0).min(80.0))
        } else if cross_down && high_vol {
            (Some(OrderSide::Sell), (50.0 + vol_ratio * 10.0 + spread.abs() * 15.0).min(80.0))
        } else if spread > 0.1 && above_vwma && high_vol {
            (Some(OrderSide::Buy), (30.0 + spread.abs() * 20.0 + vol_ratio * 5.0).min(65.0))
        } else if spread < -0.1 && !above_vwma && high_vol {
            (Some(OrderSide::Sell), (30.0 + spread.abs() * 20.0 + vol_ratio * 5.0).min(65.0))
        } else {
            (None, 0.0)
        };

        let mut indicators = HashMap::new();
        indicators.insert("spread".into(), (spread * 1000.0).round() / 1000.0);
        indicators.insert("vol_ratio".into(), (vol_ratio * 100.0).round() / 100.0);

        FormulaResult { side, confidence: confidence.round(), indicators,
            reasoning: format!("VWMA spread:{:.3}% vol:{:.1}x", spread, vol_ratio),
        }
    }
}
