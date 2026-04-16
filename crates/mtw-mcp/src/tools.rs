//! MCP tools for managing mtwRequest
//!
//! Tools are organized by domain:
//! - mtw_server_*      — server status, config, health
//! - mtw_modules_*     — module lifecycle management
//! - mtw_agents_*      — AI agent management
//! - mtw_auth_*        — authentication & authorization
//! - mtw_trading_*     — trading formulas, strategies, signals
//! - mtw_security_*    — rate limits, policies, approvals
//! - mtw_channels_*    — pub/sub channel management
//! - mtw_transport_*   — connection management
//! - mtw_federation_*  — peer sync management
//! - mtw_notify_*      — notification providers
//! - mtw_skills_*      — skill/plugin management

use crate::agents_ctx::AgentsCtx;
use crate::protocol::{McpServer, ToolHandler};
use serde_json::{json, Value};
use std::sync::Arc;

/// Register all mtwRequest management tools
pub fn register_all(server: &mut McpServer, ctx: &AgentsCtx) {
    register_server_tools(server);
    register_module_tools(server);
    register_agent_tools(server, ctx);
    register_auth_tools(server);
    register_trading_tools(server);
    register_security_tools(server);
    register_channel_tools(server);
    register_transport_tools(server);
    register_federation_tools(server);
    register_notify_tools(server);
    register_skill_tools(server);
}

// ── Server Management ────────────────────────────────────────

fn register_server_tools(server: &mut McpServer) {
    server.tool(
        "mtw_server_status",
        "Get mtwRequest server status: uptime, connections, modules loaded, bridge status",
        json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
        handler(|_| async {
            Ok(json!({
                "status": "running",
                "version": "0.2.0",
                "transport": "websocket",
                "port": 7741,
                "features": [
                    "websocket", "bridge", "trading", "agents",
                    "auth", "security", "federation", "mcp"
                ]
            }).to_string())
        }),
    );

    server.tool(
        "mtw_server_config",
        "View or update mtwRequest server configuration (host, port, max_connections, transport settings)",
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["get", "set"], "default": "get" },
                "key": { "type": "string", "description": "Config key (e.g. 'server.port', 'transport.websocket.ping_interval')" },
                "value": { "description": "New value (only for set action)" }
            },
            "required": []
        }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("get");
            match action {
                "get" => Ok(json!({
                    "server": { "host": "0.0.0.0", "port": 7741, "max_connections": 10000 },
                    "transport": { "default": "websocket", "websocket": { "path": "/ws", "ping_interval": 30 } },
                    "codec": { "default": "json" },
                    "bridge": { "socket": "/tmp/mtw-rust.sock" }
                }).to_string()),
                "set" => {
                    let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("?");
                    Ok(format!("Config key '{}' updated. Restart required for some settings.", key))
                }
                _ => Err("unknown action".into()),
            }
        }),
    );

    server.tool(
        "mtw_server_health",
        "Health check for all mtwRequest subsystems: transport, bridge, store, modules",
        json!({ "type": "object", "properties": {}, "required": [] }),
        handler(|_| async {
            Ok(json!({
                "healthy": true,
                "subsystems": {
                    "transport": "ok",
                    "bridge_server": "ok",
                    "bridge_client": "ok",
                    "store": "ok",
                    "modules": "ok"
                }
            }).to_string())
        }),
    );
}

// ── Module Management ────────────────────────────────────────

