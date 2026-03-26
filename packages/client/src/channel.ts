// =============================================================================
// @mtw/client — MtwChannel
// =============================================================================
//
// Channel subscription manager. Wraps a connection to provide a focused API
// for pub/sub messaging on a single channel.
// =============================================================================

import type { MtwConnection } from './connection';
import {
  type MtwMessage,
  type Payload,
  type SubscribeOptions,
  type MessageHandler,
  type ChannelMember,
  type ChannelEvents,
  type Unsubscribe,
  MtwError,
  createMessage,
  textPayload,
  jsonPayload,
  emptyPayload,
} from './types';

/**
 * MtwChannel represents an active subscription to a named channel on the
 * mtwRequest server.
 *
 * It handles:
 *   - Sending subscribe/unsubscribe messages to the server
 *   - Dispatching incoming messages to registered handlers
 *   - Publishing messages to the channel
 *   - Tracking channel members (presence)
 *
 * Usage:
 *   const channel = new MtwChannel(connection, "chat.general");
 *   await channel.subscribe();
 *   channel.onMessage((msg) => console.log(msg));
 *   await channel.publish("hello");
 *   await channel.unsubscribe();
 */
export class MtwChannel {
  private handlers = new Set<MessageHandler>();
  private joinHandlers = new Set<(member: ChannelMember) => void>();
  private leaveHandlers = new Set<(member: ChannelMember) => void>();
  private errorHandlers = new Set<(error: MtwError) => void>();
  private connectionUnsub: Unsubscribe | null = null;
  private _active = false;
  private _members = new Map<string, ChannelMember>();

  constructor(
    private readonly connection: MtwConnection,
    public readonly name: string,
    private readonly options: SubscribeOptions = {},
  ) {}

  /** Whether this channel subscription is active. */
  get active(): boolean {
    return this._active;
  }

  /** Current channel members (if presence tracking is enabled). */
  get members(): ChannelMember[] {
    return Array.from(this._members.values());
  }

  /** Number of members in the channel. */
  get memberCount(): number {
    return this._members.size;
  }

  // -----------------------------------------------------------------------
  // Subscribe / Unsubscribe
  // -----------------------------------------------------------------------

  /**
   * Subscribe to this channel on the server.
   *
   * Sends a Subscribe message and waits for the server Ack. Once subscribed,
   * incoming messages on this channel are dispatched to registered handlers.
   */
  async subscribe(): Promise<void> {
    if (this._active) return;

    if (!this.connection.connected) {
      throw MtwError.notConnected();
    }

    // Build subscribe message
    const msg = createMessage('subscribe', emptyPayload(), {
      channel: this.name,
      metadata: this.options.history !== undefined
        ? { history: this.options.history }
        : {},
    });

    // Send and wait for ack
    const response = await this.connection.request(msg);

    if (response.type === 'error') {
      throw new MtwError(
        'SUBSCRIBE_FAILED',
        `Failed to subscribe to ${this.name}`,
        response.payload,
      );
    }

    // Start listening for messages on this channel
    this.connectionUnsub = this.connection.onChannel(this.name, (incomingMsg) => {
      this.handleIncoming(incomingMsg);
    });

    this._active = true;

    // Process any historical messages included in the ack response
    if (response.payload.kind === 'Json' && response.payload.data) {
      const data = response.payload.data as { history?: MtwMessage[] };
      if (Array.isArray(data.history)) {
        for (const histMsg of data.history) {
          this.dispatchMessage(histMsg);
        }
      }
    }
  }

  /**
   * Unsubscribe from this channel.
   *
   * Sends an Unsubscribe message to the server and stops dispatching
   * incoming messages. All registered handlers are cleared.
   */
  async unsubscribe(): Promise<void> {
    if (!this._active) return;

    // Unregister from connection
    if (this.connectionUnsub) {
      this.connectionUnsub();
      this.connectionUnsub = null;
    }

    // Send unsubscribe message
    if (this.connection.connected) {
      try {
        const msg = createMessage('unsubscribe', emptyPayload(), {
          channel: this.name,
        });
        this.connection.send(msg);
      } catch {
        // Ignore send errors during unsubscribe
      }
    }

    this._active = false;
    this._members.clear();
  }

  // -----------------------------------------------------------------------
  // Publishing
  // -----------------------------------------------------------------------

  /**
   * Publish a text message to this channel.
   */
  publish(text: string): void;
  /**
   * Publish a JSON payload to this channel.
   */
  publish(data: Record<string, unknown>): void;
  /**
   * Publish a raw Payload to this channel.
   */
  publish(payload: Payload): void;
  publish(content: string | Record<string, unknown> | Payload): void {
    if (!this._active) {
      throw new MtwError('NOT_SUBSCRIBED', `Not subscribed to channel ${this.name}`);
    }

    let payload: Payload;
    if (typeof content === 'string') {
      payload = textPayload(content);
    } else if ('kind' in content && typeof (content as Payload).kind === 'string') {
      payload = content as Payload;
    } else {
      payload = jsonPayload(content);
    }

    const msg = createMessage('publish', payload, {
      channel: this.name,
    });

    this.connection.send(msg);
  }

