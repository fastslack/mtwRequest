// =============================================================================
// @matware/mtw-request-svelte — Global mtw instance
// =============================================================================
//
// Singleton pattern: connect once, use everywhere.
//
// Usage:
//   // layout.svelte (connect once)
//   import { mtw } from '@matware/mtw-request-svelte';
//   await mtw.connect({ url: 'ws://localhost:7741/ws' });
//
//   // any component (use channels as stores)
//   import { channel, agent } from '@matware/mtw-request-svelte';
//   const dashboard = channel('dashboard');
//   const chat = channel('chat.general');
//   const ai = agent('assistant');
//
//   // template
//   <p>{$dashboard?.revenue}</p>
//   {#each $chat.messages as msg}...{/each}

import { writable, derived, type Readable } from 'svelte/store';
import {
  MtwConnection,
  MtwChannel,
  MtwAgentClient,
  type ConnectOptions,
  type ConnectionState,
  type ConnMetadata,
  type MtwMessage,
  type SubscribeOptions,
  type AgentOptions,
  type AgentResponse,
  type ToolHandler,
} from '@matware/mtw-request-ts-client';

// ---------------------------------------------------------------------------
// Singleton connection
// ---------------------------------------------------------------------------

let connection: MtwConnection | null = null;
const connectionState = writable<ConnectionState>('disconnected');
const connectionMeta = writable<ConnMetadata | null>(null);

export const mtw = {
  /** Connect to mtwRequest server. Call once at app startup. */
  async connect(options: ConnectOptions): Promise<ConnMetadata> {
    if (connection) {
      await connection.close();
    }
    connection = new MtwConnection(options);

    connection.on('stateChange', (state) => connectionState.set(state));
    connection.on('connected', (meta) => connectionMeta.set(meta));
    connection.on('reconnected', (meta) => connectionMeta.set(meta));

    const meta = await connection.connect();
    return meta;
  },

  /** Disconnect from the server. */
  async disconnect(): Promise<void> {
    if (connection) {
      await connection.close();
      connection = null;
      connectionState.set('disconnected');
      connectionMeta.set(null);
    }
  },

  /** Current connection state as a readable store. */
  state: { subscribe: connectionState.subscribe } as Readable<ConnectionState>,

  /** Connection metadata as a readable store. */
  metadata: { subscribe: connectionMeta.subscribe } as Readable<ConnMetadata | null>,

  /** Whether connected (derived store). */
  connected: derived(connectionState, (s) => s === 'connected'),

  /** Get the raw MtwConnection (for advanced use). */
  getConnection(): MtwConnection | null {
    return connection;
  },
};

// ---------------------------------------------------------------------------
// channel() — reactive channel store with one line
// ---------------------------------------------------------------------------

interface ChannelData<T = unknown> extends Readable<T | null> {
  /** All messages received. */
  messages: Readable<MtwMessage[]>;
  /** Publish data to the channel. */
  publish: (content: string | Record<string, unknown>) => void;
  /** Leave the channel. */
  leave: () => Promise<void>;
}

const activeChannels = new Map<string, ChannelData>();

/**
 * Subscribe to a channel and get a reactive store of the latest data.
 *
 * The returned store auto-subscribes on first use and contains the
 * latest message payload data (unwrapped from MtwMessage).
 *
 * Usage:
 *   const dashboard = channel('dashboard');
 *   <p>{$dashboard?.revenue}</p>
 *
 *   const chat = channel('chat.general');
 *   {#each $chat.messages as msg}...{/each}
 *   chat.publish({ user: 'me', text: 'hello' });
 */