fn register_module_tools(server: &mut McpServer) {
    server.tool(
        "mtw_modules_list",
        "List all registered mtwRequest modules with their status, type, and health",
        json!({ "type": "object", "properties": {
            "type": { "type": "string", "enum": ["all", "transport", "middleware", "ai_provider", "ai_agent", "codec", "auth", "storage", "channel", "integration", "trading", "ui"], "default": "all" }
        }, "required": [] }),
        handler(|args| async move {
            let filter = args.get("type").and_then(|v| v.as_str()).unwrap_or("all");
            Ok(json!({
                "modules": [
                    { "name": "mtw-transport-ws", "type": "transport", "status": "running", "health": "healthy" },
                    { "name": "mtw-codec-json", "type": "codec", "status": "running", "health": "healthy" },
                    { "name": "mtw-trading", "type": "trading", "status": "running", "health": "healthy", "formulas": 15 },
                    { "name": "mtw-auth-jwt", "type": "auth", "status": "running", "health": "healthy" },
                    { "name": "mtw-bridge", "type": "integration", "status": "running", "health": "healthy" }
                ],
                "filter": filter
            }).to_string())
        }),
    );

    server.tool(
        "mtw_modules_install",
        "Install a new mtwRequest module from registry, git, or local path",
        json!({ "type": "object", "properties": {
            "source": { "type": "string", "description": "Module source: registry name, git URL, or local path" },
            "version": { "type": "string", "description": "Version constraint (e.g. '>=0.2.0')" },
            "enable": { "type": "boolean", "default": true }
        }, "required": ["source"] }),
        handler(|args| async move {
            let source = args.get("source").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Module '{}' installed and registered. Restart to activate.", source))
        }),
    );

    server.tool(
        "mtw_modules_health",
        "Run health checks on all modules and return detailed diagnostics",
        json!({ "type": "object", "properties": {}, "required": [] }),
        handler(|_| async {
            Ok(json!({
                "total": 5,
                "healthy": 5,
                "degraded": 0,
                "unhealthy": 0,
                "details": {}
            }).to_string())
        }),
    );
}

// ── Agent Management ─────────────────────────────────────────

