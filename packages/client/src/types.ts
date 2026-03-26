// =============================================================================
// @mtw/client — TypeScript type definitions
// =============================================================================
//
// These types mirror the Rust protocol definitions in crates/mtw-protocol/src/
// exactly. They serve as the contract between the Rust server and all JS/TS
// clients (browser, Node.js, React, Svelte, Vue, Three.js).
// =============================================================================

// ---------------------------------------------------------------------------
// Wire protocol types (match mtw-protocol/src/message.rs)
// ---------------------------------------------------------------------------

/**
 * Message type enum — matches Rust MsgType with snake_case serde rename.
 *
 * Organized into four groups:
 *   - Transport: connection lifecycle
 *   - Data: request/response and events
 *   - Channel: pub/sub operations
 *   - Agent: AI agent interactions
 *   - System: errors and acknowledgments
 */
export type MsgType =
  // Transport lifecycle
  | 'connect'
  | 'disconnect'
  | 'ping'
  | 'pong'
  // Data exchange
  | 'request'
  | 'response'
  | 'event'
  | 'stream'
  | 'stream_end'
  // Channel operations
  | 'subscribe'
  | 'unsubscribe'
  | 'publish'
  // AI Agent
  | 'agent_task'
  | 'agent_chunk'
  | 'agent_tool_call'
  | 'agent_tool_result'
  | 'agent_complete'
  // System
  | 'error'
  | 'ack';

/**
 * Payload variants — matches Rust Payload enum.
 *
 * The Rust side uses `#[serde(tag = "kind", content = "data")]` so the wire
 * format is: `{ "kind": "Text", "data": "hello" }`
 */
export type Payload =
  | { kind: 'None' }
  | { kind: 'Text'; data: string }
  | { kind: 'Json'; data: unknown }
  | { kind: 'Binary'; data: string }; // base64-encoded on the wire

/**
 * MtwMessage — the core wire message format.
 *
 * Every message between client and server uses this shape. Matches the Rust
 * struct exactly, including serde rename of `msg_type` to `type`.
 */
export interface MtwMessage {
  /** Unique message ID (ULID) */
  id: string;
  /** Message type */
  type: MsgType;
  /** Target channel/room (optional) */
  channel?: string;
  /** Message payload */
  payload: Payload;
  /** Arbitrary metadata */
  metadata: Record<string, unknown>;
  /** Unix timestamp in milliseconds */
  timestamp: number;
  /** Reference to another message ID (for request/response correlation) */
  ref_id?: string;
}

// ---------------------------------------------------------------------------
// Connection types
// ---------------------------------------------------------------------------

/** Authentication options for connecting to the server. */
export interface AuthOptions {
  /** Bearer token */
  token?: string;
  /** API key */
  apiKey?: string;
  /** Custom headers to include in the WebSocket upgrade request */
  headers?: Record<string, string>;
}

/** Options for creating an MtwConnection. */
export interface ConnectOptions {
  /** Server URL (WebSocket endpoint) */
  url: string;
  /** Authentication */
  auth?: AuthOptions;
  /** Enable auto-reconnect (default: true) */
  reconnect?: boolean;
  /** Maximum reconnect attempts (default: Infinity) */
  maxReconnectAttempts?: number;
  /** Base delay between reconnect attempts in ms (default: 1000) */
  reconnectDelay?: number;
  /** Maximum reconnect delay in ms, for exponential backoff cap (default: 30000) */
  maxReconnectDelay?: number;
  /** Ping interval in ms (default: 30000) */
  pingInterval?: number;
  /** Pong timeout in ms — disconnect if no pong received (default: 10000) */
  pongTimeout?: number;
  /** Connection timeout in ms (default: 10000) */
  connectTimeout?: number;
  /** Protocols to request in the WebSocket handshake */
  protocols?: string[];
}

/** Connection state. */
export type ConnectionState =
  | 'connecting'
  | 'connected'
  | 'disconnecting'
  | 'disconnected'
  | 'reconnecting';

/** Disconnect reason passed to close handlers. */
export interface DisconnectInfo {
  code: number;
  reason: string;
  wasClean: boolean;
}

// ---------------------------------------------------------------------------
// Connection metadata (matches Rust ConnMetadata)
// ---------------------------------------------------------------------------

export interface ConnMetadata {
  conn_id: string;
  remote_addr?: string;
  user_agent?: string;
  auth?: AuthInfo;
  connected_at: number;
}