export function channel<T = unknown>(
  name: string,
  options: SubscribeOptions & { maxMessages?: number } = {},
): ChannelData<T> {
  if (activeChannels.has(name)) {
    return activeChannels.get(name)! as ChannelData<T>;
  }

  const { maxMessages = 200, ...subOptions } = options;

  const latestData = writable<T | null>(null);
  const allMessages = writable<MtwMessage[]>([]);
  let mtwChannel: MtwChannel | null = null;
  let joined = false;

  async function ensureJoined(): Promise<void> {
    if (joined || !connection) return;

    mtwChannel = new MtwChannel(connection, name, subOptions);

    mtwChannel.onMessage((msg) => {
      // Unwrap payload to raw data
      let data: unknown = null;
      if (msg.payload.kind === 'Json') data = msg.payload.data;
      else if (msg.payload.kind === 'Text') data = msg.payload.data;

      latestData.set(data as T);
      allMessages.update((msgs) => {
        const updated = [...msgs, msg];
        return updated.length > maxMessages
          ? updated.slice(updated.length - maxMessages)
          : updated;
      });
    });

    await mtwChannel.subscribe();
    joined = true;
  }

  function publish(content: string | Record<string, unknown>): void {
    if (mtwChannel && mtwChannel.active) {
      mtwChannel.publish(content as string);
    }
  }

  async function leave(): Promise<void> {
    if (mtwChannel) {
      await mtwChannel.unsubscribe();
      mtwChannel = null;
      joined = false;
      activeChannels.delete(name);
    }
  }

  // Auto-join when the store is first subscribed to
  const store: ChannelData<T> = {
    subscribe(run, invalidate) {
      ensureJoined();
      return latestData.subscribe(run, invalidate);
    },
    messages: { subscribe: allMessages.subscribe },
    publish,
    leave,
  };

  activeChannels.set(name, store as ChannelData);
  return store;
}

// ---------------------------------------------------------------------------
// agent() — reactive AI agent with streaming
// ---------------------------------------------------------------------------

interface AgentData extends Readable<AgentStoreData> {
  /** Send a message and stream the response. */
  send: (content: string, options?: AgentOptions) => Promise<AgentResponse>;
  /** Register a tool handler. */
  registerTool: (name: string, handler: ToolHandler) => () => void;
  /** Clear conversation. */
  clear: () => void;
}

interface AgentStoreData {
  messages: Array<{ role: 'user' | 'assistant'; content: string; timestamp: number }>;
  isStreaming: boolean;
  streamingText: string;
  error: string | null;
}

/**
 * Create a reactive AI agent store.
 *
 * Usage:
 *   const ai = agent('assistant');
 *   await ai.send('Hello!');
 *
 *   {#each $ai.messages as msg}
 *     <div class={msg.role}>{msg.content}</div>
 *   {/each}
 *   {#if $ai.isStreaming}
 *     <div class="assistant">{$ai.streamingText}</div>
 *   {/if}
 */
export function agent(name: string): AgentData {
  const store = writable<AgentStoreData>({
    messages: [],
    isStreaming: false,
    streamingText: '',
    error: null,
  });

  let client: MtwAgentClient | null = null;

  function ensureClient(): MtwAgentClient {
    if (client) return client;
    if (!connection) throw new Error('Not connected — call mtw.connect() first');
    client = new MtwAgentClient(connection, name);
    return client;
  }

  async function send(content: string, options: AgentOptions = {}): Promise<AgentResponse> {
    const ag = ensureClient();

    store.update((s) => ({
      ...s,
      messages: [...s.messages, { role: 'user' as const, content, timestamp: Date.now() }],
      isStreaming: true,
      streamingText: '',
      error: null,
    }));

    try {
      let fullText = '';
      for await (const chunk of ag.stream(content, options)) {
        fullText += chunk.text;
        store.update((s) => ({ ...s, streamingText: fullText }));
      }

      const response: AgentResponse = {
        id: `asst-${Date.now()}`,
        text: fullText,
        toolCalls: [],
        metadata: {},
      };

      store.update((s) => ({
        ...s,
        messages: [
          ...s.messages,
          { role: 'assistant' as const, content: fullText, timestamp: Date.now() },
        ],
        isStreaming: false,
        streamingText: '',
      }));

      return response;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      store.update((s) => ({ ...s, isStreaming: false, streamingText: '', error: msg }));
      throw err;
    }
  }

  function registerTool(toolName: string, handler: ToolHandler): () => void {
    return ensureClient().registerTool(toolName, handler);
  }

  function clear(): void {
    store.update(() => ({
      messages: [],
      isStreaming: false,
      streamingText: '',
      error: null,
    }));
  }

  return {
    subscribe: store.subscribe,
    send,
    registerTool,
    clear,
  };
}