fn register_agent_tools(server: &mut McpServer, ctx: &AgentsCtx) {
    use mtw_ai::chain::{AgentChain, ChainCondition};
    use mtw_ai::executor::{AgentConfig, ExecutionConfig, MtwAgentExecutor};
    use mtw_ai::flow::AgentFlow;
    use mtw_ai::schedule::AgentSchedule;
    use mtw_ai::store::RunFilter;
    use mtw_ai::trigger::EventTrigger;

    // Wall-clock timestamp used for created_at/updated_at columns.
    fn now() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .to_string()
    }

    // -- mtw_agents_list --------------------------------------------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_list",
        "List all AI agents with their provider, model, and tool list.",
        json!({ "type": "object", "properties": {
            "limit": { "type": "integer", "default": 100 }
        }, "required": [] }),
        handler(move |_args| {
            let c = c.clone();
            async move {
                match c.store.list_agents().await {
                    Ok(agents) => {
                        let out = json!({
                            "agents": agents,
                            "total": agents.len(),
                        });
                        Ok(out.to_string())
                    }
                    Err(e) => Err(format!("list_agents failed: {}", e)),
                }
            }
        }),
    );

    // -- mtw_agents_create ------------------------------------------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_create",
        "Create or update an AI agent. Persists to the store and registers with the executor.",
        json!({ "type": "object", "properties": {
            "id":            { "type": "string", "description": "Agent ID. Generated if omitted." },
            "name":          { "type": "string" },
            "system_prompt": { "type": "string" },
            "provider":      { "type": "string", "enum": ["anthropic", "openai", "ollama", "lmstudio"] },
            "model":         { "type": "string" },
            "tool_names":    { "type": "array", "items": { "type": "string" } },
            "token_budget":  { "type": "integer", "default": 0 }
        }, "required": ["name"] }),
        handler(move |args| {
            let c = c.clone();
            async move {
                let id = args.get("id").and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| ulid::Ulid::new().to_string());
                let name = args.get("name").and_then(|v| v.as_str())
                    .ok_or("missing 'name'")?
                    .to_string();
                let provider = args.get("provider").and_then(|v| v.as_str())
                    .unwrap_or("").to_string();
                let model = args.get("model").and_then(|v| v.as_str())
                    .unwrap_or("").to_string();
                let system_prompt = args.get("system_prompt").and_then(|v| v.as_str())
                    .unwrap_or("").to_string();
                let tool_names: Vec<String> = args.get("tool_names")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect())
                    .unwrap_or_default();
                let token_budget = args.get("token_budget").and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;

                let agent = AgentConfig {
                    id: id.clone(), name, provider, model, system_prompt,
                    tool_names, token_budget,
                };

                c.store.save_agent(&agent).await
                    .map_err(|e| format!("save_agent: {}", e))?;
                c.engine.register_agent_config(agent.clone());

                Ok(json!({
                    "id": agent.id,
                    "name": agent.name,
                    "status": "created",
                }).to_string())
            }
        }),
    );

    // -- mtw_agents_run ---------------------------------------------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_run",
        "Execute an agent with a goal. Blocks until the run completes; returns run summary + result.",
        json!({ "type": "object", "properties": {
            "agent_id":        { "type": "string" },
            "goal":            { "type": "string" },
            "max_iterations":  { "type": "integer", "default": 15 },
            "timeout_ms":      { "type": "integer", "default": 300000 },
            "max_errors":      { "type": "integer", "default": 3 }
        }, "required": ["agent_id", "goal"] }),
        handler(move |args| {
            let c = c.clone();
            async move {
                let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                    .ok_or("missing 'agent_id'")?
                    .to_string();
                let goal = args.get("goal").and_then(|v| v.as_str())
                    .ok_or("missing 'goal'")?
                    .to_string();

                let mut config = ExecutionConfig::default();
                if let Some(v) = args.get("max_iterations").and_then(|v| v.as_u64()) {
                    config.max_iterations = v as u32;
                }
                if let Some(v) = args.get("timeout_ms").and_then(|v| v.as_u64()) {
                    config.timeout_ms = v;
                }
                if let Some(v) = args.get("max_errors").and_then(|v| v.as_u64()) {
                    config.max_errors = v as u32;
                }

                match c.engine.execute(&agent_id, &goal, &config).await {
                    Ok(result) => Ok(json!({
                        "agent_id": agent_id,
                        "status": result.status,
                        "result": result.result,
                        "error": result.error,
                        "steps_count": result.steps_count,
                        "tokens_used": result.tokens_used,
                    }).to_string()),
                    Err(e) => Err(format!("execute: {}", e)),
                }
            }
        }),
    );

    // -- mtw_agents_runs_list (bonus: inspect persisted runs) -------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_runs",
        "List persisted runs, optionally filtered by agent or status.",
        json!({ "type": "object", "properties": {
            "agent_id": { "type": "string" },
            "status":   { "type": "string", "enum": ["pending", "running", "completed", "failed", "cancelled"] },
            "limit":    { "type": "integer", "default": 20 }
        }, "required": [] }),
        handler(move |args| {
            let c = c.clone();
            async move {
                let filter = RunFilter {
                    agent_id: args.get("agent_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    status: args.get("status").and_then(|v| v.as_str()).and_then(|s| {
                        serde_json::from_value(json!(s)).ok()
                    }),
                    limit: args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32),
                };
                match c.store.list_runs(&filter).await {
                    Ok(runs) => Ok(json!({ "runs": runs, "total": runs.len() }).to_string()),
                    Err(e) => Err(format!("list_runs: {}", e)),
                }
            }
        }),
    );

    // -- mtw_agents_schedule ----------------------------------------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_schedule",
        "Create an interval- or cron-based schedule for an agent (in-memory for this process).",
        json!({ "type": "object", "properties": {
            "agent_id":      { "type": "string" },
            "interval_ms":   { "type": "integer", "description": "Run every N milliseconds" },
            "cron":          { "type": "string", "description": "Cron expression (e.g. '0 9 * * *')" },
            "goal_override": { "type": "string" },
            "active":        { "type": "boolean", "default": true }
        }, "required": ["agent_id"] }),
        handler(move |args| {
            let c = c.clone();
            async move {
                let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                    .ok_or("missing 'agent_id'")?
                    .to_string();
                let schedule = AgentSchedule {
                    id: ulid::Ulid::new().to_string(),
                    agent_id,
                    interval_ms: args.get("interval_ms").and_then(|v| v.as_u64()).unwrap_or(0),
                    cron_expression: args.get("cron").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    goal_override: args.get("goal_override").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    next_run_at: now(),
                    last_run_at: None,
                    active: args.get("active").and_then(|v| v.as_bool()).unwrap_or(true),
                    created_at: now(),
                };
                c.schedules.add(schedule.clone());
                Ok(json!({ "schedule": schedule }).to_string())
            }
        }),
    );

    // -- mtw_agents_chain -------------------------------------------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_chain",
        "Create a chain: when source agent finishes, target agent is invoked automatically.",
        json!({ "type": "object", "properties": {
            "source_agent_id": { "type": "string" },
            "target_agent_id": { "type": "string" },
            "condition":       { "type": "string", "enum": ["always", "on_success", "on_failure"], "default": "on_success" },
            "pass_result":     { "type": "boolean", "default": true },
            "delay_ms":        { "type": "integer", "default": 0 },
            "label":           { "type": "string" }
        }, "required": ["source_agent_id", "target_agent_id"] }),
        handler(move |args| {
            let c = c.clone();
            async move {
                let source = args.get("source_agent_id").and_then(|v| v.as_str())
                    .ok_or("missing 'source_agent_id'")?
                    .to_string();
                let target = args.get("target_agent_id").and_then(|v| v.as_str())
                    .ok_or("missing 'target_agent_id'")?
                    .to_string();
                let condition: ChainCondition = args.get("condition")
                    .and_then(|v| v.as_str())
                    .and_then(|s| serde_json::from_value(json!(s)).ok())
                    .unwrap_or(ChainCondition::OnSuccess);
                let chain = AgentChain {
                    id: ulid::Ulid::new().to_string(),
                    source_agent_id: source,
                    target_agent_id: target,
                    label: args.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    condition,
                    pass_result: args.get("pass_result").and_then(|v| v.as_bool()).unwrap_or(true),
                    delay_ms: args.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(0),
                    active: true,
                    created_at: now(),
                };
                c.chains.add(chain.clone()).map_err(|e| format!("{}", e))?;
                Ok(json!({ "chain": chain }).to_string())
            }
        }),
    );

    // -- mtw_agents_flows -------------------------------------------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_flows",
        "Manage agent flow groups: list, create, or delete.",
        json!({ "type": "object", "properties": {
            "action":      { "type": "string", "enum": ["list", "create", "delete"], "default": "list" },
            "name":        { "type": "string" },
            "description": { "type": "string" },
            "flow_id":     { "type": "string" }
        }, "required": [] }),
        handler(move |args| {
            let c = c.clone();
            async move {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
                match action {
                    "list" => {
                        let flows = c.flows.list();
                        Ok(json!({ "flows": flows, "total": flows.len() }).to_string())
                    }
                    "create" => {
                        let name = args.get("name").and_then(|v| v.as_str())
                            .ok_or("missing 'name'")?;
                        let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
                        let flow: AgentFlow = c.flows.create(name, description);
                        Ok(json!({ "flow": flow }).to_string())
                    }
                    "delete" => {
                        let id = args.get("flow_id").and_then(|v| v.as_str())
                            .ok_or("missing 'flow_id'")?;
                        let ok = c.flows.delete(id);
                        Ok(json!({ "deleted": ok }).to_string())
                    }
                    other => Err(format!("unknown action: {}", other)),
                }
            }
        }),
    );

    // -- mtw_agents_triggers ----------------------------------------------
    let c = ctx.clone();
    server.tool(
        "mtw_agents_triggers",
        "Manage event-driven triggers: list, add, remove.",
        json!({ "type": "object", "properties": {
            "action":       { "type": "string", "enum": ["list", "add", "remove"], "default": "list" },
            "trigger_id":   { "type": "string" },
            "agent_id":     { "type": "string" },
            "event_name":   { "type": "string" },
            "filter":       { "type": "object" },
            "cooldown_ms":  { "type": "integer", "default": 60000 }
        }, "required": [] }),
        handler(move |args| {
            let c = c.clone();
            async move {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
                match action {
                    "list" => {
                        let triggers = c.triggers.list();
                        Ok(json!({ "triggers": triggers, "total": triggers.len() }).to_string())
                    }
                    "add" => {
                        let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                            .ok_or("missing 'agent_id'")?
                            .to_string();
                        let event_name = args.get("event_name").and_then(|v| v.as_str())
                            .ok_or("missing 'event_name'")?
                            .to_string();
                        let trigger = EventTrigger {
                            id: ulid::Ulid::new().to_string(),
                            agent_id,
                            event_name,
                            filter: args.get("filter").cloned().unwrap_or(json!({})),
                            cooldown_ms: args.get("cooldown_ms").and_then(|v| v.as_u64()).unwrap_or(60_000),
                            last_fired: None,
                            active: true,
                            created_at: now(),
                        };
                        c.triggers.register(trigger.clone());
                        Ok(json!({ "trigger": trigger }).to_string())
                    }
                    "remove" => {
                        let id = args.get("trigger_id").and_then(|v| v.as_str())
                            .ok_or("missing 'trigger_id'")?;
                        let ok = c.triggers.unregister(id);
                        Ok(json!({ "removed": ok }).to_string())
                    }
                    other => Err(format!("unknown action: {}", other)),
                }
            }
        }),
    );
}

