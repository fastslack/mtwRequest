//! Progressive discovery: `mtw_tool_search`.
//!
//! `tools/list` returns *every* registered tool, which scales badly once
//! the catalog grows past a few dozen. This tool lets the model search the
//! catalog by free-text query (matched against name, description, tags)
//! and only retrieve the schemas it actually needs.
//!
//! Pattern (Anthropic-style "tool search"):
//! 1. Client lists the few entry-point tools (or just `mtw_tool_search`).
//! 2. When the model decides it needs to look something up, it calls
//!    `mtw_tool_search({ query: "schedule agent" })`.
//! 3. The tool returns a ranked list of names + descriptions; the model
//!    then either calls `mtw_tool_describe` for the full schema or invokes
//!    the tool directly.

use crate::protocol::{McpServer, McpTool, ToolHandler, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

pub fn register(server: &mut McpServer) {
    // Snapshot the tool catalog at registration time. We don't expect
    // tools to be hot-added during a session — if we ever do, switch this
    // to a shared Arc<RwLock<Vec<McpTool>>>.
    let catalog: Arc<Vec<McpTool>> = Arc::new(server.tools().to_vec());

    let cat = catalog.clone();
    server.tool_full(
        "mtw_tool_search",
        "Search the tool catalog by free-text query. Returns ranked matches \
         (name + description + tags) without their full input schemas. Use \
         this instead of dumping every tool into the context window. Pair \
         with `mtw_tool_describe` to fetch one tool's full schema on demand.",
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Free-text query." },
                "limit": { "type": "integer", "default": 10 }
            },
            "required": ["query"]
        }),
        Some(json!({
            "type": "object",
            "properties": {
                "matches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "description": { "type": "string" },
                            "tags": { "type": "array", "items": { "type": "string" } },
                            "score": { "type": "number" }
                        }
                    }
                },
                "total": { "type": "integer" }
            },
            "required": ["matches", "total"]
        })),
        vec!["meta".into(), "discovery".into(), "search".into()],
        handler(move |args| {
            let cat = cat.clone();
            async move {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or("missing 'query'")?
                    .to_lowercase();
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;

                let mut scored: Vec<(f64, &McpTool)> = cat
                    .iter()
                    .map(|t| (score(&query, t), t))
                    .filter(|(s, _)| *s > 0.0)
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                scored.truncate(limit);

                let matches: Vec<Value> = scored
                    .iter()
                    .map(|(s, t)| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "tags": t.tags,
                            "score": s,
                        })
                    })
                    .collect();
                let out = json!({ "matches": matches, "total": scored.len() });
                Ok(ToolResult::structured(out.to_string(), out))
            }
        }),
    );

    let cat = catalog.clone();
    server.tool_full(
        "mtw_tool_describe",
        "Fetch the full input/output schema for a tool by name. Use after \
         `mtw_tool_search` to load only the schemas you'll actually call.",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        }),
        Some(json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "description": { "type": "string" },
                "inputSchema": {},
                "outputSchema": {},
                "tags": { "type": "array", "items": { "type": "string" } }
            }
        })),
        vec!["meta".into(), "discovery".into()],
        handler(move |args| {
            let cat = cat.clone();
            async move {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or("missing 'name'")?;
                let tool = cat
                    .iter()
                    .find(|t| t.name == name)
                    .ok_or_else(|| format!("tool not found: {}", name))?;
                let out = json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                    "outputSchema": tool.output_schema,
                    "tags": tool.tags,
                });
                Ok(ToolResult::structured(out.to_string(), out))
            }
        }),
    );
}

/// Crude relevance score: substring matches on name (×3), description (×1),
/// tags (×2). No stemming, no fancy ranking — the catalog is small enough
/// that this works well for now and stays interpretable.
fn score(query: &str, tool: &McpTool) -> f64 {
    let mut score = 0.0;
    let name_lc = tool.name.to_lowercase();
    let desc_lc = tool.description.to_lowercase();

    if name_lc.contains(query) {
        score += 3.0;
    }
    if desc_lc.contains(query) {
        score += 1.0;
    }
    for tag in &tool.tags {
        if tag.to_lowercase().contains(query) {
            score += 2.0;
        }
    }

    // Token-level fallback: split the query on whitespace and award partial
    // hits. Helps queries like "list agents" match `mtw_agents_list`.
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.len() > 1 {
        for tok in tokens {
            if name_lc.contains(tok) {
                score += 0.5;
            }
            if desc_lc.contains(tok) {
                score += 0.25;
            }
        }
    }

    score
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
