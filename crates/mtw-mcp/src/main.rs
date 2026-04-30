//! mtw-mcp — MCP server for Claude Code.
//!
//! Exposes the mtwRequest agents layer via the Model Context Protocol
//! (JSON-RPC over stdio). Install with:
//!   claude mcp add mtw-mcp -- /path/to/mtw-mcp
//!
//! The server registers eight tools, all rooted at `mtw_agents_*`. See
//! `tools.rs` for the full surface.

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

    let mut server = McpServer::new("mtw-request", env!("CARGO_PKG_VERSION"));
    tools::register_all(&mut server, &ctx);
    server.run().await;
}
