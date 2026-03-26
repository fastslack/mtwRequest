use async_trait::async_trait;
use std::sync::Arc;

use mtw_core::module::{
    HealthStatus, ModuleContext, ModuleManifest, ModuleType, MtwModule, Permission,
};
use mtw_core::MtwError;
use mtw_protocol::{MsgType, MtwMessage, Payload};

use crate::config::{ExchangeConfig, ExchangeCredentials};
use crate::manager::ExchangeManager;
use crate::providers::bitvavo::BitvavoProvider;
use crate::rate_limit::TokenBucket;
use crate::types::*;

/// Exchange module — integrates exchange connectivity into the mtwRequest lifecycle.
///
/// When loaded, it parses exchange configuration from the module config section,
/// creates providers, and optionally starts WebSocket streaming.
/// Exchange events are forwarded to mtw-router channels.
pub struct ExchangeModule {
    manifest: ModuleManifest,
    manager: Option<ExchangeManager>,
    event_task: Option<tokio::task::JoinHandle<()>>,
}

impl ExchangeModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                name: "mtw-exchange".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                module_type: ModuleType::Integration,
                description: "Exchange and broker connectivity".to_string(),
                author: "fastslack".to_string(),
                license: "MIT".to_string(),
                repository: Some(
                    "https://github.com/fastslack/mtwRequest".to_string(),
                ),
                dependencies: vec![],
                config_schema: None,
                permissions: vec![Permission::Network],
                minimum_core: Some("0.1.0".to_string()),
            },
            manager: None,
            event_task: None,
        }
    }

    /// Get a reference to the exchange manager.
    pub fn manager(&self) -> Option<&ExchangeManager> {
        self.manager.as_ref()
    }

    /// Get a mutable reference to the exchange manager.
    pub fn manager_mut(&mut self) -> Option<&mut ExchangeManager> {
        self.manager.as_mut()
    }
}

impl Default for ExchangeModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MtwModule for ExchangeModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    async fn on_load(&mut self, ctx: &ModuleContext) -> Result<(), MtwError> {
        let config: ExchangeConfig = serde_json::from_value(ctx.config.clone())
            .unwrap_or_default();

        let manager = ExchangeManager::new(config.clone());

        // Initialize providers from configuration
        for provider_config in &config.providers {
            let exchange_id = provider_config.exchange_id.as_str();
            let credentials = ExchangeCredentials::from(provider_config);

            match exchange_id {
                "bitvavo" => {
                    let rate_limiter = if let Some(ref rl_config) = provider_config.rate_limit {
                        crate::rate_limit::from_config(rl_config)
                    } else {
                        // Bitvavo default: 1000 requests per minute (weight-based)
                        Box::new(TokenBucket::new(
                            10,
                            std::time::Duration::from_secs(1),
                        ))
                    };

                    let provider =
                        BitvavoProvider::new(credentials, Arc::from(rate_limiter));
                    manager.register_provider(
                        provider_config.exchange_id.clone(),
                        Arc::new(provider),
                    );
                }
                other => {
                    tracing::warn!(
                        exchange = other,
                        "exchange provider not yet implemented, skipping"
                    );
                }
            }
        }

        self.manager = Some(manager);
        tracing::info!("exchange module loaded");
        Ok(())
    }

    async fn on_start(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
        let manager = self
            .manager
            .as_mut()
            .ok_or_else(|| MtwError::Internal("exchange manager not initialized".into()))?;

        // Take the event receiver and spawn forwarding task
        if let Some(mut event_rx) = manager.take_event_receiver() {
            let handle = tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    let (channel, _msg) = exchange_event_to_message(event);
                    tracing::debug!(channel = %channel, "exchange event");
                    // Channel publishing will be connected when used with MtwRouter
                    // via SharedState or direct ChannelManager reference
                }
            });
            self.event_task = Some(handle);
        }

        tracing::info!("exchange module started");
        Ok(())
    }

    async fn on_stop(&mut self, _ctx: &ModuleContext) -> Result<(), MtwError> {
        if let Some(handle) = self.event_task.take() {
            handle.abort();
        }
        if let Some(manager) = &self.manager {
            manager.shutdown().await;
        }
        tracing::info!("exchange module stopped");
        Ok(())
    }

    async fn health(&self) -> HealthStatus {
        match &self.manager {
            Some(m) if m.provider_count() > 0 => HealthStatus::Healthy,
            Some(_) => HealthStatus::Degraded("no providers configured".into()),
            None => HealthStatus::Unhealthy("not initialized".into()),
        }
    }
}

