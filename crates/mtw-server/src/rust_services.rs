//! Rust-native services exposed as bridge tools for mtwKernel.
//!
//! Registers trading formulas, trade monitor, and rate limiting as
//! tools that external processes (e.g., mtwKernel in TypeScript) can
//! call over the Unix socket bridge.

use mtw_bridge::server::BridgeServer;
use mtw_trading::formula::FormulaRegistry;
use mtw_trading::formulas;
use mtw_trading::monitor::TradeMonitor;
use mtw_trading::types::OrderSide;
use mtw_security::rate_limit::RateLimiter;
use mtw_ai::provider::{CompletionRequest, Message, MessageRole, MtwAIProvider};
use mtw_ai::providers::openai::{OpenAIConfig, OpenAIProvider};
use mtw_ai::providers::anthropic::{AnthropicConfig, AnthropicProvider};
use mtw_ai::providers::ollama::{OllamaConfig, OllamaProvider};
use mtw_ai::providers::lmstudio::{LMStudioConfig, LMStudioProvider};
use std::sync::Arc;

/// Holds the heavy-compute Rust services that are registered with the BridgeServer.
pub struct RustServices {
    pub formula_registry: Arc<FormulaRegistry>,
    pub trade_monitor: Arc<TradeMonitor>,
    pub rate_limiter: Arc<RateLimiter>,
    pub ai_provider: Arc<dyn MtwAIProvider>,
}

impl RustServices {
    /// Create a new `RustServices` with all built-in formulas registered.
    /// AI provider is selected from environment variables.
    pub fn new() -> Self {
        let mut formula_registry = FormulaRegistry::new();
        formulas::register_all(&mut formula_registry);

        // Select AI provider from env
        let ai_provider: Arc<dyn MtwAIProvider> = Self::create_ai_provider();

        Self {
            formula_registry: Arc::new(formula_registry),
            trade_monitor: Arc::new(TradeMonitor::new()),
            rate_limiter: Arc::new(RateLimiter::default()),
            ai_provider,
        }
    }

