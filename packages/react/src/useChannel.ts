// =============================================================================
// @mtw/react — useChannel hook
// =============================================================================
//
// Hook for subscribing to an mtwRequest channel and receiving messages.
// =============================================================================

import { useEffect, useState, useCallback, useRef } from 'react';
import type {
  MtwMessage,
  ChannelMember,
  SubscribeOptions,
  MtwError,
  Payload,
} from '@mtw/client';
import { MtwChannel, textPayload, jsonPayload } from '@mtw/client';
import { useMtwContext } from './MtwProvider';

export interface UseChannelOptions extends SubscribeOptions {
  /** Whether to subscribe automatically (default: true) */
  autoSubscribe?: boolean;
  /** Maximum number of messages to keep in state (default: 100) */
  maxMessages?: number;
}

export interface UseChannelReturn<T = unknown> {
  /** Whether the channel subscription is active. */
  subscribed: boolean;
  /** Messages received on this channel. */
  messages: MtwMessage[];
  /** Current channel members (presence). */
  members: ChannelMember[];
  /** The last message received. */
  lastMessage: MtwMessage | null;
  /** Channel error, if any. */
  error: MtwError | null;
  /** Publish a text or JSON message to the channel. */
  publish: (content: string | Record<string, unknown>) => void;
  /** Subscribe to the channel (if autoSubscribe is false). */
  subscribe: () => Promise<void>;
  /** Unsubscribe from the channel. */
  unsubscribe: () => Promise<void>;
  /** Clear the message history. */
  clearMessages: () => void;
  /** The underlying MtwChannel instance. */
  channel: MtwChannel | null;
}

/**
 * Hook for subscribing to a channel and receiving messages.
 *
 * Must be used within an <MtwProvider>.
 *
 * Usage:
 *   function ChatRoom() {
 *     const { messages, publish, members, subscribed } = useChannel("chat.general");
 *
 *     return (
 *       <div>
 *         <div>Members: {members.length}</div>
 *         {messages.map(msg => (
 *           <div key={msg.id}>{msg.payload.kind === 'Text' ? msg.payload.data : '...'}</div>
 *         ))}
 *         <button onClick={() => publish("Hello!")}>Send</button>
 *       </div>
 *     );
 *   }
 */
export function useChannel<T = unknown>(
  channelName: string,
  options: UseChannelOptions = {},
): UseChannelReturn<T> {
  const { autoSubscribe = true, maxMessages = 100, ...subscribeOptions } = options;
  const ctx = useMtwContext();

  const [subscribed, setSubscribed] = useState(false);
  const [messages, setMessages] = useState<MtwMessage[]>([]);
  const [members, setMembers] = useState<ChannelMember[]>([]);
  const [lastMessage, setLastMessage] = useState<MtwMessage | null>(null);
  const [error, setError] = useState<MtwError | null>(null);

  const channelRef = useRef<MtwChannel | null>(null);
  const maxMessagesRef = useRef(maxMessages);
  maxMessagesRef.current = maxMessages;

  // Subscribe to the channel when connected
  useEffect(() => {
    if (!ctx.connected || !channelName) return;

    const ch = ctx.getChannel(channelName);
    channelRef.current = ch;

    const unsubs: Array<() => void> = [];

    // Message handler
    unsubs.push(
      ch.onMessage((msg) => {
        setMessages((prev) => {
          const next = [...prev, msg];
          if (next.length > maxMessagesRef.current) {
            return next.slice(next.length - maxMessagesRef.current);
          }
          return next;
        });
        setLastMessage(msg);
      }),
    );

    // Presence handlers
    unsubs.push(
      ch.onJoin((member) => {
        setMembers((prev) => [...prev.filter((m) => m.connId !== member.connId), member]);
      }),
    );

    unsubs.push(
      ch.onLeave((member) => {
        setMembers((prev) => prev.filter((m) => m.connId !== member.connId));
      }),
    );

    // Error handler
    unsubs.push(
      ch.onError((err) => {
        setError(err);
      }),
    );

    // Auto-subscribe
    if (autoSubscribe) {
      ch.subscribe()
        .then(() => setSubscribed(true))
        .catch((err) => setError(err));
    }

    return () => {
      unsubs.forEach((unsub) => unsub());
      ch.unsubscribe().catch(() => {});
      setSubscribed(false);
      channelRef.current = null;
    };
  }, [ctx.connected, channelName, autoSubscribe]); // eslint-disable-line react-hooks/exhaustive-deps

  const publish = useCallback(
    (content: string | Record<string, unknown>) => {
      const ch = channelRef.current;
      if (!ch || !ch.active) {
        console.warn(`[mtw] Cannot publish: not subscribed to ${channelName}`);
        return;
      }
      ch.publish(content as string);
    },
    [channelName],
  );

  const subscribe = useCallback(async () => {
    const ch = channelRef.current;
    if (ch) {
      await ch.subscribe();
      setSubscribed(true);
    }
  }, []);

  const unsubscribe = useCallback(async () => {
    const ch = channelRef.current;
    if (ch) {
      await ch.unsubscribe();
      setSubscribed(false);
    }
  }, []);

  const clearMessages = useCallback(() => {
    setMessages([]);
    setLastMessage(null);
  }, []);

  return {
    subscribed,
    messages,
    members,
    lastMessage,
    error,
    publish,
    subscribe,
    unsubscribe,
    clearMessages,
    channel: channelRef.current,
  };
}