/// Convert an ExchangeEvent into a channel name and MtwMessage.
///
/// Channel naming convention:
/// - `exchange.{exchange_id}.ticker.{symbol}` — ticker updates
/// - `exchange.{exchange_id}.candles.{symbol}.{timeframe}` — candle updates
/// - `exchange.{exchange_id}.book.{symbol}` — order book updates
/// - `exchange.{exchange_id}.order.{symbol}` — order status updates
/// - `exchange.{exchange_id}.status` — connect/disconnect/error
fn exchange_event_to_message(event: ExchangeEvent) -> (String, MtwMessage) {
    match event {
        ExchangeEvent::TickerUpdate { exchange, ticker } => {
            let channel = format!("exchange.{}.ticker.{}", exchange, ticker.symbol).to_lowercase();
            let msg = MtwMessage::new(
                MsgType::Event,
                Payload::Json(serde_json::to_value(&ticker).unwrap_or_default()),
            )
            .with_channel(&channel);
            (channel, msg)
        }
        ExchangeEvent::CandleUpdate {
            exchange,
            symbol,
            timeframe,
            candle,
        } => {
            let channel =
                format!("exchange.{}.candles.{}.{}", exchange, symbol, timeframe).to_lowercase();
            let msg = MtwMessage::new(
                MsgType::Event,
                Payload::Json(serde_json::to_value(&candle).unwrap_or_default()),
            )
            .with_channel(&channel);
            (channel, msg)
        }
        ExchangeEvent::OrderBookUpdate { exchange, book } => {
            let channel = format!("exchange.{}.book.{}", exchange, book.symbol).to_lowercase();
            let msg = MtwMessage::new(
                MsgType::Event,
                Payload::Json(serde_json::to_value(&book).unwrap_or_default()),
            )
            .with_channel(&channel);
            (channel, msg)
        }
        ExchangeEvent::OrderUpdate { exchange, order } => {
            let channel = format!("exchange.{}.order.{}", exchange, order.symbol).to_lowercase();
            let msg = MtwMessage::new(
                MsgType::Event,
                Payload::Json(serde_json::to_value(&order).unwrap_or_default()),
            )
            .with_channel(&channel);
            (channel, msg)
        }
        ExchangeEvent::Connected { exchange } => {
            let channel = format!("exchange.{}.status", exchange).to_lowercase();
            let msg = MtwMessage::event("connected").with_channel(&channel);
            (channel, msg)
        }
        ExchangeEvent::Disconnected { exchange, reason } => {
            let channel = format!("exchange.{}.status", exchange).to_lowercase();
            let msg =
                MtwMessage::event(format!("disconnected: {}", reason)).with_channel(&channel);
            (channel, msg)
        }
        ExchangeEvent::Error { exchange, message } => {
            let channel = format!("exchange.{}.status", exchange).to_lowercase();
            let msg = MtwMessage::error(500, message).with_channel(&channel);
            (channel, msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exchange_event_to_message_ticker() {
        let event = ExchangeEvent::TickerUpdate {
            exchange: ExchangeId::Bitvavo,
            ticker: Ticker {
                symbol: "BTC-EUR".into(),
                bid: 95000.0,
                ask: 95010.0,
                last: 95005.0,
                volume: 1234.5,
                change_24h: 2.5,
                timestamp: 1711000000000,
            },
        };
        let (channel, msg) = exchange_event_to_message(event);
        assert_eq!(channel, "exchange.bitvavo.ticker.btc-eur");
        assert_eq!(msg.msg_type, MsgType::Event);
    }

    #[test]
    fn test_exchange_event_to_message_candle() {
        let event = ExchangeEvent::CandleUpdate {
            exchange: ExchangeId::Bitvavo,
            symbol: "ETH-EUR".into(),
            timeframe: "1h".into(),
            candle: Candle {
                timestamp: 1711000000000,
                open: 3000.0,
                high: 3100.0,
                low: 2950.0,
                close: 3050.0,
                volume: 500.0,
            },
        };
        let (channel, _msg) = exchange_event_to_message(event);
        assert_eq!(channel, "exchange.bitvavo.candles.eth-eur.1h");
    }

    #[test]
    fn test_exchange_event_to_message_status() {
        let event = ExchangeEvent::Connected {
            exchange: ExchangeId::Kraken,
        };
        let (channel, msg) = exchange_event_to_message(event);
        assert_eq!(channel, "exchange.kraken.status");
        assert_eq!(msg.msg_type, MsgType::Event);
    }

    #[test]
    fn test_exchange_module_default() {
        let module = ExchangeModule::default();
        assert_eq!(module.manifest().name, "mtw-exchange");
        assert_eq!(module.manifest().module_type, ModuleType::Integration);
        assert!(module.manager().is_none());
    }
}
