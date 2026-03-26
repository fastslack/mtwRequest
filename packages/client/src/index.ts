// =============================================================================
// @mtw/client — Universal JavaScript client for mtwRequest
// =============================================================================

export { MtwConnection } from './connection';
export { MtwChannel } from './channel';
export { MtwAgentClient } from './agent';

// Re-export all types
export type {
  MtwMessage,
  MsgType,
  Payload,
  ConnectOptions,
  AuthOptions,
  ConnectionState,
  ConnectionEvents,
  DisconnectInfo,
  ConnMetadata,
  AuthInfo,
  SubscribeOptions,
  ChannelMember,
  ChannelEvents,
  MessageHandler,
  Unsubscribe,
  AgentOptions,
  AgentContextMessage,
  AgentChunk,
  AgentToolCall,
  AgentToolResult,
  AgentResponse,
  AgentEvents,
  ToolHandler,
  PayloadData,
} from './types';

export {
  MtwError,
  PROTOCOL_VERSION,
  FRAME_MAGIC,
  FrameType,
  MAX_FRAME_SIZE,
  textPayload,
  jsonPayload,
  binaryPayload,
  emptyPayload,
  generateId,
  createMessage,
} from './types';

// ---------------------------------------------------------------------------
// Convenience factory function
// ---------------------------------------------------------------------------

import { MtwConnection } from './connection';
import { MtwChannel } from './channel';
import { MtwAgentClient } from './agent';
import type { ConnectOptions, ConnMetadata, SubscribeOptions } from './types';

/**
 * Connect to an mtwRequest server.
 *
 * This is a convenience function that creates an MtwConnection, connects,
 * and returns a high-level client object with channel and agent helpers.
 *
 * Usage:
 *   const client = await connect({ url: "ws://localhost:8080/ws", auth: { token: "..." } });
 *   const channel = await client.channel("chat.general");
 *   const agent = client.agent("assistant");
 *   const response = await agent.send("Hello!");
 *   await client.close();
 */
export async function connect(options: ConnectOptions): Promise<MtwClient> {
  const connection = new MtwConnection(options);
  const metadata = await connection.connect();
  return new MtwClient(connection, metadata);
}

/**
 * High-level mtwRequest client returned by `connect()`.
 *
 * Wraps MtwConnection with convenience methods for channel management
 * and agent interaction.
 */
export class MtwClient {
  private channels = new Map<string, MtwChannel>();
  private agents = new Map<string, MtwAgentClient>();

  constructor(
    /** The underlying WebSocket connection. */
    public readonly connection: MtwConnection,
    /** Connection metadata from the server. */
    public readonly metadata: ConnMetadata,
  ) {}

  /** Whether the client is connected. */
  get connected(): boolean {
    return this.connection.connected;
  }

  /** The server-assigned connection ID. */
  get connectionId(): string | null {
    return this.connection.connectionId;
  }

  /**
   * Subscribe to a channel and return an MtwChannel handle.
   *
   * If already subscribed to this channel, returns the existing handle.
   */
  async channel(name: string, options?: SubscribeOptions): Promise<MtwChannel> {
    if (this.channels.has(name)) {
      return this.channels.get(name)!;
    }

    const ch = new MtwChannel(this.connection, name, options);
    await ch.subscribe();
    this.channels.set(name, ch);
    return ch;
  }

  /**
   * Create an agent interaction handle.
   *
   * If an agent handle with this name already exists, returns the existing one.
   */
  agent(name: string): MtwAgentClient {
    if (this.agents.has(name)) {
      return this.agents.get(name)!;
    }

    const ag = new MtwAgentClient(this.connection, name);
    this.agents.set(name, ag);
    return ag;
  }

  /**
   * Close the connection and clean up all channels and agents.
   */
  async close(): Promise<void> {
    // Unsubscribe from all channels
    const unsubPromises = Array.from(this.channels.values()).map((ch) => ch.unsubscribe());
    await Promise.allSettled(unsubPromises);
    this.channels.clear();
    this.agents.clear();

    await this.connection.close();
  }
}
