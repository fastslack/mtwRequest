# Frontend Guide

mtwRequest provides client SDKs for React, Svelte, Vue, and Three.js, all built on top of the universal `@mtw/client` package. This guide covers connection management, channel subscriptions, and AI agent interactions from the frontend.

---

## Client SDK Overview (@mtw/client)

The core client package (`packages/client/`) provides:

- `MtwConnection` -- WebSocket connection with auto-reconnect, ping/pong, binary framing
- `MtwChannel` -- Channel subscription and message handling
- `MtwAgentClient` -- AI agent interaction with streaming
- Type definitions mirroring the Rust protocol exactly

Source: `packages/client/src/connection.ts`, `packages/client/src/types.ts`

### Installation

```bash
npm install @mtw/client
```

### Basic Usage

```typescript
import { MtwConnection, createMessage, textPayload, emptyPayload } from '@mtw/client';

const conn = new MtwConnection({
  url: 'ws://localhost:8080/ws',
  auth: { token: 'your-jwt-token' },
  reconnect: true,
  maxReconnectAttempts: Infinity,
  reconnectDelay: 1000,
  maxReconnectDelay: 30000,
  pingInterval: 30000,
  pongTimeout: 10000,
  connectTimeout: 10000,
});

// Event handlers
conn.on('connected', (meta) => console.log('Connected:', meta.conn_id));
conn.on('disconnected', (info) => console.log('Disconnected:', info.reason));
conn.on('reconnecting', (attempt) => console.log('Reconnecting:', attempt));
conn.on('error', (err) => console.error('Error:', err.code, err.message));
conn.on('message', (msg) => console.log('Message:', msg.type, msg.channel));

// Connect
const metadata = await conn.connect();

// Send messages
conn.send(createMessage('subscribe', emptyPayload(), { channel: 'chat.general' }));
conn.send(createMessage('publish', textPayload('Hello!'), { channel: 'chat.general' }));

// Request/response with correlation
const response = await conn.request(
  createMessage('request', { kind: 'Json', data: { action: 'get_users' } }),
  30000  // timeout ms
);

// Channel-specific handler
const unsub = conn.onChannel('chat.general', (msg) => {
  console.log('Channel message:', msg.payload);
});
unsub(); // Unsubscribe

// Send binary
conn.sendBinary(new Uint8Array([0x01, 0x02, 0x03]));

// Disconnect
await conn.close();
```

### ConnectOptions

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `url` | `string` | (required) | WebSocket server URL |
| `auth` | `AuthOptions` | `undefined` | Authentication (token/apiKey) |
| `reconnect` | `boolean` | `true` | Enable auto-reconnect |
| `maxReconnectAttempts` | `number` | `Infinity` | Max reconnect attempts |
| `reconnectDelay` | `number` | `1000` | Base reconnect delay (ms) |
| `maxReconnectDelay` | `number` | `30000` | Max delay (exponential backoff cap) |
| `pingInterval` | `number` | `30000` | Ping frequency (ms) |
| `pongTimeout` | `number` | `10000` | Pong timeout (ms) |
| `connectTimeout` | `number` | `10000` | Connection timeout (ms) |
| `protocols` | `string[]` | `undefined` | WebSocket sub-protocols |

### Connection States

```
disconnected -> connecting -> connected -> disconnecting -> disconnected
                                |
                                +-> reconnecting -> connecting -> connected
```

---

## React Integration (@mtw/react)

Source: `packages/react/src/`

### Installation

```bash
npm install @mtw/react @mtw/client
```

### MtwProvider Setup

Wrap your app with `MtwProvider` to make the connection available to all hooks:

```tsx
import { MtwProvider } from '@mtw/react';

function App() {
  return (
    <MtwProvider
      url="ws://localhost:8080/ws"
      auth={{ token: 'your-jwt-token' }}
      autoConnect={true}
      onConnected={(meta) => console.log('Connected:', meta.conn_id)}
      onDisconnected={(info) => console.log('Disconnected:', info.reason)}
      onError={(err) => console.error('Error:', err)}
    >
      <YourApp />
    </MtwProvider>
  );
}
```

