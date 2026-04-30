//! MCP tools for managing mtwRequest AI agents.
//!
//! This crate exposes the agent layer via the Model Context Protocol:
//! create/run/persist agents, schedule them, chain them, group them into
//! flows, and bind event triggers to them.
//!
//! All eight tools are live against a real `AgentsCtx`:
//!   * `mtw_agents_list`     — query the persisted store
//!   * `mtw_agents_create`   — persist + register with the executor
//!   * `mtw_agents_run`      — blocking execution via the engine
//!   * `mtw_agents_runs`     — list persisted run records
//!   * `mtw_agents_schedule` — interval/cron registration (in-memory)
//!   * `mtw_agents_chain`    — source→target correlation (in-memory)
//!   * `mtw_agents_flows`    — group management (in-memory)
//!   * `mtw_agents_triggers` — event-bound registrations (in-memory)
//!
//! Persistence: agents and runs hit SQLite via `AgentStore`. Schedules,
//! chains, flows, and triggers live only in the MCP process's memory —
//! they reset when the MCP stdio session restarts.

use crate::agents_ctx::AgentsCtx;
use crate::protocol::{McpServer, ToolHandler};
use serde_json::{json, Value};
use std::sync::Arc;

/// Register the eight `mtw_agents_*` tools on the MCP server.
pub fn register_all(server: &mut McpServer, ctx: &AgentsCtx) {
    register_agent_tools(server, ctx);
}

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

    // -- mtw_agents_runs --------------------------------------------------
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