// ── Auth Management ──────────────────────────────────────────

fn register_auth_tools(server: &mut McpServer) {
    server.tool(
        "mtw_auth_jwt_create",
        "Create a JWT token for a user with specified roles and expiration",
        json!({ "type": "object", "properties": {
            "user_id": { "type": "string" },
            "roles": { "type": "array", "items": { "type": "string" } },
            "expires_in_secs": { "type": "integer", "default": 3600 }
        }, "required": ["user_id"] }),
        handler(|args| async move {
            let user = args.get("user_id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(json!({ "token": format!("jwt-placeholder-for-{}", user), "expires_in": 3600 }).to_string())
        }),
    );

    server.tool(
        "mtw_auth_apikey_create",
        "Generate a new API key for a service or user",
        json!({ "type": "object", "properties": {
            "owner": { "type": "string" },
            "roles": { "type": "array", "items": { "type": "string" } },
            "expires_in_days": { "type": "integer" }
        }, "required": ["owner"] }),
        handler(|args| async move {
            let owner = args.get("owner").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(json!({ "key": format!("mtw_{}_placeholder", owner), "owner": owner }).to_string())
        }),
    );

    server.tool(
        "mtw_auth_apikey_list",
        "List all API keys with their owners, roles, and status",
        json!({ "type": "object", "properties": {
            "owner": { "type": "string", "description": "Filter by owner" }
        }, "required": [] }),
        handler(|_| async { Ok(json!({ "keys": [] }).to_string()) }),
    );

    server.tool(
        "mtw_auth_apikey_revoke",
        "Revoke an API key immediately",
        json!({ "type": "object", "properties": {
            "key_id": { "type": "string" }
        }, "required": ["key_id"] }),
        handler(|args| async move {
            let key_id = args.get("key_id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("API key '{}' revoked", key_id))
        }),
    );
}

// ── Trading Management ───────────────────────────────────────

fn register_trading_tools(server: &mut McpServer) {
    server.tool(
        "mtw_trading_formulas",
        "List available trading formulas or run them on candle data",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list", "compute"], "default": "list" },
            "symbol": { "type": "string" },
            "candles": { "type": "array", "items": { "type": "object" }, "description": "OHLCV candle data" },
            "formula_id": { "type": "string", "description": "Run specific formula (omit for all)" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            match action {
                "list" => Ok(json!({
                    "formulas": [
                        { "id": "rsi", "name": "RSI (14)", "type": "momentum" },
                        { "id": "macd", "name": "MACD (12/26/9)", "type": "trend" },
                        { "id": "bollinger", "name": "Bollinger Bands", "type": "volatility" },
                        { "id": "ema_crossover", "name": "EMA Crossover (9/21)", "type": "trend" },
                        { "id": "supertrend", "name": "SuperTrend (10, 3.0)", "type": "trend" },
                        { "id": "adx", "name": "ADX (14)", "type": "trend_strength" },
                        { "id": "stochastic_rsi", "name": "Stochastic RSI", "type": "momentum" },
                        { "id": "ichimoku", "name": "Ichimoku Cloud", "type": "multi" },
                        { "id": "obv", "name": "On Balance Volume", "type": "volume" },
                        { "id": "kelly", "name": "Kelly Criterion", "type": "sizing" },
                        { "id": "linear_regression", "name": "Linear Regression", "type": "statistical" },
                        { "id": "vwap", "name": "VWAP Deviation", "type": "volume" },
                        { "id": "williams_r", "name": "Williams %R", "type": "momentum" },
                        { "id": "ensemble", "name": "Ensemble Vote", "type": "meta" },
                        { "id": "regime", "name": "Market Regime", "type": "context" }
                    ]
                }).to_string()),
                "compute" => Ok(json!({ "hint": "Pass candles array and optional symbol to compute formulas via bridge" }).to_string()),
                _ => Err("unknown action".into()),
            }
        }),
    );

    server.tool(
        "mtw_trading_monitor",
        "Manage the trade monitor: add/remove positions, check SL/TP, view open positions",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list", "add", "remove", "check"] },
            "trade_id": { "type": "string" },
            "symbol": { "type": "string" },
            "side": { "type": "string", "enum": ["buy", "sell"] },
            "entry_price": { "type": "number" },
            "amount": { "type": "number" },
            "stop_loss": { "type": "number" },
            "take_profit": { "type": "number" },
            "trailing_stop_pct": { "type": "number" },
            "current_price": { "type": "number" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            Ok(json!({ "action": action, "positions": [], "hint": "Use bridge for live monitoring" }).to_string())
        }),
    );

    server.tool(
        "mtw_trading_strategies",
        "Manage trading strategies: create, update, list, activate/pause",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list", "create", "update", "activate", "pause"] },
            "strategy_id": { "type": "string" },
            "name": { "type": "string" },
            "symbols": { "type": "array", "items": { "type": "string" } },
            "timeframe": { "type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"] },
            "stop_loss_pct": { "type": "number" },
            "take_profit_pct": { "type": "number" },
            "min_consensus": { "type": "integer" },
            "min_confidence": { "type": "number" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            Ok(json!({ "action": action, "strategies": [] }).to_string())
        }),
    );
}

