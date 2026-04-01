//! mtw-mcp — MCP server for Claude Code
//!
//! Exposes mtwRequest management tools via Model Context Protocol (stdio).
//! Install in Claude Code: `claude mcp add mtw-mcp -- /path/to/mtw-mcp`

mod protocol;
mod tools;

use protocol::McpServer;

#[tokio::main]
async fn main() {
    // MCP servers must not write to stderr unless for logging
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_writer(std::io::stderr)
        .init();

    let mut server = McpServer::new("mtw-request", "0.2.0");
    tools::register_all(&mut server);
    server.run().await;
}
