# @matware/mtw-request-ts-client

TypeScript/JavaScript client for **mtwRequest** — the real-time WebSocket framework.

## Install

```bash
npm install @matware/mtw-request-ts-client
# or
bun add @matware/mtw-request-ts-client
```

## Quick Start

```typescript
import { connect } from '@matware/mtw-request-ts-client';

const client = await connect({ url: 'ws://localhost:7741/ws' });

// Subscribe to a channel
const channel = await client.channel('dashboard');
channel.onMessage((msg) => {
  console.log('Data:', msg.payload);
});

// Publish to a channel
await channel.publish({ hello: 'world' });

// Close
await client.close();
```

## Connection

### Basic

```typescript
import { MtwConnection } from '@matware/mtw-request-ts-client';

const conn = new MtwConnection({
  url: 'ws://localhost:7741/ws',
  reconnect: true,
  reconnectDelay: 2000,
  maxReconnectDelay: 30000,
});

conn.on('connected', () => console.log('Connected'));
conn.on('disconnected', () => console.log('Disconnected'));
conn.on('reconnected', () => console.log('Reconnected'));
conn.on('error', (err) => console.error(err));

await conn.connect();
```

### With Authentication

```typescript
const client = await connect({
  url: 'ws://localhost:7741/ws',
  auth: {
    token: 'your-jwt-or-api-key',
  },
});
```

## Channels (Pub/Sub)

```typescript
const channel = await client.channel('trading');

// Listen for messages
channel.onMessage((msg) => {
  const data = msg.payload.kind === 'Json' ? msg.payload.data : null;
  console.log('Trading update:', data);
});

// Publish data
await channel.publish({ symbol: 'BTC/EUR', price: 74230 });

// Publish text
await channel.publishText('Hello traders!');

// Request/Response (RPC pattern)
const response = await channel.request({ action: 'getPrice', args: { symbol: 'BTC/EUR' } });
console.log('Price:', response);

// Unsubscribe
await channel.unsubscribe();
```

### Channel History

Channels can be configured on the server with `history = N`. When you subscribe, you automatically receive the last N messages.

## RPC via Channels

mtwRequest supports request/response patterns over channels. The kernel listens on the `rpc` channel:

```typescript
import { MtwConnection, createMessage, jsonPayload } from '@matware/mtw-request-ts-client';

const conn = new MtwConnection({ url: 'ws://localhost:7741/ws' });
await conn.connect();

// Subscribe to RPC channel
conn.send(createMessage('subscribe', emptyPayload(), { channel: 'rpc' }));

// Send RPC request
const requestId = crypto.randomUUID();
conn.send(createMessage('publish', jsonPayload({
  action: 'trading.session.detail',
  args: { id: 'session-123' },
}), { channel: 'rpc', id: requestId }));

// Listen for response (matched by ref_id)
conn.onChannel('rpc', (msg) => {
  if (msg.ref_id === requestId) {
    console.log('Response:', msg.payload.data);
  }
});
```

### Available RPC Actions

| Action | Description |
|--------|-------------|
| `trading.sessions.list` | List all trading sessions |
| `trading.session.detail` | Session detail with trades, KPIs |
| `trading.session.equity` | Equity curve for a session |
| `trading.overview` | Lightweight trading overview |
| `trading.market.latest` | Latest market snapshots |
| `trading.strategy.comparison` | Strategy performance comparison |
| `pine.list` | List PineScript strategies |
| `pine.compile` | Compile PineScript code |
| `pine.export` | Export session as PineScript |
| `llm.chat` | Call LLM (routed through Rust) |
| `tasks.list` / `tasks.create` / ... | Task CRUD |
| `contacts.list` / `contacts.create` / ... | CRM CRUD |
| `notes.list` / `notes.create` / ... | Notes CRUD |
| `finance.accounts.list` / ... | Finance |
| `health.metrics.log` / ... | Health tracking |
| `nutrition.entries.log` / ... | Nutrition |
| `training.workouts.start` / ... | Training |
| ...and 140+ more actions | |

## AI Agents

