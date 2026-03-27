// =============================================================================
// @mtw/client — MtwConnection
// =============================================================================
//
// WebSocket connection manager with auto-reconnect, ping/pong keep-alive,
// binary frame encoding/decoding, and typed event dispatch.
// =============================================================================

import {
  type MtwMessage,
  type ConnectOptions,
  type ConnectionState,
  type ConnectionEvents,
  type DisconnectInfo,
  type ConnMetadata,
  type Unsubscribe,
  MtwError,
  PROTOCOL_VERSION,
  FRAME_MAGIC,
  FrameType,
  MAX_FRAME_SIZE,
  createMessage,
  emptyPayload,
} from './types';

// ---------------------------------------------------------------------------
// Typed event emitter (minimal, no external deps)
// ---------------------------------------------------------------------------

class TypedEmitter<T extends Record<string, (...args: any[]) => void> = Record<string, (...args: any[]) => void>> {
  private listeners = new Map<keyof T, Set<Function>>();

  on<K extends keyof T>(event: K, fn: T[K]): Unsubscribe {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(fn);
    return () => {
      this.listeners.get(event)?.delete(fn);
    };
  }

  off<K extends keyof T>(event: K, fn: T[K]): void {
    this.listeners.get(event)?.delete(fn);
  }

  protected emit<K extends keyof T>(event: K, ...args: Parameters<T[K]>): void {
    this.listeners.get(event)?.forEach((fn) => {
      try {
        fn(...args);
      } catch (err) {
        console.error(`[mtw] Error in ${String(event)} handler:`, err);
      }
    });
  }

  removeAllListeners(event?: keyof T): void {
    if (event) {
      this.listeners.delete(event);
    } else {
      this.listeners.clear();
    }
  }
}

// ---------------------------------------------------------------------------
// Frame encoding / decoding (matches mtw-protocol/src/frame.rs)
// ---------------------------------------------------------------------------

/**
 * Encode an MtwMessage into the MTW binary frame format.
 *
 * Wire format:
 *   [MAGIC 3B] [VERSION 1B] [FRAME_TYPE 1B] [PAYLOAD_LEN 4B BE] [PAYLOAD NB]
 */
function encodeFrame(frameType: FrameType, payload: Uint8Array): ArrayBuffer {
  if (payload.byteLength > MAX_FRAME_SIZE) {
    throw MtwError.payloadTooLarge(payload.byteLength, MAX_FRAME_SIZE);
  }
  const header = 9;
  const buf = new ArrayBuffer(header + payload.byteLength);
  const view = new DataView(buf);
  const bytes = new Uint8Array(buf);

  // Magic: 'M', 'T', 'W'
  bytes[0] = FRAME_MAGIC[0];
  bytes[1] = FRAME_MAGIC[1];
  bytes[2] = FRAME_MAGIC[2];
  // Version
  view.setUint8(3, PROTOCOL_VERSION);
  // Frame type
  view.setUint8(4, frameType);
  // Payload length (big-endian u32)
  view.setUint32(5, payload.byteLength, false);
  // Payload
  bytes.set(payload, header);

  return buf;
}

function encodeMessage(msg: MtwMessage): ArrayBuffer {
  const json = JSON.stringify(msg);
  const encoder = new TextEncoder();
  const payload = encoder.encode(json);
  return encodeFrame(FrameType.Json, payload);
}

function encodePing(): ArrayBuffer {
  return encodeFrame(FrameType.Ping, new Uint8Array(0));
}

function encodePong(): ArrayBuffer {
  return encodeFrame(FrameType.Pong, new Uint8Array(0));
}

interface DecodedFrame {
  frameType: FrameType;
  payload: Uint8Array;
}

function decodeFrame(data: ArrayBuffer): DecodedFrame {
  const bytes = new Uint8Array(data);
  if (bytes.byteLength < 9) {
    throw MtwError.invalidFormat('Frame too short (need at least 9 bytes)');
  }

  // Verify magic
  if (bytes[0] !== FRAME_MAGIC[0] || bytes[1] !== FRAME_MAGIC[1] || bytes[2] !== FRAME_MAGIC[2]) {
    throw MtwError.invalidFormat('Invalid magic bytes');
  }

  // Check version
  const version = bytes[3];
  if (version !== PROTOCOL_VERSION) {
    throw MtwError.unsupportedVersion(version);
  }

  const frameType = bytes[4] as FrameType;
  const view = new DataView(data);
  const payloadLen = view.getUint32(5, false);

  if (payloadLen > MAX_FRAME_SIZE) {
    throw MtwError.payloadTooLarge(payloadLen, MAX_FRAME_SIZE);
  }

  if (bytes.byteLength < 9 + payloadLen) {
    throw MtwError.invalidFormat(
      `Expected ${payloadLen} bytes of payload, got ${bytes.byteLength - 9}`,
    );
  }

  const payload = bytes.slice(9, 9 + payloadLen);
  return { frameType, payload };
}