    fn create_ai_provider() -> Arc<dyn MtwAIProvider> {
        let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "openai".to_string());
        match provider.as_str() {
            "anthropic" => {
                let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
                let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string());
                tracing::info!(provider = "anthropic", model = %model, "AI provider initialized");
                Arc::new(AnthropicProvider::new(AnthropicConfig {
                    api_key: key,
                    base_url: "https://api.anthropic.com".to_string(),
                    default_model: model,
                }))
            }
            "ollama" => {
                let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());
                let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "llama3".to_string());
                tracing::info!(provider = "ollama", url = %url, model = %model, "AI provider initialized");
                Arc::new(OllamaProvider::new(OllamaConfig {
                    base_url: url,
                    default_model: model,
                }))
            }
            "lmstudio" => {
                let url = std::env::var("LMSTUDIO_URL").unwrap_or_else(|_| "http://localhost:1234/v1".to_string());
                let model = std::env::var("LLM_MODEL").unwrap_or_default();
                tracing::info!(provider = "lmstudio", url = %url, "AI provider initialized");
                Arc::new(LMStudioProvider::new(LMStudioConfig {
                    base_url: url,
                    default_model: model,
                    api_key: None,
                }))
            }
            _ => {
                // Default: OpenAI
                let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
                let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                tracing::info!(provider = "openai", model = %model, "AI provider initialized");
                Arc::new(OpenAIProvider::new(OpenAIConfig {
                    api_key: key,
                    base_url,
                    default_model: model,
                }))
            }
        }
    }

    /// Register all tools with the given bridge server.
    pub fn register_all(&self, server: &BridgeServer) {
        self.register_trading_tools(server);
        self.register_security_tools(server);
        self.register_ai_tools(server);
    }

    fn register_trading_tools(&self, server: &BridgeServer) {
        // 1. trading.compute_formulas
        //    Args: { "candles": [{timestamp, open, high, low, close, volume}...], "symbol": "BTC/USDT" }
        //    Returns: { "results": [...], "consensus": {...} }
        let reg = self.formula_registry.clone();
        server.register_tool(
            "trading.compute_formulas",
            Arc::new(move |args| {
                let reg = reg.clone();
                Box::pin(async move {
                    let candles_val = args
                        .get("candles")
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing candles".into()))?;
                    let candles: Vec<mtw_trading::types::Candle> =
                        serde_json::from_value(candles_val.clone()).map_err(|e| {
                            mtw_core::MtwError::Internal(format!("invalid candles: {}", e))
                        })?;
                    let symbol = args
                        .get("symbol")
                        .and_then(|v| v.as_str())
                        .unwrap_or("UNKNOWN");

                    let results = reg.compute_all(&candles, None);
                    let consensus = reg.consensus(symbol, &candles, None);

                    let results_json: Vec<serde_json::Value> = results
                        .iter()
                        .map(|(id, r)| {
                            serde_json::json!({
                                "id": id,
                                "side": r.side.map(|s| match s {
                                    OrderSide::Buy => "buy",
                                    OrderSide::Sell => "sell",
                                }),
                                "confidence": r.confidence,
                                "indicators": r.indicators,
                                "reasoning": r.reasoning,
                            })
                        })
                        .collect();

                    Ok(serde_json::json!({
                        "results": results_json,
                        "consensus": {
                            "symbol": consensus.symbol,
                            "side": consensus.side.map(|s| match s {
                                OrderSide::Buy => "buy",
                                OrderSide::Sell => "sell",
                            }),
                            "avg_confidence": consensus.avg_confidence,
                            "formula_count": consensus.formula_count,
                            "formulas": consensus.formulas,
                            "total_checked": consensus.total_checked,
                        }
                    }))
                })
            }),
        );

        // 2. trading.monitor.add_position
        let mon = self.trade_monitor.clone();
        server.register_tool(
            "trading.monitor.add_position",
            Arc::new(move |args| {
                let mon = mon.clone();
                Box::pin(async move {
                    let pos: mtw_trading::monitor::MonitoredPosition =
                        serde_json::from_value(args).map_err(|e| {
                            mtw_core::MtwError::Internal(format!("invalid position: {}", e))
                        })?;
                    mon.add_position(pos);
                    Ok(serde_json::json!({"ok": true}))
                })
            }),
        );

        // 3. trading.monitor.check
        let mon = self.trade_monitor.clone();
        server.register_tool(
            "trading.monitor.check",
            Arc::new(move |args| {
                let mon = mon.clone();
                Box::pin(async move {
                    let trade_id = args
                        .get("trade_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            mtw_core::MtwError::Internal("missing trade_id".into())
                        })?;
                    let price = args
                        .get("current_price")
                        .and_then(|v| v.as_f64())
                        .ok_or_else(|| {
                            mtw_core::MtwError::Internal("missing current_price".into())
                        })?;

                    mon.update_price(trade_id, price);
                    let signal = mon.check_position(trade_id, price);

                    Ok(match signal {
                        Some(s) => serde_json::to_value(&s).unwrap_or(serde_json::json!(null)),
                        None => serde_json::json!(null),
                    })
                })
            }),
        );

        // 4. trading.monitor.check_all
        let mon = self.trade_monitor.clone();
        server.register_tool(
            "trading.monitor.check_all",
            Arc::new(move |args| {
                let mon = mon.clone();
                Box::pin(async move {
                    let prices: std::collections::HashMap<String, f64> =
                        serde_json::from_value(
                            args.get("prices")
                                .cloned()
                                .unwrap_or(serde_json::json!({})),
                        )
                        .map_err(|e| {
                            mtw_core::MtwError::Internal(format!("invalid prices: {}", e))
                        })?;

                    for (symbol, price) in &prices {
                        mon.update_price(symbol, *price);
                    }

                    let signals = mon.check_all(&prices);
                    Ok(serde_json::to_value(&signals).unwrap_or(serde_json::json!([])))
                })
            }),
        );

        // 5. trading.monitor.remove
        let mon = self.trade_monitor.clone();
        server.register_tool(
            "trading.monitor.remove",
            Arc::new(move |args| {
                let mon = mon.clone();
                Box::pin(async move {
                    let trade_id = args
                        .get("trade_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            mtw_core::MtwError::Internal("missing trade_id".into())
                        })?;
                    mon.remove_position(trade_id);
                    Ok(serde_json::json!({"ok": true}))
                })
            }),
        );

        // 6. trading.calculate_pnl
        server.register_tool(
            "trading.calculate_pnl",
            Arc::new(|args| {
                Box::pin(async move {
                    let entry = args.get("entry").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let exit = args.get("exit").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let side_str = args
                        .get("side")
                        .and_then(|v| v.as_str())
                        .unwrap_or("buy");
                    let fee_rate = args
                        .get("fee_rate")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.001);

                    let side = if side_str == "sell" {
                        OrderSide::Sell
                    } else {
                        OrderSide::Buy
                    };
                    let pnl = TradeMonitor::calculate_pnl(entry, exit, amount, side, fee_rate);
                    Ok(serde_json::json!({"pnl": pnl}))
                })
            }),
        );
    }

    fn register_security_tools(&self, server: &BridgeServer) {
        // 1. security.rate_limit.consume
        let rl = self.rate_limiter.clone();
        server.register_tool(
            "security.rate_limit.consume",
            Arc::new(move |args| {
                let rl = rl.clone();
                Box::pin(async move {
                    let key = args
                        .get("key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing key".into()))?;
                    let allowed = rl.consume(key).is_ok();
                    let status = rl.get_status(key);
                    Ok(serde_json::json!({
                        "allowed": allowed,
                        "remaining": status.remaining,
                        "blocked": status.blocked,
                    }))
                })
            }),
        );

        // 2. security.rate_limit.check
        let rl = self.rate_limiter.clone();
        server.register_tool(
            "security.rate_limit.check",
            Arc::new(move |args| {
                let rl = rl.clone();
                Box::pin(async move {
                    let key = args
                        .get("key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing key".into()))?;
                    let allowed = rl.check(key);
                    Ok(serde_json::json!({"allowed": allowed}))
                })
            }),
        );

        // 3. security.rate_limit.status
        let rl = self.rate_limiter.clone();
        server.register_tool(
            "security.rate_limit.status",
            Arc::new(move |args| {
                let rl = rl.clone();
                Box::pin(async move {
                    let key = args
                        .get("key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing key".into()))?;
                    let status = rl.get_status(key);
                    Ok(serde_json::to_value(&status).unwrap_or(serde_json::json!(null)))
                })
            }),
        );

        // 4. _health
        server.register_tool(
            "_health",
            Arc::new(|_| {
                Box::pin(async { Ok(serde_json::json!({"ok": true, "service": "mtwRequest"})) })
            }),
        );
    }

    fn register_ai_tools(&self, server: &BridgeServer) {
        // llm.chat — unified LLM completion via configured provider
        // Args: { "system": "...", "user": "...", "model": "gpt-4o-mini", "max_tokens": 2048 }
        // Returns: { "text": "...", "model": "...", "provider": "openai", "usage": {...} }
        let provider = self.ai_provider.clone();
        server.register_tool(
            "llm.chat",
            Arc::new(move |args| {
                let provider = provider.clone();
                Box::pin(async move {
                    let system = args.get("system").and_then(|v| v.as_str()).unwrap_or("");
                    let user = args
                        .get("user")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing 'user' message".into()))?;
                    let model = args
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let max_tokens = args
                        .get("max_tokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    let temperature = args
                        .get("temperature")
                        .and_then(|v| v.as_f64())
                        .map(|v| v as f32);

                    let mut messages = Vec::new();
                    if !system.is_empty() {
                        messages.push(Message::system(system));
                    }
                    messages.push(Message::user(user));

                    let req = CompletionRequest {
                        model: if model.is_empty() {
                            String::new() // provider will use default
                        } else {
                            model
                        },
                        messages,
                        tools: None,
                        temperature,
                        max_tokens,
                        metadata: Default::default(),
                    };

                    let response = provider.complete(req).await.map_err(|e| {
                        mtw_core::MtwError::Internal(format!("LLM error: {}", e))
                    })?;

                    Ok(serde_json::json!({
                        "text": response.content,
                        "model": response.model,
                        "provider": provider.name(),
                        "usage": {
                            "input_tokens": response.usage.prompt_tokens,
                            "output_tokens": response.usage.completion_tokens,
                            "total_tokens": response.usage.total_tokens,
                        }
                    }))
                })
            }),
        );
    }
}