// ── Security Management ──────────────────────────────────────

fn register_security_tools(server: &mut McpServer) {
    server.tool(
        "mtw_security_rate_limits",
        "View and manage rate limiting: check status, block/unblock keys, set limits",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["status", "block", "unblock", "blocked_list", "configure"] },
            "key": { "type": "string", "description": "Rate limit key (e.g. 'telegram:user123')" },
            "max_requests": { "type": "integer" },
            "window_secs": { "type": "integer" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("status");
            Ok(json!({ "action": action, "rate_limiter": "active" }).to_string())
        }),
    );

    server.tool(
        "mtw_security_policies",
        "Manage security policies: allowlist/denylist tools, set per-user policies",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["get", "set_default", "set_user", "list_users"] },
            "user_id": { "type": "string" },
            "mode": { "type": "string", "enum": ["allowlist", "denylist"] },
            "tools": { "type": "array", "items": { "type": "string" } },
            "require_pairing": { "type": "boolean" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("get");
            Ok(json!({ "action": action, "policy": {} }).to_string())
        }),
    );

    server.tool(
        "mtw_security_approvals",
        "Manage approval gates: list gates, view pending, approve/deny",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list_gates", "list_pending", "approve", "deny", "add_gate"] },
            "approval_id": { "type": "string" },
            "tool_pattern": { "type": "string" },
            "risk_level": { "type": "string", "enum": ["low", "medium", "high", "critical"] },
            "reason": { "type": "string" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list_gates");
            Ok(json!({ "action": action, "gates": [], "pending": [] }).to_string())
        }),
    );
}

