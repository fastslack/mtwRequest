# @matware/mtw-request-mcp

MCP server for [Claude Code](https://claude.ai/code) — manage [mtwRequest](https://github.com/fastslack/mtwRequest) AI agents: create, run, schedule, chain, group, and trigger.

**Rust-powered. Zero MCP-library dependencies. SQLite persistence for agents and runs.**

## Install in Claude Code

```bash
claude mcp add mtw-request -- npx -y @matware/mtw-request-mcp
```

That's it. The `postinstall` picks the platform binary; supported: linux-x64, linux-arm64, darwin-x64, darwin-arm64, win32-x64.

## Providers

Credentials come from environment variables. Set whichever you have — the server registers them conditionally at startup:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...
export OLLAMA_URL=http://127.0.0.1:11434   # optional, defaults to this
```

Missing keys are logged at startup (to stderr, never stdout) and their provider is simply not registered — tools still work with whatever providers are available.

## Tools (8)

All rooted at `mtw_agents_*`. Every tool is backed by a real `AgentsCtx` — no placeholders.

| Tool | What it does | Persistence |
|---|---|---|
| `mtw_agents_list` | List agents from the store | SQLite |
| `mtw_agents_create` | Persist agent config + register with executor | SQLite |
| `mtw_agents_run` | Execute an agent against a goal, blocking until completion | SQLite (run record) |
| `mtw_agents_runs` | Query persisted runs, filter by agent/status | SQLite |
| `mtw_agents_schedule` | Interval or cron schedule for an agent | in-memory (this process) |
| `mtw_agents_chain` | When source agent finishes, invoke target agent | in-memory |
| `mtw_agents_flows` | Group agents into named flows (create/list/delete) | in-memory |
| `mtw_agents_triggers` | Event-driven agent invocation with cooldown + filter | in-memory |

> **Note on persistence.** Agents and their run history survive across MCP sessions (SQLite file). Schedules, chains, flows, and triggers live only in the running MCP process — they reset when Claude Code restarts the server. This is by design: these are orchestration primitives that re-register cheaply from your workflow scripts.

## Example usage in Claude Code

> "Create a research agent that uses Anthropic claude-sonnet-4-6 with web_search tool."

> "Run the research agent with the goal 'summarize last week's LLM papers from arXiv'."

> "Show me the last 5 failed runs."

> "Schedule the backup agent to run every 6 hours."

> "Chain the 'plan' agent to the 'execute' agent on success."

## Build from source

```bash
git clone https://github.com/fastslack/mtwRequest
cd mtwRequest
cargo build --release -p mtw-mcp
claude mcp add mtw-request -- ./target/release/mtw-mcp
```

## Roadmap

The surface is deliberately narrow in 0.3.0. Candidates for future minor bumps (all gated on real backends landing, not on stubs):

- `mtw_channels_*` — live pub/sub stats from a running mtwRequest server via bridge socket
- `mtw_transport_*` — real connection introspection + kick
- `mtw_security_*` — rate-limit configuration against the running auth layer
- `mtw_skills_*` — skill/plugin lifecycle when the marketplace layer lands

Until those wire up against real state, they stay out of this package.

## License

Apache-2.0