export interface AuthInfo {
  user_id?: string;
  token?: string;
  roles: string[];
  claims: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Channel types
// ---------------------------------------------------------------------------

/** Options for subscribing to a channel. */
export interface SubscribeOptions {
  /** Number of historical messages to replay (default: 0) */
  history?: number;
  /** Only receive messages matching this filter */
  filter?: Record<string, unknown>;
}

/** Channel member presence info. */
export interface ChannelMember {
  connId: string;
  userId?: string;
  joinedAt: number;
  metadata?: Record<string, unknown>;
}

/** Handler for incoming channel messages. */
export type MessageHandler<T = unknown> = (message: MtwMessage, data: T) => void;

/** Unsubscribe function returned by event registration. */
export type Unsubscribe = () => void;

// ---------------------------------------------------------------------------
// Agent types (match mtw-ai crate)
// ---------------------------------------------------------------------------

/** Options for sending a task to an agent. */
export interface AgentOptions {
  /** Conversation history / context messages */
  context?: AgentContextMessage[];
  /** Arbitrary metadata */
  metadata?: Record<string, unknown>;
  /** Timeout in ms (default: 120000) */
  timeout?: number;
}

/** A message in the agent's conversation context. */
export interface AgentContextMessage {
  role: 'user' | 'assistant' | 'system';
  content: string;
}

/** A streaming chunk from an agent. */
export interface AgentChunk {
  /** The text content of this chunk */
  text: string;
  /** Whether this is the final chunk */
  done: boolean;
  /** If the agent wants to call a tool */
  toolCall?: AgentToolCall;
  /** The ref_id correlating this chunk to the original task */
  refId: string;
}

/** An agent tool call request. */
export interface AgentToolCall {
  /** Unique ID for this tool call */
  id: string;
  /** Tool name */
  name: string;
  /** Tool parameters as a JSON object */
  params: Record<string, unknown>;
}

/** A tool result to send back to the agent. */
export interface AgentToolResult {
  /** The tool call ID this result corresponds to */
  toolCallId: string;
  /** The result content */
  content: string;
  /** Whether the tool call resulted in an error */
  isError?: boolean;
}

/** Complete agent response. */
export interface AgentResponse {
  /** Unique response ID */
  id: string;
  /** The complete response text */
  text: string;
  /** Any tool calls that were made during the response */
  toolCalls: AgentToolCall[];
  /** Metadata from the response */
  metadata: Record<string, unknown>;
}

/** Tool handler function. */
export type ToolHandler = (params: Record<string, unknown>) => Promise<string>;

// ---------------------------------------------------------------------------
// Frame types (match mtw-protocol/src/frame.rs)
// ---------------------------------------------------------------------------

/** Protocol version. */
export const PROTOCOL_VERSION = 1;

/** Magic bytes: 'M', 'T', 'W'. */
export const FRAME_MAGIC = new Uint8Array([0x4d, 0x54, 0x57]);

/** Frame type identifiers. */
export const enum FrameType {
  Json = 0x01,
  Binary = 0x02,
  Ping = 0x03,
  Pong = 0x04,
}

/** Maximum frame payload size (10 MB). */
export const MAX_FRAME_SIZE = 10 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/** MtwError — matches Rust ProtocolError variants. */
export class MtwError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly details?: unknown,
  ) {
    super(message);
    this.name = 'MtwError';
  }

  static invalidFormat(msg: string): MtwError {
    return new MtwError('INVALID_FORMAT', msg);
  }

  static unsupportedVersion(version: number): MtwError {
    return new MtwError('UNSUPPORTED_VERSION', `Unsupported protocol version: ${version}`);
  }

  static payloadTooLarge(size: number, max: number): MtwError {
    return new MtwError('PAYLOAD_TOO_LARGE', `Payload too large: ${size} bytes (max: ${max})`);
  }

  static connectionFailed(reason: string): MtwError {
    return new MtwError('CONNECTION_FAILED', `Connection failed: ${reason}`);
  }

  static timeout(operation: string): MtwError {
    return new MtwError('TIMEOUT', `Operation timed out: ${operation}`);
  }

  static notConnected(): MtwError {
    return new MtwError('NOT_CONNECTED', 'Client is not connected');
  }
}

// ---------------------------------------------------------------------------
// Event emitter types
// ---------------------------------------------------------------------------

/** Events emitted by MtwConnection. */
export interface ConnectionEvents {
  connected: (metadata: ConnMetadata) => void;
  disconnected: (info: DisconnectInfo) => void;
  reconnecting: (attempt: number) => void;
  reconnected: (metadata: ConnMetadata) => void;
  message: (message: MtwMessage) => void;
  error: (error: MtwError) => void;
  stateChange: (state: ConnectionState) => void;
}

/** Events emitted by MtwChannel. */
export interface ChannelEvents {
  message: (message: MtwMessage) => void;
  join: (member: ChannelMember) => void;
  leave: (member: ChannelMember) => void;
  error: (error: MtwError) => void;
}

/** Events emitted by MtwAgentClient. */
export interface AgentEvents {
  chunk: (chunk: AgentChunk) => void;
  toolCall: (toolCall: AgentToolCall) => void;
  complete: (response: AgentResponse) => void;
  error: (error: MtwError) => void;
}

// ---------------------------------------------------------------------------
// Utility types
// ---------------------------------------------------------------------------

/** Extract the payload data type from a Payload. */
export type PayloadData<P extends Payload> = P extends { data: infer D } ? D : never;

/** Create a typed Payload. */
export function textPayload(text: string): Payload {
  return { kind: 'Text', data: text };
}

export function jsonPayload(data: unknown): Payload {
  return { kind: 'Json', data };
}

export function binaryPayload(base64: string): Payload {
  return { kind: 'Binary', data: base64 };
}

export function emptyPayload(): Payload {
  return { kind: 'None' };
}

/** Generate a ULID-like unique ID (simplified — in production use a proper ULID library). */
export function generateId(): string {
  const timestamp = Date.now().toString(36);
  const random = Math.random().toString(36).substring(2, 12);
  return `${timestamp}${random}`.toUpperCase();
}

/** Create an MtwMessage with sensible defaults. */
export function createMessage(
  type: MsgType,
  payload: Payload,
  overrides?: Partial<MtwMessage>,
): MtwMessage {
  return {
    id: generateId(),
    type,
    payload,
    metadata: {},
    timestamp: Date.now(),
    ...overrides,
  };
}