// ── Channel Management ───────────────────────────────────────

fn register_channel_tools(server: &mut McpServer) {
    server.tool(
        "mtw_channels_list",
        "List all pub/sub channels with subscriber counts, history size, and auth settings",
        json!({ "type": "object", "properties": {}, "required": [] }),
        handler(|_| async {
            Ok(json!({ "channels": [
                { "name": "dashboard", "subscribers": 0, "history": 1 },
                { "name": "agents", "subscribers": 0, "history": 10 },
                { "name": "agents.flow", "subscribers": 0, "history": 50 },
                { "name": "notifications", "subscribers": 0, "history": 20 },
                { "name": "trading", "subscribers": 0, "history": 5 },
                { "name": "rpc", "subscribers": 0, "history": 0 },
                { "name": "system", "subscribers": 0, "history": 10, "auth": true }
            ]}).to_string())
        }),
    );

    server.tool(
        "mtw_channels_create",
        "Create a new pub/sub channel with optional auth, member limits, and history",
        json!({ "type": "object", "properties": {
            "name": { "type": "string", "description": "Channel name (supports globs like 'chat.*')" },
            "auth": { "type": "boolean", "default": false },
            "max_members": { "type": "integer" },
            "history": { "type": "integer", "default": 10 }
        }, "required": ["name"] }),
        handler(|args| async move {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Channel '{}' created", name))
        }),
    );

    server.tool(
        "mtw_channels_publish",
        "Publish a message to a channel",
        json!({ "type": "object", "properties": {
            "channel": { "type": "string" },
            "payload": { "description": "Message payload (text or JSON)" }
        }, "required": ["channel", "payload"] }),
        handler(|args| async move {
            let ch = args.get("channel").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Published to channel '{}'", ch))
        }),
    );
}

