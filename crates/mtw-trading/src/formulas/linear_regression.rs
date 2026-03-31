use std::collections::HashMap;
use crate::formula::{FormulaContext, FormulaResult, SignalFormula};
use crate::types::{Candle, OrderSide};

/// Linear Regression with R-squared and standard error bands.
///
/// Fits a least-squares line to closing prices. Uses slope direction,
/// R-squared (goodness of fit), and position relative to regression bands
/// to generate signals.
pub struct LinearRegressionFormula;

impl SignalFormula for LinearRegressionFormula {
    fn id(&self) -> &str { "linear_regression" }
    fn name(&self) -> &str { "Linear Regression (20)" }
    fn description(&self) -> &str { "Trend direction from least-squares fit with R² and error bands" }
    fn min_candles(&self) -> usize { 25 }

    fn compute(&self, candles: &[Candle], context: Option<&FormulaContext>) -> FormulaResult {
        let period = context.and_then(|c| c.params.get("period")).copied().unwrap_or(20.0) as usize;
        let n = candles.len();
        if n < period {
            return FormulaResult::neutral("insufficient data");
        }

        let window = &candles[n - period..n];
        let closes: Vec<f64> = window.iter().map(|c| c.close).collect();

        // Least squares: y = slope * x + intercept
        let (slope, intercept, r_squared, std_err) = least_squares(&closes);

        // Predicted value at end and deviation
        let predicted = slope * (period as f64 - 1.0) + intercept;
        let actual = closes[period - 1];
        let deviation = (actual - predicted) / std_err.max(0.0001);

        // Slope normalized to price (percentage per bar)
        let slope_pct = if predicted != 0.0 { slope / predicted * 100.0 } else { 0.0 };

        let mut indicators = HashMap::new();
        indicators.insert("slope".into(), (slope * 10000.0).round() / 10000.0);
        indicators.insert("slope_pct".into(), (slope_pct * 10000.0).round() / 10000.0);
        indicators.insert("r_squared".into(), (r_squared * 10000.0).round() / 10000.0);
        indicators.insert("std_error".into(), (std_err * 100.0).round() / 100.0);
        indicators.insert("deviation".into(), (deviation * 100.0).round() / 100.0);
        indicators.insert("predicted".into(), (predicted * 100.0).round() / 100.0);

        // Signal: strong trend (high R²) + clear slope direction
        let strong_fit = r_squared > 0.6;
        let (side, confidence) = if strong_fit {
            if slope > 0.0 && deviation > -1.5 {
                // Uptrend with price not too far below regression
                let conf = 55.0 + r_squared * 30.0;
                (Some(OrderSide::Buy), conf)
            } else if slope < 0.0 && deviation < 1.5 {
                let conf = 55.0 + r_squared * 30.0;
                (Some(OrderSide::Sell), conf)
            } else {
                (None, 0.0) // Mean-reversion scenario (outside bands)
            }
        } else {
            (None, 0.0)
        };

        FormulaResult {
            side,
            confidence: confidence.min(90.0),
            indicators,
            reasoning: format!("LinReg slope {:.4} R²={:.3} dev {:.2}σ", slope, r_squared, deviation),
        }
    }
}

/// Compute least-squares linear regression.
/// Returns (slope, intercept, r_squared, standard_error).
fn least_squares(data: &[f64]) -> (f64, f64, f64, f64) {
    let n = data.len() as f64;
    if n < 2.0 { return (0.0, data.first().copied().unwrap_or(0.0), 0.0, 0.0); }

    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_x2 = 0.0;
    let mut sum_y2 = 0.0;

    for (i, &y) in data.iter().enumerate() {
        let x = i as f64;
        sum_x += x;
        sum_y += y;
        sum_xy += x * y;
        sum_x2 += x * x;
        sum_y2 += y * y;
    }

    let denom = n * sum_x2 - sum_x * sum_x;
    if denom.abs() < 1e-10 {
        return (0.0, sum_y / n, 0.0, 0.0);
    }

    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;

    // R-squared
    let y_mean = sum_y / n;
    let ss_tot = sum_y2 - n * y_mean * y_mean;
    let mut ss_res = 0.0;
    for (i, &y) in data.iter().enumerate() {
        let predicted = slope * i as f64 + intercept;
        ss_res += (y - predicted).powi(2);
    }
    let r_squared = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 };

    // Standard error of regression
    let std_err = if n > 2.0 { (ss_res / (n - 2.0)).sqrt() } else { 0.0 };

    (slope, intercept, r_squared.max(0.0), std_err)
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
    fn test_linreg_perfect_uptrend() {
        let prices: Vec<f64> = (0..30).map(|i| 100.0 + i as f64 * 2.0).collect();
        let r = LinearRegressionFormula.compute(&make_candles(&prices), None);
        assert_eq!(r.side, Some(OrderSide::Buy));
        assert!(r.indicators["r_squared"] > 0.99);
        assert!(r.indicators["slope"] > 0.0);
    }

    #[test]
    fn test_linreg_perfect_downtrend() {
        let prices: Vec<f64> = (0..30).map(|i| 200.0 - i as f64 * 2.0).collect();
        let r = LinearRegressionFormula.compute(&make_candles(&prices), None);
        assert_eq!(r.side, Some(OrderSide::Sell));
        assert!(r.indicators["r_squared"] > 0.99);
    }

    #[test]
    fn test_least_squares_line() {
        let data = vec![1.0, 3.0, 5.0, 7.0, 9.0];
        let (slope, intercept, r2, _) = least_squares(&data);
        assert!((slope - 2.0).abs() < 0.01);
        assert!((intercept - 1.0).abs() < 0.01);
        assert!((r2 - 1.0).abs() < 0.01);
    }
}
