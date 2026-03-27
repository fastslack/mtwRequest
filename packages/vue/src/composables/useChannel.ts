// =============================================================================
// @mtw/vue — useChannel composable
// =============================================================================
//
// Vue composable for subscribing to channels and handling messages.
// =============================================================================

import {
  ref,
  readonly,
  watch,
  onUnmounted,
  type Ref,
} from 'vue';
import {
  MtwChannel,
  type MtwMessage,
  type ChannelMember,
  type SubscribeOptions,
  type MtwError,
} from '@matware/mtw-request-ts-client';
import { useMtwInject } from './useMtw';

export interface UseChannelOptions extends SubscribeOptions {
  /** Subscribe automatically when connected (default: true) */
  autoSubscribe?: boolean;
  /** Maximum messages to keep (default: 100) */
  maxMessages?: number;
}

export interface UseChannelReturn {
  /** Whether the subscription is active. */
  subscribed: Ref<boolean>;
  /** Received messages. */
  messages: Ref<MtwMessage[]>;
  /** Channel members (presence). */
  members: Ref<ChannelMember[]>;
  /** Last received message. */
  lastMessage: Ref<MtwMessage | null>;
  /** Channel error. */
  error: Ref<MtwError | null>;
  /** Publish a message. */
  publish: (content: string | Record<string, unknown>) => void;
  /** Manually subscribe. */
  subscribe: () => Promise<void>;
  /** Unsubscribe. */
  unsubscribe: () => Promise<void>;
  /** Clear messages. */
  clearMessages: () => void;
}

/**
 * Composable for channel subscriptions.
 *
 * Usage:
 *   const { messages, publish, subscribed } = useChannel('chat.general');
 *
 *   // In template:
 *   // <div v-for="msg in messages" :key="msg.id">...</div>
 *   // <button @click="publish('hello')">Send</button>
 */
export function useChannel(
  channelName: string | Ref<string>,
  options: UseChannelOptions = {},
): UseChannelReturn {
  const { autoSubscribe = true, maxMessages = 100 } = options;
  const ctx = useMtwInject();

  const subscribed = ref(false);
  const messages = ref<MtwMessage[]>([]);
  const members = ref<ChannelMember[]>([]);
  const lastMessage = ref<MtwMessage | null>(null);
  const error = ref<MtwError | null>(null);

  let channel: MtwChannel | null = null;
  let cleanupFns: Array<() => void> = [];

  function cleanup() {
    cleanupFns.forEach((fn) => fn());
    cleanupFns = [];
  }

  async function doSubscribe(name: string) {
    cleanup();

    if (!ctx.connected.value) return;

    channel = ctx.getChannel(name);

    cleanupFns.push(
      channel.onMessage((msg) => {
        const current = messages.value;
        const next = [...current, msg];
        messages.value = next.length > maxMessages
          ? next.slice(next.length - maxMessages)
          : next;
        lastMessage.value = msg;
      }),
    );

    cleanupFns.push(
      channel.onJoin((member) => {
        members.value = [
          ...members.value.filter((m) => m.connId !== member.connId),
          member,
        ];
      }),
    );

    cleanupFns.push(
      channel.onLeave((member) => {
        members.value = members.value.filter((m) => m.connId !== member.connId);
      }),
    );

    cleanupFns.push(
      channel.onError((err) => {
        error.value = err;
      }),
    );

    try {
      await channel.subscribe();
      subscribed.value = true;
    } catch (err) {
      error.value = err as MtwError;
    }
  }

  async function doUnsubscribe() {
    if (channel) {
      await channel.unsubscribe();
      cleanup();
      channel = null;
      subscribed.value = false;
      members.value = [];
    }
  }

  function publish(content: string | Record<string, unknown>) {
    if (!channel || !channel.active) {
      console.warn(`[mtw] Cannot publish: not subscribed to channel`);
      return;
    }
    channel.publish(content as string);
  }

  function clearMessages() {
    messages.value = [];
    lastMessage.value = null;
  }

  // Watch connection state and channel name
  const nameRef = typeof channelName === 'string' ? ref(channelName) : channelName;

  watch(
    [ctx.connected, nameRef],
    async ([isConnected, name]) => {
      if (isConnected && name && autoSubscribe) {
        await doSubscribe(name);
      } else if (!isConnected) {
        subscribed.value = false;
      }
    },
    { immediate: true },
  );

  onUnmounted(() => {
    doUnsubscribe().catch(() => {});
  });

  return {
    subscribed: readonly(subscribed) as Ref<boolean>,
    messages: readonly(messages) as Ref<MtwMessage[]>,
    members: readonly(members) as Ref<ChannelMember[]>,
    lastMessage: readonly(lastMessage) as Ref<MtwMessage | null>,
    error: readonly(error) as Ref<MtwError | null>,
    publish,
    subscribe: () => doSubscribe(nameRef.value),
    unsubscribe: doUnsubscribe,
    clearMessages,
  };
}