You can also pass full connection options:

```tsx
<MtwProvider options={{
  url: 'ws://localhost:8080/ws',
  auth: { token: 'your-jwt' },
  reconnect: true,
  maxReconnectAttempts: 10,
  pingInterval: 30000,
}}>
```

### MtwProvider Props

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `url` | `string` | `ws://localhost:8080/ws` | Server URL (shorthand) |
| `auth` | `AuthOptions` | - | Auth options (shorthand) |
| `options` | `ConnectOptions` | - | Full connection options |
| `autoConnect` | `boolean` | `true` | Connect on mount |
| `onConnected` | `(meta) => void` | - | Connected callback |
| `onDisconnected` | `(info) => void` | - | Disconnected callback |
| `onError` | `(error) => void` | - | Error callback |

Source: `packages/react/src/MtwProvider.tsx`

### useChannel() Hook

Subscribe to a channel and receive messages reactively:

```tsx
import { useChannel } from '@mtw/react';

function ChatRoom() {
  const {
    subscribed,      // boolean -- is the subscription active?
    messages,        // MtwMessage[] -- received messages
    members,         // ChannelMember[] -- current members (presence)
    lastMessage,     // MtwMessage | null -- most recent message
    error,           // MtwError | null
    publish,         // (content: string | object) => void
    subscribe,       // () => Promise<void>
    unsubscribe,     // () => Promise<void>
    clearMessages,   // () => void
    channel,         // MtwChannel | null -- underlying instance
  } = useChannel("chat.general", {
    autoSubscribe: true,   // Subscribe on mount (default: true)
    maxMessages: 100,      // Keep last N messages in state (default: 100)
  });

  return (
    <div>
      <p>{members.length} online</p>
      {messages.map(msg => (
        <div key={msg.id}>
          {msg.payload.kind === 'Text' ? msg.payload.data : JSON.stringify(msg.payload.data)}
        </div>
      ))}
      <button onClick={() => publish("Hello!")}>Send</button>
    </div>
  );
}
```

Source: `packages/react/src/useChannel.ts`

### useAgent() Hook with Streaming

Interact with AI agents, including streaming responses and tool calling:

```tsx
import { useAgent } from '@mtw/react';

function AIChat() {
  const {
    send,            // (content: string) => Promise<AgentResponse>
    messages,        // AgentMessage[] -- conversation history (user + assistant)
    isStreaming,     // boolean -- is the agent currently streaming?
    streamingText,   // string -- current streaming content (updates live)
    error,           // MtwError | null
    clearMessages,   // () => void
    addMessage,      // (msg: AgentMessage) => void
    registerTool,    // (name, handler) => unsubscribe
    agent,           // MtwAgentClient | null
    abort,           // () => void -- abort current stream
  } = useAgent("assistant", {
    includeHistory: true,     // Send conversation history as context (default: true)
    maxHistory: 20,           // Max history messages to include (default: 20)
    systemPrompt: "You are a helpful coding assistant.",
    timeout: 120000,          // Request timeout (default: 120000ms)
    tools: {
      search: async (params) => {
        const res = await fetch(`/api/search?q=${params.query}`);
        return await res.text();
      },
    },
  });

  const [input, setInput] = useState('');

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!input.trim()) return;
    setInput('');
    try {
      await send(input);
    } catch (err) {
      console.error('Agent error:', err);
    }
  };

  return (
    <div>
      {messages.map(msg => (
        <div key={msg.id} className={msg.role}>
          <strong>{msg.role}:</strong> {msg.content}
        </div>
      ))}

      {isStreaming && (
        <div className="assistant streaming">
          <strong>assistant:</strong> {streamingText}
        </div>
      )}

      <form onSubmit={handleSubmit}>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          disabled={isStreaming}
          placeholder="Ask me anything..."
        />
        <button type="submit" disabled={isStreaming}>Send</button>
        {isStreaming && <button onClick={abort}>Stop</button>}
      </form>
    </div>
  );
}
```