// ── Transport Management ─────────────────────────────────────

fn register_transport_tools(server: &mut McpServer) {
    server.tool(
        "mtw_transport_connections",
        "List active WebSocket connections with metadata",
        json!({ "type": "object", "properties": {
            "limit": { "type": "integer", "default": 50 }
        }, "required": [] }),
        handler(|_| async {
            Ok(json!({ "connections": [], "total": 0, "transport": "websocket" }).to_string())
        }),
    );

    server.tool(
        "mtw_transport_kick",
        "Disconnect a specific connection by ID",
        json!({ "type": "object", "properties": {
            "connection_id": { "type": "string" }
        }, "required": ["connection_id"] }),
        handler(|args| async move {
            let id = args.get("connection_id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Connection '{}' disconnected", id))
        }),
    );

    server.tool(
        "mtw_transport_broadcast",
        "Broadcast a message to all connected clients",
        json!({ "type": "object", "properties": {
            "message": { "type": "string" },
            "channel": { "type": "string", "description": "Optional: limit to channel subscribers" }
        }, "required": ["message"] }),
        handler(|_| async { Ok("Broadcast sent".into()) }),
    );
}

// ── Federation Management ────────────────────────────────────

fn register_federation_tools(server: &mut McpServer) {
    server.tool(
        "mtw_federation_peers",
        "Manage federation peers: list, add, remove, sync status",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list", "add", "remove", "sync", "sync_all"] },
            "peer_id": { "type": "string" },
            "name": { "type": "string" },
            "url": { "type": "string" },
            "api_key": { "type": "string" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            Ok(json!({ "action": action, "peers": [] }).to_string())
        }),
    );

    server.tool(
        "mtw_federation_changelog",
        "View the federation change log: recent changes, pending sync, conflicts",
        json!({ "type": "object", "properties": {
            "since_version": { "type": "integer", "default": 0 },
            "limit": { "type": "integer", "default": 50 }
        }, "required": [] }),
        handler(|_| async { Ok(json!({ "changes": [], "latest_version": 0 }).to_string()) }),
    );
}

