// =============================================================================
// @mtw/client — MtwAgentClient
// =============================================================================
//
// High-level client for interacting with AI agents on the mtwRequest server.
// Handles task submission, streaming responses, and tool call orchestration.
// =============================================================================

import type { MtwConnection } from './connection';
import {
  type MtwMessage,
  type AgentOptions,
  type AgentChunk,
  type AgentToolCall,
  type AgentToolResult,
  type AgentResponse,
  type AgentContextMessage,
  type ToolHandler,
  type Unsubscribe,
  MtwError,
  createMessage,
  textPayload,
  jsonPayload,
} from './types';

/**
 * MtwAgentClient provides a high-level interface for interacting with AI
 * agents running on the mtwRequest server.
 *
 * Features:
 *   - Send tasks and receive complete responses
 *   - Stream responses chunk by chunk
 *   - Automatic tool call handling via registered handlers
 *   - Conversation context management
 *
 * Usage:
 *   const agent = new MtwAgentClient(connection, "assistant");
 *
 *   // Complete response
 *   const response = await agent.send("Hello!");
 *   console.log(response.text);
 *
 *   // Streaming
 *   for await (const chunk of agent.stream("Tell me a story")) {
 *     process.stdout.write(chunk.text);
 *   }
 *
 *   // Tool handling
 *   agent.registerTool("search", async (params) => {
 *     return JSON.stringify(await doSearch(params.query));
 *   });
 */
export class MtwAgentClient {
  private toolHandlers = new Map<string, ToolHandler>();
  private _isStreaming = false;
  private chunkListeners = new Set<(chunk: AgentChunk) => void>();
  private completeListeners = new Set<(response: AgentResponse) => void>();
  private errorListeners = new Set<(error: MtwError) => void>();

  constructor(
    private readonly connection: MtwConnection,
    public readonly agentName: string,
  ) {}

  /** Whether a streaming response is currently in progress. */
  get isStreaming(): boolean {
    return this._isStreaming;
  }

  // -----------------------------------------------------------------------
  // Task submission
  // -----------------------------------------------------------------------

  /**
   * Send a task to the agent and wait for the complete response.
   *
   * This collects all streaming chunks into a single response. For
   * real-time streaming, use `stream()` instead.
   */
  async send(content: string, options: AgentOptions = {}): Promise<AgentResponse> {
    const taskMsg = this.buildTaskMessage(content, options);
    const taskId = taskMsg.id;

    return new Promise<AgentResponse>((resolve, reject) => {
      let fullText = '';
      const toolCalls: AgentToolCall[] = [];
      const responseMetadata: Record<string, unknown> = {};

      const timeout = options.timeout ?? 120000;
      const timer = setTimeout(() => {
        cleanup();
        reject(MtwError.timeout(`Agent response from ${this.agentName}`));
      }, timeout);

      const cleanup = () => {
        clearTimeout(timer);
        unsub();
      };

      const unsub = this.listenForTask(taskId, {
        onChunk: (chunk) => {
          fullText += chunk.text;
          if (chunk.toolCall) {
            toolCalls.push(chunk.toolCall);
          }
        },
        onComplete: (msg) => {
          cleanup();
          // Extract any metadata from the complete message
          Object.assign(responseMetadata, msg.metadata);
          resolve({
            id: taskId,
            text: fullText,
            toolCalls,
            metadata: responseMetadata,
          });
        },
        onToolCall: async (toolCall, msg) => {
          await this.handleToolCall(toolCall, msg);
        },
        onError: (error) => {
          cleanup();
          reject(error);
        },
      });

      // Send the task
      try {
        this.connection.send(taskMsg);
      } catch (err) {
        cleanup();
        reject(err instanceof MtwError ? err : MtwError.notConnected());
      }
    });
  }