```typescript
const agent = client.agent('assistant');

// Simple request/response
const response = await agent.send('What is the weather?');
console.log(response.text);

// Streaming
for await (const chunk of agent.stream('Tell me about Bitcoin')) {
  process.stdout.write(chunk.text);
}

// Register tool handlers
agent.registerTool('search', async (params) => {
  const results = await searchDatabase(params.query);
  return JSON.stringify(results);
});

// Agent with tools
const response = await agent.send('Search for recent BTC news');
// Agent automatically calls search tool and returns combined response
```

## LLM via mtwRequest

All LLM calls are routed through mtwRequest's Rust AI providers:

```typescript
// Via RPC
const result = await channel.request({
  action: 'llm.chat',
  args: {
    system: 'You are a helpful assistant',
    user: 'What is 2+2?',
    model: 'gpt-4o-mini',      // optional, uses server default
    max_tokens: 1024,           // optional
    temperature: 0.7,           // optional
  },
});
console.log(result.text);       // "4"
console.log(result.provider);   // "openai"
console.log(result.model);      // "gpt-4o-mini"
```

### Supported Providers

Configure via environment variables on the mtwRequest server:

| Provider | Env Vars | Models |
|----------|----------|--------|
| **OpenAI** | `LLM_PROVIDER=openai` `OPENAI_API_KEY=sk-...` | gpt-4o, gpt-4o-mini, o1 |
| **Anthropic** | `LLM_PROVIDER=anthropic` `ANTHROPIC_API_KEY=sk-ant-...` | claude-sonnet-4, claude-haiku-4.5 |
| **Ollama** | `LLM_PROVIDER=ollama` `OLLAMA_URL=http://localhost:11434` | llama3, mistral, codellama |
| **LM Studio** | `LLM_PROVIDER=lmstudio` `LMSTUDIO_URL=http://localhost:1234/v1` | any loaded model |

## Low-Level API

### Message Format

```typescript
interface MtwMessage {
  id: string;              // Unique message ID
  type: MsgType;           // 'subscribe' | 'publish' | 'request' | 'response' | ...
  channel?: string;        // Target channel
  ref_id?: string;         // Correlation ID for request/response
  payload: Payload;        // { kind: 'Json' | 'Text' | 'Binary' | 'None', data: any }
  metadata: Record<string, any>;
  timestamp: number;
}
```

### Creating Messages

```typescript
import { createMessage, jsonPayload, textPayload, emptyPayload } from '@matware/mtw-request-ts-client';

// JSON payload
const msg = createMessage('publish', jsonPayload({ key: 'value' }), { channel: 'my-channel' });

// Text payload
const msg = createMessage('publish', textPayload('Hello'), { channel: 'chat' });

// Empty (subscribe/unsubscribe)
const msg = createMessage('subscribe', emptyPayload(), { channel: 'dashboard' });
```

### Direct Connection Events

```typescript
const conn = new MtwConnection({ url: 'ws://localhost:7741/ws' });

conn.on('connected', () => { /* ... */ });
conn.on('disconnected', () => { /* ... */ });
conn.on('reconnected', () => { /* ... */ });
conn.on('error', (err) => { /* ... */ });

// Listen to specific channel
conn.onChannel('trading', (msg) => {
  console.log('Trading data:', msg.payload.data);
});

// Send raw message
conn.send(createMessage('publish', jsonPayload(data), { channel: 'my-channel' }));
```

## Framework Bindings

| Package | Framework |
|---------|-----------|
| `@matware/mtw-request-ts-client` | Vanilla JS/TS, Node.js |
| `@matware/mtw-request-svelte` | Svelte stores integration |
| `@matware/mtw-request-react` | React hooks |
| `@matware/mtw-request-vue` | Vue composables |
| `@matware/mtw-request-three` | Three.js real-time 3D |

## Architecture

```
Browser/App
    ↓ WebSocket
mtwRequest (Rust server, port 7741)
    ├── Channels (pub/sub with history)
    ├── AI Providers (OpenAI, Anthropic, Ollama, LMStudio)
    ├── Bridge Server (Unix socket → mtwKernel tools)
    ├── Store (read-only SQLite access)
    └── Rate Limiting, Auth, Federation
    ↓ Unix Socket
mtwKernel (Node.js, 50+ modules, 600+ tools)
    ├── Trading (57 formulas, auto-trader, sessions)
    ├── CRM, Tasks, Calendar, Finance, Health, ...
    └── PineScript engine, Graph Intelligence, ...
```