// ---------------------------------------------------------------------------
// MtwConnection
// ---------------------------------------------------------------------------

const DEFAULT_OPTIONS: Required<
  Pick<
    ConnectOptions,
    | 'reconnect'
    | 'maxReconnectAttempts'
    | 'reconnectDelay'
    | 'maxReconnectDelay'
    | 'pingInterval'
    | 'pongTimeout'
    | 'connectTimeout'
  >
> = {
  reconnect: true,
  maxReconnectAttempts: Infinity,
  reconnectDelay: 1000,
  maxReconnectDelay: 30000,
  pingInterval: 30000,
  pongTimeout: 10000,
  connectTimeout: 10000,
};

/**
 * MtwConnection manages a WebSocket connection to an mtwRequest server.
 *
 * Features:
 *   - Binary frame encoding/decoding (MTW wire protocol)
 *   - Auto-reconnect with exponential backoff
 *   - Ping/pong keep-alive
 *   - Request/response correlation
 *   - Typed event emitter
 *
 * Usage:
 *   const conn = new MtwConnection({ url: "ws://localhost:7741/ws" });
 *   conn.on('connected', (meta) => console.log('Connected:', meta.conn_id));
 *   conn.on('message', (msg) => console.log('Message:', msg));
 *   await conn.connect();
 */
export class MtwConnection extends TypedEmitter<ConnectionEvents> {
  private ws: WebSocket | null = null;
  private options: ConnectOptions & typeof DEFAULT_OPTIONS;
  private _state: ConnectionState = 'disconnected';
  private _connMetadata: ConnMetadata | null = null;
  private reconnectAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private pongTimer: ReturnType<typeof setTimeout> | null = null;
  private pendingRequests = new Map<
    string,
    { resolve: (msg: MtwMessage) => void; reject: (err: MtwError) => void; timer: ReturnType<typeof setTimeout> }
  >();
  private messageHandlers = new Map<string, Set<(msg: MtwMessage) => void>>();
  private intentionalClose = false;

  constructor(options: ConnectOptions) {
    super();
    this.options = { ...DEFAULT_OPTIONS, ...options };
  }

  /** Current connection state. */
  get state(): ConnectionState {
    return this._state;
  }

  /** Whether the connection is currently open. */
  get connected(): boolean {
    return this._state === 'connected';
  }

  /** Server-assigned connection metadata. */
  get connMetadata(): ConnMetadata | null {
    return this._connMetadata;
  }

  /** Server-assigned connection ID. */
  get connectionId(): string | null {
    return this._connMetadata?.conn_id ?? null;
  }

  // -----------------------------------------------------------------------
  // Connection lifecycle
  // -----------------------------------------------------------------------

  /** Open the WebSocket connection. */
  async connect(): Promise<ConnMetadata> {
    if (this._state === 'connected' || this._state === 'connecting') {
      throw new MtwError('ALREADY_CONNECTED', 'Connection is already open or opening');
    }

    this.intentionalClose = false;
    return this.doConnect();
  }

  /** Close the connection gracefully. */
  async close(): Promise<void> {
    this.intentionalClose = true;
    this.stopPing();
    this.clearReconnectTimer();

    if (this.ws) {
      this.setState('disconnecting');
      // Send disconnect message before closing
      try {
        this.sendRaw(createMessage('disconnect', emptyPayload()));
      } catch {
        // ignore send errors during close
      }
      this.ws.close(1000, 'Client closing');
      this.ws = null;
    }

    this.setState('disconnected');
    this.rejectAllPending(MtwError.notConnected());
  }

  // -----------------------------------------------------------------------
  // Messaging
  // -----------------------------------------------------------------------

  /** Send a message through the WebSocket. */
  send(msg: MtwMessage): void {
    if (!this.ws || this._state !== 'connected') {
      throw MtwError.notConnected();
    }
    const frame = encodeMessage(msg);
    this.ws.send(frame);
  }

  /** Send a raw binary frame. */
  sendBinary(data: Uint8Array): void {
    if (!this.ws || this._state !== 'connected') {
      throw MtwError.notConnected();
    }
    const frame = encodeFrame(FrameType.Binary, data);
    this.ws.send(frame);
  }

