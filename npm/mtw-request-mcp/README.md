# @matware/mtw-request-mcp

MCP server for [Claude Code](https://claude.ai/code) — manage [mtwRequest](https://github.com/fastslack/mtwRequest) modules, agents, auth, trading, and more.

**Rust-powered. 37 tools. Zero dependencies.**

## Install in Claude Code

```bash
claude mcp add mtw-request -- npx -y @matware/mtw-request-mcp
```

That's it. One command.

## What it does

Once installed, Claude Code can manage your entire mtwRequest infrastructure:

- **Server**: status, config, health checks
- **Modules**: install, enable/disable, health diagnostics
- **Agents**: create, run, schedule, chain, flows, event triggers
- **Auth**: JWT tokens, API keys, revocation
- **Trading**: 15 formulas, SL/TP monitor, strategies
- **Security**: rate limits, policies, approval gates
- **Channels**: pub/sub management, publish messages
- **Transport**: WebSocket connections, broadcast
- **Federation**: P2P peers, sync, changelog
- **Notifications**: multi-channel send, providers
- **Skills**: marketplace, permissions, install/manage

## Example usage in Claude Code

> "List all mtwRequest modules and their health status"

> "Create an agent that monitors BTC/USDT with RSI and MACD formulas"

> "Show me the trading formulas available"

> "Set up a rate limit of 100 req/min for the API"

## Build from source

```bash
git clone https://github.com/fastslack/mtwRequest
cd mtwRequest
cargo build --release -p mtw-mcp
claude mcp add mtw-request -- ./target/release/mtw-mcp
```

## License

Apache-2.0