Source: `packages/react/src/useAgent.ts`

### useStream() Hook

For raw streaming data (e.g., 3D sync, audio):

```tsx
import { useStream } from '@mtw/react';

function DataStream() {
  const { data, subscribe } = useStream("sensor-data");

  return <pre>{JSON.stringify(data, null, 2)}</pre>;
}
```

### Complete Example: AI Chat Component

```tsx
import { MtwProvider, useAgent, useChannel } from '@mtw/react';
import { useState, FormEvent } from 'react';

function App() {
  return (
    <MtwProvider url="ws://localhost:8080/ws" auth={{ token: localStorage.getItem('token') || '' }}>
      <div style={{ display: 'flex', gap: '1rem' }}>
        <ChatRoom channel="chat.general" />
        <AIAssistant />
      </div>
    </MtwProvider>
  );
}

function ChatRoom({ channel }: { channel: string }) {
  const { messages, publish, members, subscribed } = useChannel(channel);
  const [input, setInput] = useState('');

  return (
    <div style={{ flex: 1 }}>
      <h2>#{channel} ({members.length} online)</h2>
      <div style={{ height: 400, overflow: 'auto' }}>
        {messages.map(msg => (
          <p key={msg.id}>
            {msg.payload.kind === 'Text' ? msg.payload.data : JSON.stringify(msg.payload.data)}
          </p>
        ))}
      </div>
      <form onSubmit={(e) => { e.preventDefault(); publish(input); setInput(''); }}>
        <input value={input} onChange={(e) => setInput(e.target.value)} disabled={!subscribed} />
      </form>
    </div>
  );
}

function AIAssistant() {
  const { send, messages, isStreaming, streamingText, abort } = useAgent("assistant");
  const [input, setInput] = useState('');

  return (
    <div style={{ flex: 1 }}>
      <h2>AI Assistant</h2>
      <div style={{ height: 400, overflow: 'auto' }}>
        {messages.map(msg => (
          <div key={msg.id} style={{ margin: '0.5rem 0' }}>
            <strong>{msg.role}:</strong> {msg.content}
          </div>
        ))}
        {isStreaming && (
          <div><strong>assistant:</strong> {streamingText}<span className="cursor">|</span></div>
        )}
      </div>
      <form onSubmit={async (e) => {
        e.preventDefault();
        const q = input; setInput('');
        await send(q);
      }}>
        <input value={input} onChange={(e) => setInput(e.target.value)} disabled={isStreaming} />
        {isStreaming ? <button type="button" onClick={abort}>Stop</button> : <button type="submit">Ask</button>}
      </form>
    </div>
  );
}
```

---

## Svelte Integration (@mtw/svelte)

```bash
npm install @mtw/svelte @mtw/client
```

### Connection Store

```svelte
<script>
  import { createMtwConnection } from '@mtw/svelte';

  const { connection, state, connected, error } = createMtwConnection({
    url: 'ws://localhost:8080/ws',
    auth: { token: 'your-jwt' },
  });
</script>

<p>State: {$state} | Connected: {$connected}</p>
{#if $error}<p class="error">{$error.message}</p>{/if}
```

### Channel Store

```svelte
<script>
  import { createChannelStore } from '@mtw/svelte';

  const { messages, members, publish, subscribed } = createChannelStore('chat.general');
</script>

<p>Members: {$members.length}</p>
{#each $messages as msg (msg.id)}
  <p>{msg.payload.kind === 'Text' ? msg.payload.data : '...'}</p>
{/each}

<button on:click={() => publish('Hello from Svelte!')}>Send</button>
```

### Agent Store

```svelte
<script>
  import { createAgentStore } from '@mtw/svelte';

  const { send, messages, isStreaming, streamingText } = createAgentStore('assistant');
</script>

{#each $messages as msg (msg.id)}
  <div class={msg.role}>{msg.content}</div>
{/each}
{#if $isStreaming}<div class="streaming">{$streamingText}</div>{/if}
```