  /**
   * Stream a response from the agent.
   *
   * Returns an async iterable that yields AgentChunk objects as they arrive.
   * Tool calls are handled automatically if handlers are registered.
   *
   * Usage:
   *   for await (const chunk of agent.stream("Hello")) {
   *     process.stdout.write(chunk.text);
   *   }
   */
  async *stream(
    content: string,
    options: AgentOptions = {},
  ): AsyncGenerator<AgentChunk, AgentResponse, undefined> {
    const taskMsg = this.buildTaskMessage(content, options);
    const taskId = taskMsg.id;

    this._isStreaming = true;

    try {
      // Create a queue to bridge callback-based messaging to async iteration
      const queue: Array<
        | { type: 'chunk'; chunk: AgentChunk }
        | { type: 'complete'; msg: MtwMessage }
        | { type: 'error'; error: MtwError }
      > = [];
      let resolve: (() => void) | null = null;
      let done = false;

      const pushAndNotify = (
        item:
          | { type: 'chunk'; chunk: AgentChunk }
          | { type: 'complete'; msg: MtwMessage }
          | { type: 'error'; error: MtwError },
      ) => {
        queue.push(item);
        if (resolve) {
          resolve();
          resolve = null;
        }
      };

      const waitForItem = (): Promise<void> => {
        if (queue.length > 0) return Promise.resolve();
        return new Promise<void>((r) => {
          resolve = r;
        });
      };

      const unsub = this.listenForTask(taskId, {
        onChunk: (chunk) => {
          pushAndNotify({ type: 'chunk', chunk });
          // Emit to listeners
          this.chunkListeners.forEach((l) => l(chunk));
        },
        onComplete: (msg) => {
          pushAndNotify({ type: 'complete', msg });
        },
        onToolCall: async (toolCall, msg) => {
          await this.handleToolCall(toolCall, msg);
        },
        onError: (error) => {
          pushAndNotify({ type: 'error', error });
        },
      });

      // Send the task
      this.connection.send(taskMsg);

      // Yield chunks as they arrive
      let fullText = '';
      const toolCalls: AgentToolCall[] = [];

      while (!done) {
        await waitForItem();

        while (queue.length > 0) {
          const item = queue.shift()!;

          if (item.type === 'chunk') {
            fullText += item.chunk.text;
            if (item.chunk.toolCall) {
              toolCalls.push(item.chunk.toolCall);
            }
            yield item.chunk;
          } else if (item.type === 'complete') {
            done = true;
            unsub();
            const response: AgentResponse = {
              id: taskId,
              text: fullText,
              toolCalls,
              metadata: item.msg.metadata,
            };
            this.completeListeners.forEach((l) => l(response));
            return response;
          } else if (item.type === 'error') {
            done = true;
            unsub();
            this.errorListeners.forEach((l) => l(item.error));
            throw item.error;
          }
        }
      }

      unsub();
      return {
        id: taskId,
        text: fullText,
        toolCalls,
        metadata: {},
      };
    } finally {
      this._isStreaming = false;
    }
  }

  // -----------------------------------------------------------------------
  // Tool registration
  // -----------------------------------------------------------------------

  /**
   * Register a tool handler.
   *
   * When the agent requests this tool during a task, the handler is called
   * automatically and the result is sent back to continue the agent's work.
   */
  registerTool(toolName: string, handler: ToolHandler): Unsubscribe {
    this.toolHandlers.set(toolName, handler);
    return () => {
      this.toolHandlers.delete(toolName);
    };
  }

  // -----------------------------------------------------------------------
  // Event listeners
  // -----------------------------------------------------------------------

  /** Listen for streaming chunks from any active task. */
  onChunk(handler: (chunk: AgentChunk) => void): Unsubscribe {
    this.chunkListeners.add(handler);
    return () => {
      this.chunkListeners.delete(handler);
    };
  }

  /** Listen for task completions. */
  onComplete(handler: (response: AgentResponse) => void): Unsubscribe {
    this.completeListeners.add(handler);
    return () => {
      this.completeListeners.delete(handler);
    };
  }

  /** Listen for errors. */
  onError(handler: (error: MtwError) => void): Unsubscribe {
    this.errorListeners.add(handler);
    return () => {
      this.errorListeners.delete(handler);
    };
  }

