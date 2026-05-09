// Several public items in protocol/elicitation are surface for external
// embedders or future transport features and aren't called from main yet.
#![allow(dead_code)]

//! mtw-mcp — MCP server for Claude Code, ChatGPT, Cursor, and any other
//! Model Context Protocol client.
//!
//! ## Modes
//!
//! ```text
//! mtw-mcp                       # stdio (default — same as v0.3.x)
//! mtw-mcp serve --http          # streamable HTTP on 127.0.0.1:7700
//! mtw-mcp serve --http --port N --bind 0.0.0.0
//! ```
//!
//! ## What's exposed
//!
//! * `tools/list` — `mtw_agents_*` (8) + `mtw_kernel_agents_*` (2) +
//!   `mtw_tool_search` / `mtw_tool_describe` (progressive discovery) +
//!   `mtw_code_run` (programmatic tool calling, requires
//!   `--features code-mode` at build time).
//! * `tasks/*` — async A2A task primitive.
//! * `resources/*` and `prompts/*` — populated from `mtw-skills` so any
//!   MCP client gets skills-over-MCP for free.
//!
//! ## Back-compat
//!
//! Clients pinned at protocol version `2024-11-05` see exactly the v0.3.x
//! surface: same 10 `mtw_*` tools with the same names, descriptions, and
//! input schemas. Newer features (outputSchema, tasks, resources, prompts,
//! ui content, applications, elicitation) are gated behind capability
//! negotiation and protocol-version checks.

mod agents_ctx;
mod code_mode;
mod elicitation;
mod kernel_client;
mod kernel_tools;
mod protocol;
mod resources;
mod tasks;
mod tool_search;
mod tools;
mod transport;

use agents_ctx::AgentsCtx;
use protocol::McpServer;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug)]
struct CliArgs {
    mode: Mode,
    bind: String,
    port: u16,
}

#[derive(Debug, PartialEq)]
enum Mode {
    Stdio,
    Http,
}

impl Default for CliArgs {
    fn default() -> Self {
        Self {
            mode: Mode::Stdio,
            bind: "127.0.0.1".to_string(),
            port: 7700,
        }
    }
}

fn parse_args() -> CliArgs {
    let mut out = CliArgs::default();
    let mut args = std::env::args().skip(1).peekable();
    // The first positional may be `serve` (or anything else — we ignore
    // unknown verbs and rely on flags). This keeps `mtw-mcp` (no-arg)
    // exactly identical to v0.3.x.
    if let Some(first) = args.peek() {
        if first == "serve" {
            args.next();
        }
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--http" => out.mode = Mode::Http,
            "--stdio" => out.mode = Mode::Stdio,
            "--port" => {
                if let Some(v) = args.next() {
                    out.port = v.parse().unwrap_or(out.port);
                }
            }
            "--bind" => {
                if let Some(v) = args.next() {
                    out.bind = v;
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "mtw-mcp v{}\n\
                     Usage:\n  \
                       mtw-mcp                          # stdio (default)\n  \
                       mtw-mcp serve --http             # HTTP on 127.0.0.1:7700\n  \
                       mtw-mcp serve --http --port N --bind 0.0.0.0\n",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }
    out
}

#[tokio::main]
async fn main() {
    // MCP servers must not write to stdout outside JSON-RPC.
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("MTW_MCP_LOG").unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

    let args = parse_args();

    // Bootstrap agent stack (store + engine + registries + providers).
    let ctx = match AgentsCtx::bootstrap() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mtw-mcp: failed to bootstrap agent context: {}", e);
            std::process::exit(1);
        }
    };

    let mut server = McpServer::new("mtw-request", env!("CARGO_PKG_VERSION"));

    // Core tool surface (back-compat with v0.3.x).
    tools::register_all(&mut server, &ctx);
    kernel_tools::register_all(&mut server);

    // Progressive discovery — registered AFTER core tools so it can
    // snapshot the full catalog.
    tool_search::register(&mut server);

    // Programmatic tool calling. Without `--features code-mode` this
    // registers a stub that returns a clear error message.
    code_mode::register(&mut server);

    // Tasks primitive (A2A).
    let task_provider = Arc::new(tasks::TaskRegistry::new(ctx.clone()));
    server = server.with_tasks(task_provider);

    // Skills over MCP — only wired when the workspace has a SkillRegistry
    // available. Today we instantiate a fresh empty registry so the
    // capability is advertised; users / other crates can populate it
    // before the server starts when they embed mtw-mcp as a library.
    let skill_registry = Arc::new(mtw_skills::registry::SkillRegistry::new(
        mtw_skills::registry::SkillRegistryConfig::default(),
    ));
    server = server
        .with_resources(Arc::new(resources::SkillResources::new(skill_registry.clone())))
        .with_prompts(Arc::new(resources::SkillPrompts::new(skill_registry.clone())));

    // Wrap and activate code-mode dispatcher.
    let server = Arc::new(server);
    code_mode::activate(server.clone());

    match args.mode {
        Mode::Stdio => {
            transport::serve_stdio(server).await;
        }
        Mode::Http => {
            let addr: SocketAddr = format!("{}:{}", args.bind, args.port)
                .parse()
                .expect("invalid --bind/--port");
            if let Err(e) = transport::serve_http(server, addr).await {
                eprintln!("mtw-mcp: HTTP transport error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
