//! MtwModule implementation for the trading crate.
//!
//! This module is gated behind the `module` feature flag, making mtw-trading
//! a fully installable/uninstallable plugin for the mtwRequest runtime.
//!
//! Enable with: `mtw-trading = { path = "...", features = ["module"] }`

use async_trait::async_trait;
use mtw_core::module::{
    HealthStatus, ModuleContext, ModuleManifest, ModuleType, MtwModule, Permission,
};
use mtw_core::MtwError;
use std::sync::Arc;

use crate::formula::FormulaRegistry;
use crate::formulas;
use crate::monitor::TradeMonitor;

/// The trading module — installable plugin for mtwRequest
///
/// Provides:
/// - 15 technical analysis formulas (RSI, MACD, Bollinger, etc.)
/// - Signal consensus engine
/// - Trade monitoring with SL/TP/trailing stop enforcement
/// - PnL calculation
pub struct TradingModule {
    manifest: ModuleManifest,
    formula_registry: Option<Arc<FormulaRegistry>>,
    trade_monitor: Option<Arc<TradeMonitor>>,
}

impl TradingModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                name: "mtw-trading".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                module_type: ModuleType::Trading,
                description: "Trading strategies, signals, and technical analysis".to_string(),
                author: "fastslack".to_string(),
                license: "Apache-2.0".to_string(),
                repository: Some("https://github.com/fastslack/mtwRequest".to_string()),
                dependencies: vec![],
                config_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "monitor_interval_secs": { "type": "integer", "default": 30 },
                        "formulas_enabled": { "type": "boolean", "default": true },
                        "paper_trading": { "type": "boolean", "default": true }
                    }
                })),
                permissions: vec![Permission::Network, Permission::Database],
                minimum_core: Some("0.2.0".to_string()),
            },
            formula_registry: None,
            trade_monitor: None,
        }
    }

    /// Get the formula registry (available after on_load)
    pub fn formula_registry(&self) -> Option<&Arc<FormulaRegistry>> {
        self.formula_registry.as_ref()
    }

    /// Get the trade monitor (available after on_load)
    pub fn trade_monitor(&self) -> Option<&Arc<TradeMonitor>> {
        self.trade_monitor.as_ref()
    }
}

impl Default for TradingModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MtwModule for TradingModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    async fn on_load(&mut self, ctx: &ModuleContext) -> Result<(), MtwError> {
        tracing::info!("loading trading module");

        // Initialize formula registry with all built-in formulas
        let formulas_enabled = ctx
            .config
            .get("formulas_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if formulas_enabled {
            let mut registry = FormulaRegistry::new();
            formulas::register_all(&mut registry);
            let count = registry.list().len();
            tracing::info!(count, "registered trading formulas");
            self.formula_registry = Some(Arc::new(registry));
        }

        // Initialize trade monitor
        self.trade_monitor = Some(Arc::new(TradeMonitor::new()));
        tracing::info!("trade monitor initialized");

        // Expose via shared state
        if let Some(ref reg) = self.formula_registry {
            let formula_list: Vec<String> = reg.list().iter().map(|(id, _)| id.to_string()).collect();
            ctx.shared.set(
                "trading.formulas",
                serde_json::to_value(&formula_list).unwrap_or_default(),
            );
        }
        ctx.shared
            .set("trading.loaded", serde_json::json!(true));

        Ok(())
    }

    async fn on_start(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
        tracing::info!("trading module started");
        Ok(())
    }

    async fn on_stop(&mut self, ctx: &ModuleContext) -> Result<(), MtwError> {
        tracing::info!("trading module stopping");

        if let Some(ref monitor) = self.trade_monitor {
            let open = monitor.position_count();
            if open > 0 {
                tracing::warn!(open, "trading module stopping with open positions");
            }
        }

        ctx.shared.remove("trading.loaded");
        ctx.shared.remove("trading.formulas");

        self.formula_registry = None;
        self.trade_monitor = None;

        Ok(())
    }

    async fn health(&self) -> HealthStatus {
        match (&self.formula_registry, &self.trade_monitor) {
            (Some(reg), Some(monitor)) => {
                let formula_count = reg.list().len();
                let _open_positions = monitor.position_count();
                if formula_count == 0 {
                    HealthStatus::Degraded("no formulas loaded".to_string())
                } else {
                    HealthStatus::Healthy
                }
            }
            _ => HealthStatus::Unhealthy("trading module not loaded".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_ctx() -> ModuleContext {
        ModuleContext {
            config: serde_json::json!({}),
            shared: Arc::new(mtw_core::module::SharedState::new()),
        }
    }

    #[tokio::test]
    async fn test_module_lifecycle() {
        let mut module = TradingModule::new();
        let ctx = make_ctx();

        assert!(module.formula_registry().is_none());
        assert!(module.trade_monitor().is_none());

        module.on_load(&ctx).await.unwrap();
        assert!(module.formula_registry().is_some());
        assert!(module.trade_monitor().is_some());

        // Formulas should be registered
        let reg = module.formula_registry().unwrap();
        assert!(reg.list().len() >= 15);

        // Shared state should be set
        assert_eq!(ctx.shared.get("trading.loaded"), Some(serde_json::json!(true)));

        module.on_start(&ctx).await.unwrap();

        let health = module.health().await;
        assert_eq!(health, HealthStatus::Healthy);

        module.on_stop(&ctx).await.unwrap();
        assert!(module.formula_registry().is_none());
        assert!(ctx.shared.get("trading.loaded").is_none());
    }

    #[tokio::test]
    async fn test_module_formulas_disabled() {
        let mut module = TradingModule::new();
        let ctx = ModuleContext {
            config: serde_json::json!({ "formulas_enabled": false }),
            shared: Arc::new(mtw_core::module::SharedState::new()),
        };

        module.on_load(&ctx).await.unwrap();
        assert!(module.formula_registry().is_none());
        assert!(module.trade_monitor().is_some());
    }

    #[test]
    fn test_manifest() {
        let module = TradingModule::new();
        let m = module.manifest();
        assert_eq!(m.name, "mtw-trading");
        assert_eq!(m.module_type, ModuleType::Trading);
        assert!(m.permissions.contains(&Permission::Network));
    }
}
