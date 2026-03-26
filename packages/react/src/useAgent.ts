// =============================================================================
// @mtw/react — useAgent hook
// =============================================================================
//
// Hook for interacting with AI agents on the mtwRequest server.
// Manages streaming state, message history, and tool call handling.
// =============================================================================

import { useEffect, useState, useCallback, useRef } from 'react';
import type {
  AgentChunk,
  AgentResponse,
  AgentOptions,
  AgentContextMessage,
  AgentToolCall,
  ToolHandler,
  MtwError,
} from '@mtw/client';
import { MtwAgentClient } from '@mtw/client';
import { useMtwContext } from './MtwProvider';

export interface AgentMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  toolCalls?: AgentToolCall[];
  metadata?: Record<string, unknown>;
}

export interface UseAgentOptions {
  /** Whether to include message history as context (default: true) */
  includeHistory?: boolean;
  /** Maximum number of history messages to send as context (default: 20) */
  maxHistory?: number;
  /** System prompt override */
  systemPrompt?: string;
  /** Default timeout in ms (default: 120000) */
  timeout?: number;
  /** Tool handlers to register */
  tools?: Record<string, ToolHandler>;
}

export interface UseAgentReturn {
  /** Send a message to the agent and stream the response. */
  send: (content: string) => Promise<AgentResponse>;
  /** Conversation messages (user + assistant). */
  messages: AgentMessage[];
  /** Whether the agent is currently streaming a response. */
  isStreaming: boolean;
  /** The current streaming text (updates as chunks arrive). */
  streamingText: string;
  /** Last error from the agent. */
  error: MtwError | null;
  /** Clear the conversation history. */
  clearMessages: () => void;
  /** Add a message to the conversation manually. */
  addMessage: (message: AgentMessage) => void;
  /** Register a tool handler. Returns an unsubscribe function. */
  registerTool: (name: string, handler: ToolHandler) => () => void;
  /** The underlying MtwAgentClient. */
  agent: MtwAgentClient | null;
  /** Abort the current streaming response. */
  abort: () => void;
}

/**
 * Hook for interacting with AI agents.
 *
 * Provides a chat-like interface with automatic streaming, message history
 * management, and tool call handling.
 *
 * Usage:
 *   function Chat() {
 *     const { send, messages, isStreaming, streamingText } = useAgent("assistant");
 *
 *     return (
 *       <div>
 *         {messages.map(msg => (
 *           <div key={msg.id}>{msg.role}: {msg.content}</div>
 *         ))}
 *         {isStreaming && <div>assistant: {streamingText}</div>}
 *         <form onSubmit={(e) => {
 *           e.preventDefault();
 *           const input = e.currentTarget.elements.namedItem('msg') as HTMLInputElement;
 *           send(input.value);
 *           input.value = '';
 *         }}>
 *           <input name="msg" disabled={isStreaming} />
 *         </form>
 *       </div>
 *     );
 *   }
 */
export function useAgent(
  agentName: string,
  options: UseAgentOptions = {},
): UseAgentReturn {
  const {
    includeHistory = true,
    maxHistory = 20,
    systemPrompt,
    timeout = 120000,
    tools = {},
  } = options;

  const ctx = useMtwContext();

  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingText, setStreamingText] = useState('');
  const [error, setError] = useState<MtwError | null>(null);

  const agentRef = useRef<MtwAgentClient | null>(null);
  const abortRef = useRef(false);
  const toolsRef = useRef(tools);
  toolsRef.current = tools;

  // Initialize agent when connected
  useEffect(() => {
    if (!ctx.connected || !agentName) return;

    const agent = ctx.getAgent(agentName);
    agentRef.current = agent;

    // Register tool handlers
    const toolUnsubs: Array<() => void> = [];
    for (const [name, handler] of Object.entries(toolsRef.current)) {
      toolUnsubs.push(agent.registerTool(name, handler));
    }

    return () => {
      toolUnsubs.forEach((unsub) => unsub());
      agentRef.current = null;
    };
  }, [ctx.connected, agentName]); // eslint-disable-line react-hooks/exhaustive-deps

  const send = useCallback(
    async (content: string): Promise<AgentResponse> => {
      const agent = agentRef.current;
      if (!agent) {
        throw new Error('Agent not initialized — ensure the provider is connected');
      }

      setError(null);
      abortRef.current = false;

      // Add user message
      const userMsg: AgentMessage = {
        id: `user-${Date.now()}-${Math.random().toString(36).slice(2)}`,
        role: 'user',
        content,
        timestamp: Date.now(),
      };
      setMessages((prev) => [...prev, userMsg]);

      // Build context from message history
      const agentOptions: AgentOptions = { timeout };

      if (includeHistory) {
        setMessages((prevMessages) => {
          const historySlice = prevMessages.slice(-maxHistory);
          agentOptions.context = historySlice.map(
            (msg): AgentContextMessage => ({
              role: msg.role,
              content: msg.content,
            }),
          );
          return prevMessages;
        });
      }

      if (systemPrompt) {
        agentOptions.metadata = { system_prompt: systemPrompt };
      }

      setIsStreaming(true);
      setStreamingText('');

      try {
        let fullText = '';
        const allToolCalls: AgentToolCall[] = [];
        let finalResponse: AgentResponse | null = null;

        for await (const chunk of agent.stream(content, agentOptions)) {
          if (abortRef.current) break;

          fullText += chunk.text;
          setStreamingText(fullText);

          if (chunk.toolCall) {
            allToolCalls.push(chunk.toolCall);
          }

          if (chunk.done) {
            break;
          }
        }

        // Build the final response
        finalResponse = {
          id: `asst-${Date.now()}-${Math.random().toString(36).slice(2)}`,
          text: fullText,
          toolCalls: allToolCalls,
          metadata: {},
        };

        // Add assistant message to history
        const assistantMsg: AgentMessage = {
          id: finalResponse.id,
          role: 'assistant',
          content: fullText,
          timestamp: Date.now(),
          toolCalls: allToolCalls.length > 0 ? allToolCalls : undefined,
        };
        setMessages((prev) => [...prev, assistantMsg]);

        return finalResponse;
      } catch (err) {
        const mtwErr = err as MtwError;
        setError(mtwErr);
        throw err;
      } finally {
        setIsStreaming(false);
        setStreamingText('');
      }
    },
    [includeHistory, maxHistory, systemPrompt, timeout],
  );

  const clearMessages = useCallback(() => {
    setMessages([]);
    setStreamingText('');
    setError(null);
  }, []);

  const addMessage = useCallback((message: AgentMessage) => {
    setMessages((prev) => [...prev, message]);
  }, []);

  const registerTool = useCallback(
    (name: string, handler: ToolHandler): (() => void) => {
      const agent = agentRef.current;
      if (agent) {
        return agent.registerTool(name, handler);
      }
      // If agent not ready, queue the registration
      toolsRef.current[name] = handler;
      return () => {
        delete toolsRef.current[name];
        agentRef.current?.registerTool(name, handler);
      };
    },
    [],
  );

  const abort = useCallback(() => {
    abortRef.current = true;
  }, []);

  return {
    send,
    messages,
    isStreaming,
    streamingText,
    error,
    clearMessages,
    addMessage,
    registerTool,
    agent: agentRef.current,
    abort,
  };
}