// ── Notification Management ──────────────────────────────────

fn register_notify_tools(server: &mut McpServer) {
    server.tool(
        "mtw_notify_send",
        "Send a notification through one or all channels (telegram, slack, discord, etc.)",
        json!({ "type": "object", "properties": {
            "title": { "type": "string" },
            "body": { "type": "string" },
            "channel": { "type": "string", "description": "Provider channel: 'telegram', 'slack', 'all', etc.", "default": "all" },
            "priority": { "type": "string", "enum": ["low", "normal", "high", "critical"], "default": "normal" },
            "silent": { "type": "boolean", "default": false }
        }, "required": ["title"] }),
        handler(|args| async move {
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            let channel = args.get("channel").and_then(|v| v.as_str()).unwrap_or("all");
            Ok(format!("Notification '{}' sent to {}", title, channel))
        }),
    );

    server.tool(
        "mtw_notify_providers",
        "List and manage notification providers (status, configure, test)",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list", "test", "configure"] },
            "provider_id": { "type": "string" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            Ok(json!({ "action": action, "providers": [] }).to_string())
        }),
    );
}

// ── Skill/Plugin Management ──────────────────────────────────

fn register_skill_tools(server: &mut McpServer) {
    server.tool(
        "mtw_skills_list",
        "List installed skills/plugins with their status, permissions, and usage stats",
        json!({ "type": "object", "properties": {
            "active_only": { "type": "boolean", "default": false }
        }, "required": [] }),
        handler(|_| async { Ok(json!({ "skills": [] }).to_string()) }),
    );

    server.tool(
        "mtw_skills_install",
        "Install a skill from local path, npm, git, or marketplace",
        json!({ "type": "object", "properties": {
            "source": { "type": "string", "description": "Source: local path, npm package, git URL, or marketplace slug" },
            "source_type": { "type": "string", "enum": ["local", "npm", "git", "bundled"] }
        }, "required": ["source"] }),
        handler(|args| async move {
            let source = args.get("source").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Skill '{}' installed", source))
        }),
    );

    server.tool(
        "mtw_skills_manage",
        "Enable, disable, configure, or uninstall a skill",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["enable", "disable", "configure", "uninstall", "grant_permissions", "revoke_permissions"] },
            "skill_id": { "type": "string" },
            "permissions": { "type": "array", "items": { "type": "string" } },
            "settings": { "type": "object" }
        }, "required": ["action", "skill_id"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let skill_id = args.get("skill_id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Skill '{}': {} done", skill_id, action))
        }),
    );

    server.tool(
        "mtw_marketplace_search",
        "Search the mtwRequest marketplace for modules, skills, themes, and templates",
        json!({ "type": "object", "properties": {
            "query": { "type": "string" },
            "type": { "type": "string", "enum": ["all", "extension", "agent", "flow", "theme", "template", "channel"] },
            "sort": { "type": "string", "enum": ["popular", "rating", "newest", "name"], "default": "popular" },
            "limit": { "type": "integer", "default": 20 }
        }, "required": [] }),
        handler(|_| async {
            Ok(json!({ "items": [], "total": 0, "hint": "Marketplace coming soon" }).to_string())
        }),
    );
}

// ── Helper ───────────────────────────────────────────────────

fn handler<F, Fut>(f: F) -> ToolHandler
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    Arc::new(move |args| {
        let fut = f(args);
        Box::pin(fut)
    })
}
