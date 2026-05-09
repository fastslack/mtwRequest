//! Programmatic tool calling (`mtw_code_run`).
//!
//! Instead of invoking tools one-by-one through inference, the model can
//! write a small Rhai script that composes them, runs it server-side, and
//! gets a single result back. This cuts inference rounds and latency for
//! workflows like "list agents → filter by provider → run the first one".
//!
//! ## Why Rhai
//! Embeddable, sandboxed by default (no FS / network unless we add it),
//! JS-like syntax the model already speaks fluently, and a tiny binary
//! footprint compared to a full V8 isolate.
//!
//! ## What's exposed inside the script
//! * `tool(name, args)` — call any registered MCP tool by name. Returns
//!   the parsed JSON result (or a string with the error message).
//! * `log(msg)` — append to the script's log buffer (returned alongside
//!   the result).
//! * `input` scope variable — the `vars` object passed to `mtw_code_run`.
//! * Standard Rhai stdlib (arrays, maps, strings, math).
//!
//! ## Activation order
//! ```ignore
//! code_mode::register(&mut server);     // declares the tool
//! let server = Arc::new(server);
//! code_mode::activate(server.clone());  // wires `tool(...)` to the live server
//! ```
//!
//! Hard limits: 5s wall clock, 1_000_000 operations, 64 KB script size.

use crate::protocol::{McpServer, ToolHandler, ToolResult};
use serde_json::json;
#[cfg(feature = "code-mode")]
use serde_json::Value;
use std::sync::{Arc, OnceLock};

/// Late-bound handle to the live `McpServer`. Filled in by [`activate`]
/// once the server is wrapped in an `Arc`. Until then, `tool(...)` calls
/// from a script return an explanatory error.
static DISPATCHER: OnceLock<Arc<McpServer>> = OnceLock::new();

#[cfg(feature = "code-mode")]
const MAX_SCRIPT_LEN: usize = 64 * 1024;
#[cfg(feature = "code-mode")]
const MAX_OPERATIONS: u64 = 1_000_000;
#[cfg(feature = "code-mode")]
const MAX_RUNTIME_MS: u64 = 5_000;

pub fn register(server: &mut McpServer) {
    server.tool_full(
        "mtw_code_run",
        "Run a small Rhai script that can call other MCP tools via `tool(name, args)`. \
         Use this to compose multiple tool calls in one inference round (e.g. list-then-filter, \
         create-then-run). Sandbox: no FS, no network, 5s wall-clock, 1M operations. \
         Available helpers: `tool(name, args)`, `log(msg)`, `input` (vars). \
         Example: `let r = tool(\"mtw_agents_list\", #{}); r.agents[0].name`",
        json!({
            "type": "object",
            "properties": {
                "script": { "type": "string", "description": "Rhai source code." },
                "vars":   { "type": "object", "description": "Variables exposed as the `input` map." }
            },
            "required": ["script"]
        }),
        Some(json!({
            "type": "object",
            "properties": {
                "result": {},
                "logs": { "type": "array", "items": { "type": "string" } }
            }
        })),
        vec!["meta".into(), "code".into(), "compose".into()],
        run_handler(),
    );
}

/// Wire `mtw_code_run`'s `tool(...)` host function to the live MCP server.
/// Call this exactly once, after the `McpServer` has been wrapped in an
/// `Arc` and just before `serve_stdio` / `serve_http`.
pub fn activate(server: Arc<McpServer>) {
    let _ = DISPATCHER.set(server);
}

#[cfg(feature = "code-mode")]
fn run_handler() -> ToolHandler {
    use rhai::{Dynamic, Engine, Scope};
    use std::time::{Duration, Instant};
    use tokio::sync::Mutex;

    Arc::new(move |args: Value| {
        Box::pin(async move {
            let script = args
                .get("script")
                .and_then(|v| v.as_str())
                .ok_or("missing 'script'")?
                .to_string();
            if script.len() > MAX_SCRIPT_LEN {
                return Err(format!(
                    "script too large: {} bytes (max {})",
                    script.len(),
                    MAX_SCRIPT_LEN
                ));
            }
            let vars = args.get("vars").cloned().unwrap_or(json!({}));

            let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let logs_for_script = logs.clone();
            let started = Instant::now();
            let handle = tokio::runtime::Handle::current();

            let result_blob = tokio::task::spawn_blocking(move || -> Result<Value, String> {
                let mut engine = Engine::new();
                engine.set_max_operations(MAX_OPERATIONS);
                engine.set_max_expr_depths(64, 64);
                engine.on_progress(move |_ops| {
                    if started.elapsed() > Duration::from_millis(MAX_RUNTIME_MS) {
                        Some(Dynamic::from("timeout"))
                    } else {
                        None
                    }
                });

                // log(msg)
                let logs_ref = logs_for_script.clone();
                let log_handle = handle.clone();
                engine.register_fn("log", move |msg: String| {
                    let logs = logs_ref.clone();
                    log_handle.block_on(async move {
                        logs.lock().await.push(msg);
                    });
                });

                // tool(name, args)
                let dispatch_handle = handle.clone();
                engine.register_fn("tool", move |name: String, args: rhai::Map| -> Dynamic {
                    let Some(server) = DISPATCHER.get() else {
                        return Dynamic::from("dispatcher not attached".to_string());
                    };
                    let server = server.clone();
                    let json_args: Value = rhai::serde::from_dynamic(&Dynamic::from_map(args))
                        .unwrap_or(Value::Null);
                    let outcome = dispatch_handle.block_on(async move {
                        server.call_tool(&name, json_args).await
                    });
                    match outcome {
                        Ok(r) => {
                            // Prefer structured payload when present.
                            let v = r.structured.unwrap_or(Value::String(r.text));
                            rhai::serde::to_dynamic(&v).unwrap_or(Dynamic::UNIT)
                        }
                        Err(e) => Dynamic::from(format!("error: {}", e)),
                    }
                });

                let mut scope = Scope::new();
                scope.push_dynamic(
                    "input",
                    rhai::serde::to_dynamic(&vars).unwrap_or(Dynamic::UNIT),
                );

                let value: Dynamic = engine
                    .eval_with_scope::<Dynamic>(&mut scope, &script)
                    .map_err(|e| e.to_string())?;
                let json: Value = rhai::serde::from_dynamic(&value).unwrap_or(Value::Null);
                Ok(json)
            })
            .await
            .map_err(|e| format!("script join: {}", e))??;

            let collected_logs = logs.lock().await.clone();
            let out = json!({
                "result": result_blob,
                "logs": collected_logs,
            });
            Ok(ToolResult::structured(out.to_string(), out))
        })
    })
}

#[cfg(not(feature = "code-mode"))]
fn run_handler() -> ToolHandler {
    Arc::new(|_args| {
        Box::pin(async move {
            Err::<ToolResult, String>(
                "code-mode disabled at build time (rebuild with --features code-mode)".into(),
            )
        })
    })
}
