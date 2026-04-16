//! mtw-mcp — MCP server for Claude Code
//!
//! Exposes mtwRequest management tools via Model Context Protocol (stdio).
//! Install in Claude Code: `claude mcp add mtw-mcp -- /path/to/mtw-mcp`

mod agents_ctx;
mod protocol;
mod tools;

use agents_ctx::AgentsCtx;
use protocol::McpServer;

#[tokio::main]
async fn main() {
    // MCP servers must not write to stdout outside JSON-RPC.
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_writer(std::io::stderr)
        .init();

    // Bootstrap agent stack (store + engine + registries + providers).
    // Failures here are fatal — without a working agent context, the
    // `mtw_agents_*` tools can't function and we'd rather fail loudly at
    // startup than serve broken tools to Claude.
    let ctx = match AgentsCtx::bootstrap() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mtw-mcp: failed to bootstrap agent context: {}", e);
            std::process::exit(1);
        }
    };

    let mut server = McpServer::new("mtw-request", "0.2.0");
    tools::register_all(&mut server, &ctx);
    server.run().await;
}
