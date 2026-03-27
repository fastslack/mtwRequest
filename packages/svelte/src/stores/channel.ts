// =============================================================================
// @mtw/svelte — Channel store
// =============================================================================
//
// Svelte store for channel subscriptions and messaging.
// =============================================================================

import { writable, type Readable, type Writable } from 'svelte/store';
import {
  MtwChannel,
  type MtwMessage,
  type ChannelMember,
  type SubscribeOptions,
  type MtwError,
} from '@matware/mtw-request-ts-client';
import type { ConnectionStore } from './connection';

// ---------------------------------------------------------------------------
// Store types
// ---------------------------------------------------------------------------

export interface ChannelStoreState {
  /** Whether the channel subscription is active. */
  subscribed: boolean;
  /** Messages received on the channel. */
  messages: MtwMessage[];
  /** Current channel members (presence). */
  members: ChannelMember[];
  /** Last message received. */
  lastMessage: MtwMessage | null;
  /** Channel error. */
  error: MtwError | null;
}

export interface ChannelStore extends Readable<ChannelStoreState> {
  /** Subscribe to the channel. */
  subscribe: Readable<ChannelStoreState>['subscribe'];
  /** Connect to the channel (sends Subscribe message). */
  join: () => Promise<void>;
  /** Unsubscribe from the channel. */
  leave: () => Promise<void>;
  /** Publish a message to the channel. */
  publish: (content: string | Record<string, unknown>) => void;
  /** Clear stored messages. */
  clearMessages: () => void;
}

// ---------------------------------------------------------------------------
// Create channel store
// ---------------------------------------------------------------------------

/**
 * Create a Svelte store for a specific channel.
 *
 * Usage:
 *   <script>
 *     import { createConnectionStore } from '@mtw/svelte';
 *     import { createChannelStore } from '@mtw/svelte';
 *
 *     const connection = createConnectionStore();
 *     const chat = createChannelStore(connection, 'chat.general');
 *
 *     onMount(async () => {
 *       await connection.connect({ url: 'ws://localhost:7741/ws' });
 *       await chat.join();
 *     });
 *   </script>
 *
 *   {#each $chat.messages as msg (msg.id)}
 *     <div>{msg.payload.kind === 'Text' ? msg.payload.data : '...'}</div>
 *   {/each}
 *
 *   <button on:click={() => chat.publish('Hello!')}>Send</button>
 */
export function createChannelStore(
  connectionStore: ConnectionStore,
  channelName: string,
  options: SubscribeOptions & { maxMessages?: number } = {},
): ChannelStore {
  const { maxMessages = 100, ...subscribeOptions } = options;

  const store: Writable<ChannelStoreState> = writable({
    subscribed: false,
    messages: [],
    members: [],
    lastMessage: null,
    error: null,
  });

  let channel: MtwChannel | null = null;
  let cleanupFns: Array<() => void> = [];

  function cleanup() {
    cleanupFns.forEach((fn) => fn());
    cleanupFns = [];
  }

  async function join(): Promise<void> {
    const conn = connectionStore.getConnection();
    if (!conn) {
      throw new Error('Not connected — call connection.connect() first');
    }

    cleanup();

    channel = new MtwChannel(conn, channelName, subscribeOptions);

    cleanupFns.push(
      channel.onMessage((msg) => {
        store.update((s) => {
          const messages = [...s.messages, msg];
          const trimmed = messages.length > maxMessages
            ? messages.slice(messages.length - maxMessages)
            : messages;
          return { ...s, messages: trimmed, lastMessage: msg };
        });
      }),
    );

    cleanupFns.push(
      channel.onJoin((member) => {
        store.update((s) => ({
          ...s,
          members: [...s.members.filter((m) => m.connId !== member.connId), member],
        }));
      }),
    );

    cleanupFns.push(
      channel.onLeave((member) => {
        store.update((s) => ({
          ...s,
          members: s.members.filter((m) => m.connId !== member.connId),
        }));
      }),
    );

    cleanupFns.push(
      channel.onError((err) => {
        store.update((s) => ({ ...s, error: err }));
      }),
    );

    await channel.subscribe();
    store.update((s) => ({ ...s, subscribed: true }));
  }

  async function leave(): Promise<void> {
    if (channel) {
      await channel.unsubscribe();
      cleanup();
      channel = null;
      store.update((s) => ({
        ...s,
        subscribed: false,
        members: [],
      }));
    }
  }

  function publish(content: string | Record<string, unknown>): void {
    if (!channel || !channel.active) {
      console.warn(`[mtw] Cannot publish: not subscribed to ${channelName}`);
      return;
    }
    channel.publish(content as string);
  }

  function clearMessages(): void {
    store.update((s) => ({
      ...s,
      messages: [],
      lastMessage: null,
    }));
  }

  return {
    subscribe: store.subscribe,
    join,
    leave,
    publish,
    clearMessages,
  };
}