  /**
   * Send a request and wait for a correlated response.
   *
   * @param msg The request message (must have type 'request')
   * @param timeoutMs Timeout in ms (default: 30000)
   * @returns The response message
   */
  request(msg: MtwMessage, timeoutMs = 30000): Promise<MtwMessage> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingRequests.delete(msg.id);
        reject(MtwError.timeout(`Request ${msg.id}`));
      }, timeoutMs);

      this.pendingRequests.set(msg.id, { resolve, reject, timer });
      this.send(msg);
    });
  }

  /**
   * Register a handler for messages on a specific channel.
   * Returns an unsubscribe function.
   */
  onChannel(channel: string, handler: (msg: MtwMessage) => void): Unsubscribe {
    if (!this.messageHandlers.has(channel)) {
      this.messageHandlers.set(channel, new Set());
    }
    this.messageHandlers.get(channel)!.add(handler);
    return () => {
      this.messageHandlers.get(channel)?.delete(handler);
      if (this.messageHandlers.get(channel)?.size === 0) {
        this.messageHandlers.delete(channel);
      }
    };
  }

  // -----------------------------------------------------------------------
  // Internal: connection
  // -----------------------------------------------------------------------

  private doConnect(): Promise<ConnMetadata> {
    return new Promise((resolve, reject) => {
      this.setState('connecting');

      const url = this.buildUrl();
      const ws = new WebSocket(url, this.options.protocols);
      ws.binaryType = 'arraybuffer';
      this.ws = ws;

      // Connection timeout
      const connectTimer = setTimeout(() => {
        ws.close();
        const err = MtwError.timeout('Connection');
        this.emit('error', err);
        reject(err);
      }, this.options.connectTimeout);

      ws.onopen = () => {
        clearTimeout(connectTimer);
        // Send Connect message with auth
        const connectMsg = createMessage('connect', emptyPayload(), {
          metadata: this.options.auth
            ? {
                token: this.options.auth.token,
                api_key: this.options.auth.apiKey,
              }
            : {},
        });
        try {
          const frame = encodeMessage(connectMsg);
          ws.send(frame);
        } catch (err) {
          reject(MtwError.connectionFailed('Failed to send connect message'));
        }
      };

      let handshakeComplete = false;

      ws.onmessage = (event: MessageEvent) => {
        try {
          const data = event.data as ArrayBuffer;
          const { frameType, payload } = decodeFrame(data);

          if (frameType === FrameType.Ping) {
            ws.send(encodePong());
            return;
          }

          if (frameType === FrameType.Pong) {
            this.handlePong();
            return;
          }

          if (frameType === FrameType.Binary) {
            // Binary frames are dispatched as raw events
            // Channel extraction would require a header — for now broadcast
            return;
          }

          // JSON frame — decode as MtwMessage
          const decoder = new TextDecoder();
          const json = decoder.decode(payload);
          const msg = JSON.parse(json) as MtwMessage;

          // Handle connection handshake
          if (!handshakeComplete && msg.type === 'ack' && msg.ref_id) {
            handshakeComplete = true;
            this._connMetadata = {
              conn_id: (msg.metadata?.conn_id as string) ?? msg.id,
              connected_at: msg.timestamp,
              roles: [],
              claims: {},
            } as unknown as ConnMetadata;

            this.setState('connected');
            this.reconnectAttempt = 0;
            this.startPing();

            if (this.reconnectAttempt > 0) {
              this.emit('reconnected', this._connMetadata);
            } else {
              this.emit('connected', this._connMetadata);
            }

            resolve(this._connMetadata);
            return;
          }

          this.handleMessage(msg);
        } catch (err) {
          this.emit('error', new MtwError('DECODE_ERROR', `Failed to decode message: ${err}`));
        }
      };

      ws.onclose = (event: CloseEvent) => {
        clearTimeout(connectTimer);
        this.stopPing();

        const info: DisconnectInfo = {
          code: event.code,
          reason: event.reason,
          wasClean: event.wasClean,
        };

        this.emit('disconnected', info);
        this.rejectAllPending(MtwError.notConnected());

        if (!handshakeComplete) {
          reject(MtwError.connectionFailed(`WebSocket closed: ${event.code} ${event.reason}`));
        }

        if (!this.intentionalClose && this.options.reconnect) {
          this.scheduleReconnect();
        } else {
          this.setState('disconnected');
        }
      };

      ws.onerror = () => {
        // The close event will fire after this, which handles cleanup
        this.emit('error', MtwError.connectionFailed('WebSocket error'));
      };
    });
  }

  private buildUrl(): string {
    const url = new URL(this.options.url);
    // Attach auth as query param if present (some servers prefer this)
    if (this.options.auth?.token) {
      url.searchParams.set('token', this.options.auth.token);
    }
    if (this.options.auth?.apiKey) {
      url.searchParams.set('api_key', this.options.auth.apiKey);
    }
    return url.toString();
  }

  private sendRaw(msg: MtwMessage): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      const frame = encodeMessage(msg);
      this.ws.send(frame);
    }
  }

  // -----------------------------------------------------------------------
  // Internal: message dispatch
  // -----------------------------------------------------------------------

  private handleMessage(msg: MtwMessage): void {
    // Check for pending request/response correlation
    if (msg.ref_id && this.pendingRequests.has(msg.ref_id)) {
      const pending = this.pendingRequests.get(msg.ref_id)!;
      this.pendingRequests.delete(msg.ref_id);
      clearTimeout(pending.timer);

      if (msg.type === 'error') {
        const errData = msg.payload.kind === 'Json' ? msg.payload.data : msg.payload;
        pending.reject(
          new MtwError(
            'SERVER_ERROR',
            `Server error: ${JSON.stringify(errData)}`,
            errData,
          ),
        );
      } else {
        pending.resolve(msg);
      }
      return;
    }

    // Dispatch to channel handlers
    if (msg.channel) {
      const handlers = this.messageHandlers.get(msg.channel);
      if (handlers) {
        handlers.forEach((h) => {
          try {
            h(msg);
          } catch (err) {
            console.error(`[mtw] Error in channel handler for ${msg.channel}:`, err);
          }
        });
      }

      // Also check wildcard patterns (e.g. "chat.*" matches "chat.general")
      this.messageHandlers.forEach((handlerSet, pattern) => {
        if (pattern !== msg.channel && this.matchChannelPattern(pattern, msg.channel!)) {
          handlerSet.forEach((h) => {
            try {
              h(msg);
            } catch (err) {
              console.error(`[mtw] Error in channel handler for ${pattern}:`, err);
            }
          });
        }
      });
    }

    // Emit to global listeners
    this.emit('message', msg);
  }

  private matchChannelPattern(pattern: string, channel: string): boolean {
    if (pattern === channel) return true;
    if (pattern.endsWith('.*')) {
      const prefix = pattern.slice(0, -2);
      return channel.startsWith(prefix + '.');
    }
    if (pattern.endsWith('.**')) {
      const prefix = pattern.slice(0, -3);
      return channel.startsWith(prefix + '.') || channel === prefix;
    }
    return false;
  }

  // -----------------------------------------------------------------------
  // Internal: ping/pong keep-alive
  // -----------------------------------------------------------------------

  private startPing(): void {
    this.stopPing();
    this.pingTimer = setInterval(() => {
      if (this.ws && this.ws.readyState === WebSocket.OPEN) {
        this.ws.send(encodePing());
        this.pongTimer = setTimeout(() => {
          // No pong received — connection is dead
          this.emit('error', MtwError.timeout('Pong'));
          this.ws?.close(4000, 'Pong timeout');
        }, this.options.pongTimeout);
      }
    }, this.options.pingInterval);
  }

  private stopPing(): void {
    if (this.pingTimer) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
    if (this.pongTimer) {
      clearTimeout(this.pongTimer);
      this.pongTimer = null;
    }
  }

  private handlePong(): void {
    if (this.pongTimer) {
      clearTimeout(this.pongTimer);
      this.pongTimer = null;
    }
  }

  // -----------------------------------------------------------------------
  // Internal: reconnection with exponential backoff
  // -----------------------------------------------------------------------

  private scheduleReconnect(): void {
    if (this.reconnectAttempt >= this.options.maxReconnectAttempts) {
      this.setState('disconnected');
      this.emit(
        'error',
        new MtwError('MAX_RECONNECT', `Max reconnect attempts reached (${this.options.maxReconnectAttempts})`),
      );
      return;
    }

    this.setState('reconnecting');
    this.reconnectAttempt++;

    // Exponential backoff with jitter
    const baseDelay = this.options.reconnectDelay;
    const exponentialDelay = baseDelay * Math.pow(2, this.reconnectAttempt - 1);
    const jitter = Math.random() * baseDelay * 0.5;
    const delay = Math.min(exponentialDelay + jitter, this.options.maxReconnectDelay);

    this.emit('reconnecting', this.reconnectAttempt);

    this.reconnectTimer = setTimeout(async () => {
      try {
        await this.doConnect();
      } catch {
        // doConnect failure triggers onclose which calls scheduleReconnect again
      }
    }, delay);
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  // -----------------------------------------------------------------------
  // Internal: state management
  // -----------------------------------------------------------------------

  private setState(state: ConnectionState): void {
    if (this._state !== state) {
      this._state = state;
      this.emit('stateChange', state);
    }
  }

  private rejectAllPending(err: MtwError): void {
    this.pendingRequests.forEach((pending) => {
      clearTimeout(pending.timer);
      pending.reject(err);
    });
    this.pendingRequests.clear();
  }
}