  /**
   * Publish a binary payload to this channel.
   * Useful for 3D scene data, audio, or other binary content.
   */
  publishBinary(data: Uint8Array): void {
    if (!this._active) {
      throw new MtwError('NOT_SUBSCRIBED', `Not subscribed to channel ${this.name}`);
    }
    // Binary data is sent as a separate binary frame
    this.connection.sendBinary(data);
  }

  /**
   * Send a request on this channel and wait for a response.
   */
  async request(payload: Payload, timeoutMs = 30000): Promise<MtwMessage> {
    if (!this._active) {
      throw new MtwError('NOT_SUBSCRIBED', `Not subscribed to channel ${this.name}`);
    }
    const msg = createMessage('request', payload, { channel: this.name });
    return this.connection.request(msg, timeoutMs);
  }

  // -----------------------------------------------------------------------
  // Event handlers
  // -----------------------------------------------------------------------

  /**
   * Register a handler for incoming messages on this channel.
   * Returns an unsubscribe function.
   */
  onMessage<T = unknown>(handler: MessageHandler<T>): Unsubscribe {
    this.handlers.add(handler as MessageHandler);
    return () => {
      this.handlers.delete(handler as MessageHandler);
    };
  }

  /**
   * Register a handler for member join events.
   */
  onJoin(handler: (member: ChannelMember) => void): Unsubscribe {
    this.joinHandlers.add(handler);
    return () => {
      this.joinHandlers.delete(handler);
    };
  }

  /**
   * Register a handler for member leave events.
   */
  onLeave(handler: (member: ChannelMember) => void): Unsubscribe {
    this.leaveHandlers.add(handler);
    return () => {
      this.leaveHandlers.delete(handler);
    };
  }

  /**
   * Register a handler for errors.
   */
  onError(handler: (error: MtwError) => void): Unsubscribe {
    this.errorHandlers.add(handler);
    return () => {
      this.errorHandlers.delete(handler);
    };
  }

  /**
   * Wait for the next message on this channel.
   * Useful for one-shot message consumption.
   */
  nextMessage(timeoutMs = 30000): Promise<MtwMessage> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        unsub();
        reject(MtwError.timeout(`Next message on ${this.name}`));
      }, timeoutMs);

      const unsub = this.onMessage((msg) => {
        clearTimeout(timer);
        unsub();
        resolve(msg);
      });
    });
  }

  // -----------------------------------------------------------------------
  // Internal
  // -----------------------------------------------------------------------

  private handleIncoming(msg: MtwMessage): void {
    switch (msg.type) {
      case 'publish':
      case 'event':
      case 'response':
      case 'stream':
      case 'stream_end':
        this.dispatchMessage(msg);
        break;

      case 'ack':
        // Presence: member joined
        if (msg.metadata?.event === 'join') {
          const member: ChannelMember = {
            connId: (msg.metadata.conn_id as string) ?? '',
            userId: msg.metadata.user_id as string | undefined,
            joinedAt: msg.timestamp,
            metadata: msg.metadata,
          };
          this._members.set(member.connId, member);
          this.joinHandlers.forEach((h) => {
            try {
              h(member);
            } catch (err) {
              console.error(`[mtw] Error in join handler:`, err);
            }
          });
        }
        // Presence: member left
        if (msg.metadata?.event === 'leave') {
          const connId = (msg.metadata.conn_id as string) ?? '';
          const member = this._members.get(connId);
          if (member) {
            this._members.delete(connId);
            this.leaveHandlers.forEach((h) => {
              try {
                h(member);
              } catch (err) {
                console.error(`[mtw] Error in leave handler:`, err);
              }
            });
          }
        }
        break;

      case 'error':
        const error = new MtwError(
          'CHANNEL_ERROR',
          `Error on channel ${this.name}`,
          msg.payload,
        );
        this.errorHandlers.forEach((h) => {
          try {
            h(error);
          } catch (err) {
            console.error(`[mtw] Error in error handler:`, err);
          }
        });
        break;
    }
  }

  private dispatchMessage(msg: MtwMessage): void {
    // Extract typed data from the payload
    let data: unknown = null;
    if (msg.payload.kind === 'Text') {
      data = msg.payload.data;
    } else if (msg.payload.kind === 'Json') {
      data = msg.payload.data;
    } else if (msg.payload.kind === 'Binary') {
      data = msg.payload.data;
    }

    this.handlers.forEach((h) => {
      try {
        h(msg, data);
      } catch (err) {
        console.error(`[mtw] Error in message handler for ${this.name}:`, err);
      }
    });
  }
}