---

## Vue Integration (@mtw/vue)

```bash
npm install @mtw/vue @mtw/client
```

### useMtw() Composable

```vue
<script setup>
import { useMtw } from '@mtw/vue';

const { connection, state, connected, error, reconnect, disconnect } = useMtw({
  url: 'ws://localhost:8080/ws',
  auth: { token: 'your-jwt' },
});
</script>

<template>
  <p>State: {{ state }} | Connected: {{ connected }}</p>
  <button @click="reconnect">Reconnect</button>
</template>
```

### useChannel() Composable

```vue
<script setup>
import { useChannel } from '@mtw/vue';

const { messages, members, publish, subscribed } = useChannel('chat.general');
const input = ref('');

function sendMessage() {
  publish(input.value);
  input.value = '';
}
</script>

<template>
  <p>Members: {{ members.length }}</p>
  <div v-for="msg in messages" :key="msg.id">
    {{ msg.payload.kind === 'Text' ? msg.payload.data : '...' }}
  </div>
  <input v-model="input" @keyup.enter="sendMessage" />
</template>
```

### useAgent() Composable

```vue
<script setup>
import { useAgent } from '@mtw/vue';

const { send, messages, isStreaming, streamingText, abort } = useAgent('assistant');
const input = ref('');

async function ask() {
  const q = input.value;
  input.value = '';
  await send(q);
}
</script>

<template>
  <div v-for="msg in messages" :key="msg.id">
    <strong>{{ msg.role }}:</strong> {{ msg.content }}
  </div>
  <div v-if="isStreaming">{{ streamingText }}</div>
  <input v-model="input" @keyup.enter="ask" :disabled="isStreaming" />
</template>
```

---

## Three.js Integration (@mtw/three)

Source: `packages/three/`

### Scene Sync

Synchronize Three.js scene state across clients in real time over a binary channel:

```typescript
import { useMtwScene } from '@mtw/three';

const { scene, camera, syncObject, onObjectUpdate } = useMtwScene({
  channel: '3d-sync',
  codec: 'binary',
});

// Sync a mesh's position/rotation/scale to all clients
syncObject(mesh, { properties: ['position', 'rotation', 'scale'] });

// Listen for updates from other clients
onObjectUpdate((objectId, transform) => {
  const obj = scene.getObjectByName(objectId);
  if (obj) {
    obj.position.copy(transform.position);
    obj.rotation.copy(transform.rotation);
  }
});
```

### Asset Streaming

Stream large assets (textures, models) in chunks over binary channels:

```typescript
import { useMtwAsset } from '@mtw/three';

const { loadAsset, progress } = useMtwAsset({ channel: 'assets' });

const texture = await loadAsset('textures/ground.png');
// Progress updates via the progress reactive value
```

---

## TypeScript Types Reference

All types are defined in `packages/client/src/types.ts` and mirror the Rust protocol:

```typescript
// Wire message
interface MtwMessage {
  id: string; type: MsgType; channel?: string;
  payload: Payload; metadata: Record<string, unknown>;
  timestamp: number; ref_id?: string;
}

// Payload variants
type Payload =
  | { kind: 'None' }
  | { kind: 'Text'; data: string }
  | { kind: 'Json'; data: unknown }
  | { kind: 'Binary'; data: string };

// Utility functions
textPayload(text: string): Payload
jsonPayload(data: unknown): Payload
binaryPayload(base64: string): Payload
emptyPayload(): Payload
createMessage(type: MsgType, payload: Payload, overrides?: Partial<MtwMessage>): MtwMessage
generateId(): string
```

---

## Next Steps

- [Protocol Guide](./protocol-guide.md) -- understand the wire format
- [AI Agents Guide](./ai-agents-guide.md) -- build agents for the frontend to use
- [Auth Guide](./auth-guide.md) -- authenticate connections
