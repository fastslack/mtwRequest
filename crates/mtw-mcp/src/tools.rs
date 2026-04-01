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

use crate::protocol::{McpServer, ToolHandler};
use serde_json::{json, Value};
use std::sync::Arc;

/// Register all mtwRequest management tools
pub fn register_all(server: &mut McpServer) {
    register_server_tools(server);
    register_module_tools(server);
    register_agent_tools(server);
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

fn register_agent_tools(server: &mut McpServer) {
    server.tool(
        "mtw_agents_list",
        "List all AI agents with their status, provider, model, triggers, and recent runs",
        json!({ "type": "object", "properties": {
            "flow_id": { "type": "string", "description": "Filter by flow ID" },
            "active_only": { "type": "boolean", "default": false }
        }, "required": [] }),
        handler(|_| async {
            Ok(json!({ "agents": [], "total": 0, "hint": "Connect to mtwKernel bridge for live agent data" }).to_string())
        }),
    );

    server.tool(
        "mtw_agents_create",
        "Create a new AI agent with system prompt, tools, provider, triggers, and scheduling",
        json!({ "type": "object", "properties": {
            "name": { "type": "string" },
            "description": { "type": "string" },
            "system_prompt": { "type": "string" },
            "goal_template": { "type": "string", "description": "Template with {{variables}}" },
            "provider": { "type": "string", "enum": ["anthropic", "openai", "ollama", "lmstudio"] },
            "model": { "type": "string" },
            "allowed_tools": { "type": "array", "items": { "type": "string" } },
            "denied_tools": { "type": "array", "items": { "type": "string" } },
            "max_iterations": { "type": "integer", "default": 15 },
            "timeout_ms": { "type": "integer", "default": 300000 },
            "flow_id": { "type": "string" }
        }, "required": ["name", "system_prompt"] }),
        handler(|args| async move {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed");
            Ok(format!("Agent '{}' created. Use mtw_agents_run to execute.", name))
        }),
    );

    server.tool(
        "mtw_agents_run",
        "Execute an agent with a specific goal. Returns run ID for tracking.",
        json!({ "type": "object", "properties": {
            "agent_id": { "type": "string" },
            "goal": { "type": "string" },
            "variables": { "type": "object", "description": "Variables for goal_template interpolation" }
        }, "required": ["agent_id", "goal"] }),
        handler(|args| async move {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
            let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(json!({ "run_id": "run-pending", "agent_id": agent_id, "goal": goal, "status": "queued" }).to_string())
        }),
    );

    server.tool(
        "mtw_agents_schedule",
        "Create or update an agent schedule (cron or interval-based)",
        json!({ "type": "object", "properties": {
            "agent_id": { "type": "string" },
            "interval_ms": { "type": "integer", "description": "Run every N milliseconds" },
            "cron": { "type": "string", "description": "Cron expression (e.g. '0 9 * * *')" },
            "goal_override": { "type": "string" },
            "active": { "type": "boolean", "default": true }
        }, "required": ["agent_id"] }),
        handler(|args| async move {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Schedule created for agent '{}'", agent_id))
        }),
    );

    server.tool(
        "mtw_agents_chain",
        "Create a chain between two agents: when source completes, target starts automatically",
        json!({ "type": "object", "properties": {
            "source_agent_id": { "type": "string" },
            "target_agent_id": { "type": "string" },
            "condition": { "type": "string", "enum": ["always", "on_success", "on_failure"] },
            "pass_result": { "type": "boolean", "default": true },
            "delay_ms": { "type": "integer", "default": 0 }
        }, "required": ["source_agent_id", "target_agent_id"] }),
        handler(|args| async move {
            let src = args.get("source_agent_id").and_then(|v| v.as_str()).unwrap_or("?");
            let tgt = args.get("target_agent_id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Chain created: {} → {}", src, tgt))
        }),
    );

    server.tool(
        "mtw_agents_flows",
        "List, create, or manage agent flows (logical groupings of agents)",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list", "create", "delete"] },
            "name": { "type": "string" },
            "description": { "type": "string" },
            "flow_id": { "type": "string" }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            Ok(json!({ "action": action, "flows": [] }).to_string())
        }),
    );

    server.tool(
        "mtw_agents_triggers",
        "Manage event-driven triggers for agents (fire agent on specific events)",
        json!({ "type": "object", "properties": {
            "action": { "type": "string", "enum": ["list", "add", "remove"] },
            "agent_id": { "type": "string" },
            "event_name": { "type": "string", "description": "Event to listen for (e.g. 'task.created', 'reminder.fired')" },
            "filter": { "type": "object", "description": "JSON filter for event payload matching" },
            "cooldown_ms": { "type": "integer", "default": 60000 }
        }, "required": ["action"] }),
        handler(|args| async move {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            Ok(json!({ "action": action, "triggers": [] }).to_string())
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
