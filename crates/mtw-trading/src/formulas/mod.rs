pub mod rsi;
pub mod macd;
pub mod ema_crossover;
pub mod bollinger;
pub mod supertrend;
pub mod adx;
pub mod stochastic_rsi;
pub mod ichimoku;
pub mod obv;
pub mod kelly;
pub mod linear_regression;
pub mod vwap;
pub mod williams_r;
pub mod ensemble;
pub mod regime;

pub use rsi::RsiFormula;
pub use macd::MacdFormula;
pub use ema_crossover::EmaCrossoverFormula;
pub use bollinger::BollingerFormula;
pub use supertrend::SuperTrendFormula;
pub use adx::AdxFormula;
pub use stochastic_rsi::StochasticRsiFormula;
pub use ichimoku::IchimokuFormula;
pub use obv::ObvFormula;
pub use kelly::KellyFormula;
pub use linear_regression::LinearRegressionFormula;
pub use vwap::VwapFormula;
pub use williams_r::WilliamsRFormula;
pub use ensemble::EnsembleFormula;
pub use regime::RegimeFormula;

use crate::formula::FormulaRegistry;

/// Register all built-in formulas
pub fn register_all(registry: &mut FormulaRegistry) {
    registry.register(Box::new(RsiFormula));
    registry.register(Box::new(MacdFormula));
    registry.register(Box::new(EmaCrossoverFormula));
    registry.register(Box::new(BollingerFormula));
    registry.register(Box::new(SuperTrendFormula));
    registry.register(Box::new(AdxFormula));
    registry.register(Box::new(StochasticRsiFormula));
    registry.register(Box::new(IchimokuFormula));
    registry.register(Box::new(ObvFormula));
    registry.register(Box::new(KellyFormula));
    registry.register(Box::new(LinearRegressionFormula));
    registry.register(Box::new(VwapFormula));
    registry.register(Box::new(WilliamsRFormula));
    registry.register(Box::new(EnsembleFormula));
    registry.register(Box::new(RegimeFormula));
}
