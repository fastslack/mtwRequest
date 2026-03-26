// =============================================================================
// @mtw/vue — useAgent composable
// =============================================================================
//
// Vue composable for AI agent interaction with streaming.
// =============================================================================

import {
  ref,
  readonly,
  watch,
  onUnmounted,
  type Ref,
} from 'vue';
import {
  MtwAgentClient,
  type AgentChunk,
  type AgentResponse,
  type AgentOptions,
  type AgentContextMessage,
  type AgentToolCall,
  type ToolHandler,
  type MtwError,
} from '@mtw/client';
import { useMtwInject } from './useMtw';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface AgentMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  toolCalls?: AgentToolCall[];
}

export interface UseAgentOptions {
  /** Include message history as context (default: true). */
  includeHistory?: boolean;
  /** Max history messages (default: 20). */
  maxHistory?: number;
  /** Default timeout in ms (default: 120000). */
  timeout?: number;
  /** Tool handlers. */
  tools?: Record<string, ToolHandler>;
}

export interface UseAgentReturn {
  /** Send a message and stream the response. */
  send: (content: string) => Promise<AgentResponse>;
  /** Conversation messages. */
  messages: Ref<AgentMessage[]>;
  /** Whether the agent is streaming. */
  isStreaming: Ref<boolean>;
  /** Current streaming text. */
  streamingText: Ref<string>;
  /** Last error. */
  error: Ref<MtwError | null>;
  /** Clear conversation. */
  clearMessages: () => void;
  /** Register a tool handler. */
  registerTool: (name: string, handler: ToolHandler) => () => void;
  /** Abort current stream. */
  abort: () => void;
}

// ---------------------------------------------------------------------------
// Composable
// ---------------------------------------------------------------------------

/**
 * Composable for AI agent interaction.
 *
 * Usage:
 *   const { send, messages, isStreaming, streamingText } = useAgent('assistant');
 *
 *   // In template:
 *   // <div v-for="msg in messages" :key="msg.id">{{ msg.role }}: {{ msg.content }}</div>
 *   // <div v-if="isStreaming">{{ streamingText }}</div>
 *   // <button @click="send(input)">Send</button>
 */
export function useAgent(
  agentName: string,
  options: UseAgentOptions = {},
): UseAgentReturn {
  const {
    includeHistory = true,
    maxHistory = 20,
    timeout = 120000,
    tools = {},
  } = options;

  const ctx = useMtwInject();

  const messages = ref<AgentMessage[]>([]);
  const isStreaming = ref(false);
  const streamingText = ref('');
  const error = ref<MtwError | null>(null);

  let agentClient: MtwAgentClient | null = null;
  let aborted = false;
  const toolUnsubs: Array<() => void> = [];

  // Initialize agent when connected
  watch(
    ctx.connected,
    (isConnected) => {
      if (isConnected) {
        agentClient = ctx.getAgent(agentName);
        // Register tools
        for (const [name, handler] of Object.entries(tools)) {
          toolUnsubs.push(agentClient.registerTool(name, handler));
        }
      } else {
        agentClient = null;
        toolUnsubs.forEach((u) => u());
        toolUnsubs.length = 0;
      }
    },
    { immediate: true },
  );

  async function send(content: string): Promise<AgentResponse> {
    if (!agentClient) {
      throw new Error('Agent not initialized — ensure the connection is established');
    }

    aborted = false;
    error.value = null;

    // Add user message
    const userMsg: AgentMessage = {
      id: `user-${Date.now()}-${Math.random().toString(36).slice(2)}`,
      role: 'user',
      content,
      timestamp: Date.now(),
    };
    messages.value = [...messages.value, userMsg];

    // Build options
    const agentOptions: AgentOptions = { timeout };

    if (includeHistory) {
      const historySlice = messages.value.slice(-maxHistory);
      agentOptions.context = historySlice.map(
        (msg): AgentContextMessage => ({
          role: msg.role,
          content: msg.content,
        }),
      );
    }

    isStreaming.value = true;
    streamingText.value = '';

    try {
      let fullText = '';
      const allToolCalls: AgentToolCall[] = [];

      for await (const chunk of agentClient.stream(content, agentOptions)) {
        if (aborted) break;

        fullText += chunk.text;
        if (chunk.toolCall) {
          allToolCalls.push(chunk.toolCall);
        }
        streamingText.value = fullText;
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

      messages.value = [...messages.value, assistantMsg];
      return response;
    } catch (err) {
      error.value = err as MtwError;
      throw err;
    } finally {
      isStreaming.value = false;
      streamingText.value = '';
    }
  }

  function clearMessages() {
    messages.value = [];
    streamingText.value = '';
    error.value = null;
  }

  function registerTool(name: string, handler: ToolHandler): () => void {
    if (agentClient) {
      return agentClient.registerTool(name, handler);
    }
    tools[name] = handler;
    return () => {
      delete tools[name];
    };
  }

  function abort() {
    aborted = true;
  }

  onUnmounted(() => {
    toolUnsubs.forEach((u) => u());
  });

  return {
    send,
    messages: readonly(messages) as Ref<AgentMessage[]>,
    isStreaming: readonly(isStreaming) as Ref<boolean>,
    streamingText: readonly(streamingText) as Ref<string>,
    error: readonly(error) as Ref<MtwError | null>,
    clearMessages,
    registerTool,
    abort,
  };
}
