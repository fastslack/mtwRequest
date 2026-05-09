//! MCP tools proxying mtwKernel's agent surface.

use crate::kernel_client::{KernelAgent, KernelClient, KernelRank};
use crate::protocol::{McpServer, ToolHandler, ToolResult};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

const COMODORO: &str = "Comodoro";

pub fn register_all(server: &mut McpServer) {
    register_list(server);
    register_run(server);
}

fn register_list(server: &mut McpServer) {
    server.tool_full(
        "mtw_kernel_agents_list",
        "List mtwKernel agents enriched with role, executor, flow, rank, and importance markers (manager/dashboard/high-rank). Use this to discover which agents can be assigned tasks.",
        json!({
            "type": "object",
            "properties": {
                "importance_only": {
                    "type": "boolean",
                    "default": false,
                    "description": "When true, return only managers, dashboard-pinned agents, or rank ≥ Comodoro."
                },
                "limit": { "type": "integer", "default": 200 }
            },
            "required": []
        }),
        Some(json!({
            "type": "object",
            "properties": {
                "agents": { "type": "array" },
                "total": { "type": "integer" },
                "all_count": { "type": "integer" },
                "importance_only": { "type": "boolean" },
                "comodoro_level": { "type": ["integer", "null"] }
            },
            "required": ["agents", "total", "all_count"]
        })),
        vec!["kernel".into(), "agents".into(), "list".into(), "discovery".into()],
        handler(|args| async move {
            let importance_only = args
                .get("importance_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize;

            let client = KernelClient::from_env();
            let (agents, ranks) = match tokio::join!(client.list_agents(), client.list_ranks()) {
                (Ok(a), Ok(r)) => (a, r),
                (Err(e), _) => return Err(e),
                (_, Err(e)) => return Err(e),
            };

            let rank_by_id: HashMap<&str, &KernelRank> =
                ranks.iter().map(|r| (r.id.as_str(), r)).collect();
            let comodoro_level = ranks
                .iter()
                .find(|r| r.name.eq_ignore_ascii_case(COMODORO))
                .map(|r| r.level)
                .unwrap_or(i64::MAX);

            let enriched: Vec<Value> = agents
                .iter()
                .map(|a| classify(a, &rank_by_id, comodoro_level))
                .collect();

            let filtered: Vec<&Value> = enriched
                .iter()
                .filter(|v| {
                    if !importance_only {
                        return true;
                    }
                    v.get("importance")
                        .and_then(|imp| imp.get("any"))
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false)
                })
                .take(limit)
                .collect();

            let out = json!({
                "agents": filtered,
                "total": filtered.len(),
                "all_count": agents.len(),
                "importance_only": importance_only,
                "comodoro_level": if comodoro_level == i64::MAX { Value::Null } else { json!(comodoro_level) },
            });
            Ok(ToolResult::structured(out.to_string(), out))
        }),
    );
}

fn register_run(server: &mut McpServer) {
    server.tool_full(
        "mtw_kernel_agents_run",
        "Dispatch a goal/prompt to a mtwKernel agent. Non-blocking — returns the run_id immediately while the kernel executes the agent in the background.",
        json!({
            "type": "object",
            "properties": {
                "agent_id":  { "type": "string", "description": "Kernel agent UUID (from mtw_kernel_agents_list)." },
                "goal":      { "type": "string", "description": "The task or prompt to assign. Falls back to the agent's goal_template when omitted." },
                "workspace": { "type": "string", "description": "Optional alphanumeric workspace name (max 64 chars) for an isolated cwd." }
            },
            "required": ["agent_id", "goal"]
        }),
        Some(json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string" },
                "run_id":   { "type": "string" },
                "status":   { "type": "string" },
                "success":  { "type": "boolean" }
            },
            "required": ["agent_id", "run_id"]
        })),
        vec!["kernel".into(), "agents".into(), "run".into()],
        handler(|args| async move {
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .ok_or("missing 'agent_id'")?
                .to_string();
            let goal = args
                .get("goal")
                .and_then(|v| v.as_str())
                .ok_or("missing 'goal'")?
                .to_string();
            let workspace = args
                .get("workspace")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let client = KernelClient::from_env();
            let resp = client
                .run_agent(&agent_id, &goal, workspace.as_deref())
                .await?;
            let out = json!({
                "agent_id": agent_id,
                "run_id": resp.run_id,
                "status": resp.status,
                "success": resp.success,
            });
            Ok(ToolResult::structured(out.to_string(), out))
        }),
    );
}

fn classify(
    agent: &KernelAgent,
    rank_by_id: &HashMap<&str, &KernelRank>,
    comodoro_level: i64,
) -> Value {
    let rank = rank_by_id.get(agent.rank_id.as_str()).copied();
    let rank_name = rank.map(|r| r.name.clone()).unwrap_or_default();
    let rank_level = rank.map(|r| r.level).unwrap_or(0);
    let rank_insignia = rank.map(|r| r.insignia.clone()).unwrap_or_default();

    let is_manager = agent.role == "manager";
    let is_dashboard = agent.show_on_dashboard == 1;
    let is_high_rank = rank.is_some() && rank_level >= comodoro_level;
    let any = is_manager || is_dashboard || is_high_rank;

    json!({
        "id": agent.id,
        "name": agent.name,
        "description": agent.description,
        "role": if agent.role.is_empty() { "worker".to_string() } else { agent.role.clone() },
        "executor_type": if agent.executor_type.is_empty() { "native".to_string() } else { agent.executor_type.clone() },
        "flow_id": agent.flow_id,
        "provider": agent.provider,
        "model": agent.model,
        "rank_name": rank_name,
        "rank_level": rank_level,
        "rank_insignia": rank_insignia,
        "active": agent.active == 1,
        "show_on_dashboard": is_dashboard,
        "importance": {
            "manager": is_manager,
            "dashboard": is_dashboard,
            "high_rank": is_high_rank,
            "any": any,
        },
    })
}

fn handler<F, Fut>(f: F) -> ToolHandler
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ToolResult, String>> + Send + 'static,
{
    Arc::new(move |args| {
        let fut = f(args);
        Box::pin(fut)
    })
}