  // -----------------------------------------------------------------------
  // Internal
  // -----------------------------------------------------------------------

  private buildTaskMessage(content: string, options: AgentOptions): MtwMessage {
    const metadata: Record<string, unknown> = {
      agent: this.agentName,
      ...options.metadata,
    };

    if (options.context && options.context.length > 0) {
      metadata.context = options.context;
    }

    return createMessage('agent_task', textPayload(content), { metadata });
  }

  private listenForTask(
    taskId: string,
    handlers: {
      onChunk: (chunk: AgentChunk) => void;
      onComplete: (msg: MtwMessage) => void;
      onToolCall: (toolCall: AgentToolCall, msg: MtwMessage) => Promise<void>;
      onError: (error: MtwError) => void;
    },
  ): Unsubscribe {
    const unsub = this.connection.on('message', (msg) => {
      // Only handle messages related to this task
      if (msg.ref_id !== taskId) return;

      switch (msg.type) {
        case 'agent_chunk': {
          const text =
            msg.payload.kind === 'Text'
              ? msg.payload.data
              : msg.payload.kind === 'Json'
                ? String((msg.payload.data as { text?: string })?.text ?? '')
                : '';

          handlers.onChunk({
            text,
            done: false,
            refId: taskId,
          });
          break;
        }

        case 'agent_tool_call': {
          if (msg.payload.kind === 'Json') {
            const data = msg.payload.data as {
              id: string;
              name: string;
              params: Record<string, unknown>;
            };
            const toolCall: AgentToolCall = {
              id: data.id,
              name: data.name,
              params: data.params ?? {},
            };

            // Also emit as a chunk with toolCall attached
            handlers.onChunk({
              text: '',
              done: false,
              toolCall,
              refId: taskId,
            });

            handlers.onToolCall(toolCall, msg).catch((err) => {
              console.error(`[mtw] Error handling tool call ${data.name}:`, err);
            });
          }
          break;
        }

        case 'agent_complete': {
          const finalText =
            msg.payload.kind === 'Text'
              ? msg.payload.data
              : '';

          if (finalText) {
            handlers.onChunk({
              text: finalText,
              done: true,
              refId: taskId,
            });
          }

          handlers.onComplete(msg);
          break;
        }

        case 'error': {
          const errPayload = msg.payload.kind === 'Json'
            ? msg.payload.data as { code?: number; message?: string }
            : { message: msg.payload.kind === 'Text' ? msg.payload.data : 'Unknown error' };

          handlers.onError(
            new MtwError(
              'AGENT_ERROR',
              (errPayload as { message?: string }).message ?? 'Agent error',
              errPayload,
            ),
          );
          break;
        }
      }
    });

    return unsub;
  }

  private async handleToolCall(toolCall: AgentToolCall, _originalMsg: MtwMessage): Promise<void> {
    const handler = this.toolHandlers.get(toolCall.name);

    if (!handler) {
      // No handler registered — send error back
      const resultMsg = createMessage(
        'agent_tool_result',
        jsonPayload({
          toolCallId: toolCall.id,
          content: `Tool "${toolCall.name}" is not registered on the client`,
          isError: true,
        }),
        { ref_id: _originalMsg.ref_id },
      );
      this.connection.send(resultMsg);
      return;
    }

    try {
      const result = await handler(toolCall.params);
      const resultMsg = createMessage(
        'agent_tool_result',
        jsonPayload({
          toolCallId: toolCall.id,
          content: result,
          isError: false,
        } satisfies AgentToolResult),
        { ref_id: _originalMsg.ref_id },
      );
      this.connection.send(resultMsg);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      const resultMsg = createMessage(
        'agent_tool_result',
        jsonPayload({
          toolCallId: toolCall.id,
          content: `Tool error: ${errorMessage}`,
          isError: true,
        } satisfies AgentToolResult),
        { ref_id: _originalMsg.ref_id },
      );
      this.connection.send(resultMsg);
    }
  }
}
