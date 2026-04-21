//! Rust-native services exposed as bridge tools for mtwKernel.
//!
//! Registers trading formulas, trade monitor, and rate limiting as
//! tools that external processes (e.g., mtwKernel in TypeScript) can
//! call over the Unix socket bridge.

use dashmap::DashMap;
use mtw_bridge::server::BridgeServer;
use mtw_trading::formula::FormulaRegistry;
use mtw_trading::formulas;
use mtw_trading::monitor::TradeMonitor;
use mtw_trading::types::OrderSide;
use mtw_security::rate_limit::RateLimiter;
use mtw_ai::provider::{CompletionRequest, Message, MtwAIProvider};
use mtw_ai::providers::openai::{OpenAIConfig, OpenAIProvider};
use mtw_ai::providers::anthropic::{AnthropicConfig, AnthropicProvider};
use mtw_ai::providers::ollama::{OllamaConfig, OllamaProvider};
use mtw_ai::providers::lmstudio::{LMStudioConfig, LMStudioProvider};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Holds the heavy-compute Rust services that are registered with the BridgeServer.
pub struct RustServices {
    pub formula_registry: Arc<FormulaRegistry>,
    pub trade_monitor: Arc<TradeMonitor>,
    pub rate_limiter: Arc<RateLimiter>,
    /// Named LLM providers. `llm.chat` picks by `provider` arg; `credentials.set`
    /// hot-registers new ones pushed by mtwKernel over the bridge.
    pub providers: Arc<DashMap<String, Arc<dyn MtwAIProvider>>>,
    /// Provider used when no explicit `provider` arg is given to `llm.chat`.
    pub default_provider: String,
    /// Concurrency limiters per provider. Local models (lmstudio, ollama) get 1 permit
    /// (serial queue). Cloud providers (openai, anthropic) get 5 concurrent.
    pub provider_semaphores: Arc<DashMap<String, Arc<Semaphore>>>,
}

impl RustServices {
    /// Create a new `RustServices` with all built-in formulas and
    /// providers seeded from environment variables.
    pub fn new() -> Self {
        let mut formula_registry = FormulaRegistry::new();
        formulas::register_all(&mut formula_registry);

        let providers: Arc<DashMap<String, Arc<dyn MtwAIProvider>>> =
            Arc::new(DashMap::new());

        // Seed providers from env (backwards-compatible with existing docker-compose).
        let default_provider = Self::seed_providers_from_env(&providers);

        // Create concurrency semaphores: 1 for local models, 5 for cloud
        let provider_semaphores: Arc<DashMap<String, Arc<Semaphore>>> = Arc::new(DashMap::new());
        provider_semaphores.insert("lmstudio".into(), Arc::new(Semaphore::new(1)));
        provider_semaphores.insert("ollama".into(), Arc::new(Semaphore::new(1)));
        provider_semaphores.insert("openai".into(), Arc::new(Semaphore::new(5)));
        provider_semaphores.insert("anthropic".into(), Arc::new(Semaphore::new(5)));

        Self {
            formula_registry: Arc::new(formula_registry),
            trade_monitor: Arc::new(TradeMonitor::new()),
            rate_limiter: Arc::new(RateLimiter::default()),
            providers,
            default_provider,
            provider_semaphores,
        }
    }

