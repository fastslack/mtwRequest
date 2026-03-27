// =============================================================================
// @mtw/svelte — Agent store
// =============================================================================
//
// Svelte store for AI agent interactions with streaming support.
// =============================================================================

import { writable, get, type Readable, type Writable } from 'svelte/store';
import {
  MtwAgentClient,
  type AgentResponse,
  type AgentOptions,
  type AgentContextMessage,
  type AgentToolCall,
  type ToolHandler,
  type MtwError,
} from '@matware/mtw-request-ts-client';
import type { ConnectionStore } from './connection';

// ---------------------------------------------------------------------------
// Store types
// ---------------------------------------------------------------------------

export interface AgentMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  toolCalls?: AgentToolCall[];
}

export interface AgentStoreState {
  /** Conversation messages. */
  messages: AgentMessage[];
  /** Whether the agent is currently streaming. */
  isStreaming: boolean;
  /** Current streaming text (updates as chunks arrive). */
  streamingText: string;
  /** Last error. */
  error: MtwError | null;
}

export interface AgentStore extends Readable<AgentStoreState> {
  /** Send a message and stream the response. */
  send: (content: string, options?: AgentOptions) => Promise<AgentResponse>;
  /** Register a tool handler. */
  registerTool: (name: string, handler: ToolHandler) => () => void;
  /** Clear conversation history. */
  clearMessages: () => void;
  /** Abort current streaming response. */
  abort: () => void;
}

// ---------------------------------------------------------------------------
// Create agent store
// ---------------------------------------------------------------------------

export interface CreateAgentStoreOptions {
  /** Whether to include history as context (default: true). */
  includeHistory?: boolean;
  /** Max history messages to include (default: 20). */
  maxHistory?: number;
  /** Default timeout in ms (default: 120000). */
  timeout?: number;
  /** Initial tool handlers. */
  tools?: Record<string, ToolHandler>;
}

/**
 * Create a Svelte store for interacting with an AI agent.
 *
 * Usage:
 *   <script>
 *     import { createConnectionStore, createAgentStore } from '@mtw/svelte';
 *
 *     const connection = createConnectionStore();
 *     const agent = createAgentStore(connection, 'assistant');
 *
 *     let input = '';
 *
 *     async function handleSend() {
 *       await agent.send(input);
 *       input = '';
 *     }
 *   </script>
 *
 *   {#each $agent.messages as msg (msg.id)}
 *     <div class={msg.role}>{msg.content}</div>
 *   {/each}
 *
 *   {#if $agent.isStreaming}
 *     <div class="assistant">{$agent.streamingText}</div>
 *   {/if}
 *
 *   <input bind:value={input} disabled={$agent.isStreaming} />
 *   <button on:click={handleSend} disabled={$agent.isStreaming}>Send</button>
 */
export function createAgentStore(
  connectionStore: ConnectionStore,
  agentName: string,
  options: CreateAgentStoreOptions = {},
): AgentStore {
  const {
    includeHistory = true,
    maxHistory = 20,
    timeout = 120000,
    tools = {},
  } = options;

  const store: Writable<AgentStoreState> = writable({
    messages: [],
    isStreaming: false,
    streamingText: '',
    error: null,
  });

  let agentClient: MtwAgentClient | null = null;
  let aborted = false;

  // Register tools when agent client is available
  function ensureAgent(): MtwAgentClient {
    if (agentClient) return agentClient;

    const conn = connectionStore.getConnection();
    if (!conn) {
      throw new Error('Not connected — call connection.connect() first');
    }

    agentClient = new MtwAgentClient(conn, agentName);

    // Register initial tools
    for (const [name, handler] of Object.entries(tools)) {
      agentClient.registerTool(name, handler);
    }

    return agentClient;
  }

  async function send(content: string, agentOptions: AgentOptions = {}): Promise<AgentResponse> {
    const agent = ensureAgent();
    aborted = false;

    // Add user message
    const userMsg: AgentMessage = {
      id: `user-${Date.now()}-${Math.random().toString(36).slice(2)}`,
      role: 'user',
      content,
      timestamp: Date.now(),
    };

    store.update((s) => ({
      ...s,
      messages: [...s.messages, userMsg],
      isStreaming: true,
      streamingText: '',
      error: null,
    }));

    // Build context
    const opts: AgentOptions = {
      timeout,
      ...agentOptions,
    };

    if (includeHistory && !opts.context) {
      const state = get(store);
      const historySlice = state.messages.slice(-maxHistory);
      opts.context = historySlice.map(
        (msg): AgentContextMessage => ({
          role: msg.role,
          content: msg.content,
        }),
      );
    }

    try {
      let fullText = '';
      const allToolCalls: AgentToolCall[] = [];

      for await (const chunk of agent.stream(content, opts)) {
        if (aborted) break;

        fullText += chunk.text;
        if (chunk.toolCall) {
          allToolCalls.push(chunk.toolCall);
        }

        store.update((s) => ({ ...s, streamingText: fullText }));
      }

      const response: AgentResponse = {
        id: `asst-${Date.now()}-${Math.random().toString(36).slice(2)}`,
        text: fullText,
        toolCalls: allToolCalls,
        metadata: {},
      };

      const assistantMsg: AgentMessage = {
        id: response.id,
        role: 'assistant',
        content: fullText,
        timestamp: Date.now(),
        toolCalls: allToolCalls.length > 0 ? allToolCalls : undefined,
      };

      store.update((s) => ({
        ...s,
        messages: [...s.messages, assistantMsg],
        isStreaming: false,
        streamingText: '',
      }));

      return response;
    } catch (err) {
      store.update((s) => ({
        ...s,
        isStreaming: false,
        streamingText: '',
        error: err as MtwError,
      }));
      throw err;
    }
  }

  function registerTool(name: string, handler: ToolHandler): () => void {
    tools[name] = handler;
    if (agentClient) {
      return agentClient.registerTool(name, handler);
    }
    return () => {
      delete tools[name];
    };
  }

  function clearMessages(): void {
    store.update((s) => ({
      ...s,
      messages: [],
      streamingText: '',
      error: null,
    }));
  }

  function abort(): void {
    aborted = true;
  }

  return {
    subscribe: store.subscribe,
    send,
    registerTool,
    clearMessages,
    abort,
  };
}