    /// Seed the provider map from environment variables.
    /// Returns the name of the default provider.
    fn seed_providers_from_env(
        providers: &DashMap<String, Arc<dyn MtwAIProvider>>,
    ) -> String {
        let chosen = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "openai".to_string());

        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                let model = std::env::var("LLM_MODEL")
                    .unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string());
                tracing::info!(provider = "anthropic", model = %model, "seeded from env");
                providers.insert(
                    "anthropic".into(),
                    Arc::new(AnthropicProvider::new(AnthropicConfig {
                        api_key: key,
                        base_url: "https://api.anthropic.com".to_string(),
                        default_model: model,
                    })),
                );
            }
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                let model = std::env::var("LLM_MODEL")
                    .unwrap_or_else(|_| "gpt-4o-mini".to_string());
                let base_url = std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                tracing::info!(provider = "openai", model = %model, "seeded from env");
                providers.insert(
                    "openai".into(),
                    Arc::new(OpenAIProvider::new(OpenAIConfig {
                        api_key: key,
                        base_url,
                        default_model: model,
                    })),
                );
            }
        }

        let ollama_url = std::env::var("OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let ollama_model = if chosen == "ollama" {
            std::env::var("LLM_MODEL").unwrap_or_else(|_| "llama3".to_string())
        } else {
            "llama3".to_string()
        };
        providers.insert(
            "ollama".into(),
            Arc::new(OllamaProvider::new(OllamaConfig {
                base_url: ollama_url,
                default_model: ollama_model,
            })),
        );

        if let Ok(url) = std::env::var("LMSTUDIO_URL") {
            if !url.is_empty() {
                providers.insert(
                    "lmstudio".into(),
                    Arc::new(LMStudioProvider::new(LMStudioConfig {
                        base_url: url,
                        default_model: std::env::var("LLM_MODEL").unwrap_or_default(),
                        api_key: None,
                    })),
                );
            }
        }

        chosen
    }

    /// Register all tools with the given bridge server.
    ///
    /// `kernel_bridge` is the outgoing bridge client that connects to the
    /// kernel's bridge-server (`/tmp/mtw-kernel.sock`). When an agent calls
    /// a tool the Rust side doesn't have locally, the executor forwards the
    /// call to the kernel through this bridge.
    pub fn register_all(
        &self,
        server: &BridgeServer,
        kernel_bridge: Option<Arc<dyn mtw_bridge::MtwBridge>>,
    ) {
        self.register_trading_tools(server);
        self.register_security_tools(server);
        self.register_ai_tools(server);
        self.register_agent_tools(server, kernel_bridge);
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
        // credentials.set — hot-register LLM providers pushed by mtwKernel.
        // Args: { "providers": [{ "name": "anthropic", "api_key": "...", "model": "...", "base_url": "..." }] }
        let providers = self.providers.clone();
        server.register_tool(
            "credentials.set",
            Arc::new(move |args| {
                let providers = providers.clone();
                Box::pin(async move {
                    let list = args
                        .get("providers")
                        .and_then(|v| v.as_array())
                        .ok_or_else(|| {
                            mtw_core::MtwError::Internal("missing 'providers' array".into())
                        })?;

                    let mut registered = Vec::new();
                    for entry in list {
                        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let api_key = entry.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
                        let model = entry.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let base_url = entry.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();

                        if name.is_empty() || api_key.is_empty() {
                            continue;
                        }

                        let provider: Arc<dyn MtwAIProvider> = match name {
                            "anthropic" => Arc::new(AnthropicProvider::new(AnthropicConfig {
                                api_key: api_key.to_string(),
                                base_url: if base_url.is_empty() {
                                    "https://api.anthropic.com".to_string()
                                } else {
                                    base_url
                                },
                                default_model: if model.is_empty() {
                                    "claude-haiku-4-5-20251001".to_string()
                                } else {
                                    model
                                },
                            })),
                            "openai" => Arc::new(OpenAIProvider::new(OpenAIConfig {
                                api_key: api_key.to_string(),
                                base_url: if base_url.is_empty() {
                                    "https://api.openai.com/v1".to_string()
                                } else {
                                    base_url
                                },
                                default_model: if model.is_empty() {
                                    "gpt-4o-mini".to_string()
                                } else {
                                    model
                                },
                            })),
                            "ollama" => Arc::new(OllamaProvider::new(OllamaConfig {
                                base_url: if base_url.is_empty() {
                                    "http://localhost:11434".to_string()
                                } else {
                                    base_url
                                },
                                default_model: if model.is_empty() {
                                    "llama3".to_string()
                                } else {
                                    model
                                },
                            })),
                            "lmstudio" => Arc::new(LMStudioProvider::new(LMStudioConfig {
                                base_url: if base_url.is_empty() {
                                    "http://localhost:1234/v1".to_string()
                                } else {
                                    base_url
                                },
                                default_model: model,
                                api_key: Some(api_key.to_string()),
                            })),
                            _ => continue,
                        };

                        tracing::info!(provider = %name, "credentials.set: registered");
                        providers.insert(name.to_string(), provider);
                        registered.push(name.to_string());
                    }

                    Ok(serde_json::json!({
                        "registered": registered,
                        "total_providers": providers.len(),
                    }))
                })
            }),
        );

        // llm.chat — unified LLM completion with per-provider queue.
        // Local models (lmstudio) get serial access (1 at a time).
        // Cloud providers get 5 concurrent requests.
        let providers = self.providers.clone();
        let default_name = self.default_provider.clone();
        let semaphores = self.provider_semaphores.clone();
        server.register_tool(
            "llm.chat",
            Arc::new(move |args| {
                let providers = providers.clone();
                let default_name = default_name.clone();
                let semaphores = semaphores.clone();
                Box::pin(async move {
                    let provider_name = args.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                    let key = if provider_name.is_empty() { &default_name } else { provider_name };
                    let provider = providers
                        .get(key)
                        .map(|r| Arc::clone(r.value()))
                        .ok_or_else(|| {
                            mtw_core::MtwError::Internal(format!(
                                "provider '{}' not registered (available: {:?})",
                                key,
                                providers.iter().map(|e| e.key().clone()).collect::<Vec<_>>(),
                            ))
                        })?;

                    // Acquire semaphore — queues requests for local models
                    let sem = semaphores
                        .entry(key.to_string())
                        .or_insert_with(|| Arc::new(Semaphore::new(1)))
                        .clone();
                    let _permit = sem.acquire().await.map_err(|_| {
                        mtw_core::MtwError::Internal("semaphore closed".into())
                    })?;

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
                        model: if model.is_empty() { String::new() } else { model },
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

    fn register_agent_tools(
        &self,
        server: &BridgeServer,
        kernel_bridge: Option<Arc<dyn mtw_bridge::MtwBridge>>,
    ) {
        use mtw_ai::executor::{AgentConfig, ExecutionConfig, ExecutorEngine, MtwAgentExecutor, ToolDefinition};
        use mtw_ai::store::{AgentStore, AgentStoreConfig};
        use mtw_ai::store_sqlite::SqliteAgentStore;

        // Build an ExecutorEngine that shares our providers and can call kernel tools via bridge.
        let engine = Arc::new(ExecutorEngine::new(self.default_provider.clone()));

        // Share the same providers the bridge already has (incl. credentials.set updates).
        for entry in self.providers.iter() {
            engine.register_provider(Arc::clone(entry.value()));
        }

        // If a kernel bridge is available, register a wildcard tool forwarder:
        // any tool_name not locally registered gets forwarded to the kernel.
        if let Some(kb) = kernel_bridge.clone() {
            // We register a special "_remote_" prefixed tool that the executor won't
            // know about. Instead, we override invoke_tool behavior by registering
            // every kernel tool on-demand via a catch-all approach.
            // Simpler: register a single tool `_forward` and have the executor use it.
            // But cleanest: before running, fetch the tool list from the kernel and
            // register proxy definitions.
            //
            // For now: we'll list available tools from kernel at registration time
            // (one _health call is enough to know the bridge works). Actual tool
            // forwarding happens in `agents.run` handler below where we build a
            // fresh engine per run with dynamic proxy tools.
            let _ = kb; // used below in agents.run closure
        }

        // Open agent store (if MTW_AGENTS_DB is set or default path exists).
        let store: Option<Arc<dyn AgentStore>> = {
            let path = std::env::var("MTW_AGENTS_DB")
                .unwrap_or_else(|_| "./data/agents.db".to_string());
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match SqliteAgentStore::open(&AgentStoreConfig {
                path: path.clone(),
                ..Default::default()
            }) {
                Ok(s) => {
                    tracing::info!(path = %path, "agent store opened for bridge");
                    Some(Arc::new(s) as Arc<dyn AgentStore>)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "agent store not available, agents.run disabled");
                    None
                }
            }
        };

        // agents.run — execute an agent end-to-end.
        // Builds a fresh ExecutorEngine per run, with proxy tools that forward
        // to the kernel via bridge for any tool the Rust side doesn't have locally.
        let providers = self.providers.clone();
        let default_provider = self.default_provider.clone();
        let kernel_bridge = kernel_bridge.clone();
        let store_for_run = store.clone();
        server.register_tool(
            "agents.run",
            Arc::new(move |args| {
                let providers = providers.clone();
                let default_provider = default_provider.clone();
                let kb = kernel_bridge.clone();
                let store = store_for_run.clone();
                Box::pin(async move {
                    let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing 'agent_id'".into()))?
                        .to_string();
                    let goal = args.get("goal").and_then(|v| v.as_str())
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing 'goal'".into()))?
                        .to_string();

                    let mut config = ExecutionConfig::default();
                    if let Some(v) = args.get("max_iterations").and_then(|v| v.as_u64()) {
                        config.max_iterations = v as u32;
                    }
                    if let Some(v) = args.get("timeout_ms").and_then(|v| v.as_u64()) {
                        config.timeout_ms = v;
                    }

                    // Build engine with shared providers + store.
                    let mut engine = ExecutorEngine::new(default_provider);
                    for entry in providers.iter() {
                        engine.register_provider(Arc::clone(entry.value()));
                    }
                    if let Some(s) = &store {
                        engine = engine.with_store(Arc::clone(s));
                    }

                    // Resolve agent config: store first, then inline args as fallback.
                    let from_store = if let Some(s) = &store {
                        s.get_agent(&agent_id).await.ok().flatten()
                    } else {
                        None
                    };
                    let ac = from_store.unwrap_or_else(|| {
                        let tool_names: Vec<String> = args.get("tool_names")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                            .unwrap_or_default();
                        AgentConfig {
                            id: agent_id.clone(),
                            name: args.get("name").and_then(|v| v.as_str()).unwrap_or(&agent_id).to_string(),
                            provider: args.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            model: args.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            system_prompt: args.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            tool_names,
                            token_budget: args.get("token_budget").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        }
                    });

                    // Register proxy tools: for each tool_name, create a forwarder to kernel.
                    if let Some(ref kb) = kb {
                        for tool_name in &ac.tool_names {
                            let kb_clone = Arc::clone(kb);
                            let name = tool_name.clone();
                            engine.register_tool(ToolDefinition {
                                name: name.clone(),
                                description: format!("Proxy to kernel tool '{}'", name),
                                parameters: serde_json::json!({"type": "object"}),
                                handler: Arc::new(move |tool_args| {
                                    let kb = kb_clone.clone();
                                    let tool = name.clone();
                                    Box::pin(async move {
                                        kb.call_tool(&tool, tool_args).await
                                    })
                                }),
                            });
                        }
                    }
                    engine.register_agent_config(ac);

                    let result = engine.execute(&agent_id, &goal, &config).await?;

                    Ok(serde_json::json!({
                        "agent_id": agent_id,
                        "status": result.status,
                        "result": result.result,
                        "error": result.error,
                        "steps_count": result.steps_count,
                        "tokens_used": result.tokens_used,
                    }))
                })
            }),
        );

        // agents.create — persist an agent config.
        let store_for_create = store.clone();
        server.register_tool(
            "agents.create",
            Arc::new(move |args| {
                let store = store_for_create.clone();
                Box::pin(async move {
                    let store = store.as_ref().ok_or_else(|| {
                        mtw_core::MtwError::Internal("agent store not available".into())
                    })?;
                    let id = args.get("id").and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| ulid::Ulid::new().to_string());
                    let name = args.get("name").and_then(|v| v.as_str())
                        .ok_or_else(|| mtw_core::MtwError::Internal("missing 'name'".into()))?
                        .to_string();
                    let tool_names: Vec<String> = args.get("tool_names")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();

                    let agent = AgentConfig {
                        id: id.clone(),
                        name,
                        provider: args.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        model: args.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        system_prompt: args.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        tool_names,
                        token_budget: args.get("token_budget").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    };
                    store.save_agent(&agent).await?;
                    Ok(serde_json::json!({"id": id, "status": "created"}))
                })
            }),
        );

        // agents.list — list all persisted agents.
        let store_for_list = store.clone();
        server.register_tool(
            "agents.list",
            Arc::new(move |_args| {
                let store = store_for_list.clone();
                Box::pin(async move {
                    let store = store.as_ref().ok_or_else(|| {
                        mtw_core::MtwError::Internal("agent store not available".into())
                    })?;
                    let agents = store.list_agents().await?;
                    Ok(serde_json::json!({"agents": agents, "total": agents.len()}))
                })
            }),
        );
    }
}
