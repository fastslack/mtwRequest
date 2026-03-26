// =============================================================================
// @mtw/react — useStream hook
// =============================================================================
//
// Hook for raw streaming data from a channel. Useful for real-time binary
// data like 3D scene updates, audio streams, or custom protocols.
// =============================================================================

import { useEffect, useState, useCallback, useRef } from 'react';
import type { MtwMessage, Payload, Unsubscribe, MtwError } from '@mtw/client';
import { MtwChannel } from '@mtw/client';
import { useMtwContext } from './MtwProvider';

export interface UseStreamOptions {
  /** Whether to subscribe automatically (default: true) */
  autoSubscribe?: boolean;
  /** Buffer size — how many messages to keep (default: 1, latest only) */
  bufferSize?: number;
  /** Optional transform function for incoming data */
  transform?: (msg: MtwMessage) => unknown;
}

export interface UseStreamReturn<T = unknown> {
  /** The latest data received from the stream. */
  data: T | null;
  /** Buffer of recent data (length controlled by bufferSize). */
  buffer: T[];
  /** Whether the stream is active. */
  active: boolean;
  /** Error, if any. */
  error: MtwError | null;
  /** Manually subscribe to the stream. */
  subscribe: () => Promise<void>;
  /** Unsubscribe from the stream. */
  unsubscribe: () => Promise<void>;
  /** Send data to the stream channel. */
  send: (payload: string | Record<string, unknown> | Uint8Array) => void;
  /** Clear the buffer. */
  clear: () => void;
  /** Number of messages received since subscription. */
  count: number;
}

/**
 * Hook for raw streaming data on a channel.
 *
 * Unlike useChannel which stores full MtwMessage objects, useStream
 * extracts the payload data and optionally transforms it, making it
 * ideal for high-frequency binary or structured data streams.
 *
 * Usage:
 *   // Raw binary stream (e.g. 3D scene updates)
 *   const { data, active } = useStream<Float32Array>("3d-sync", {
 *     transform: (msg) => {
 *       if (msg.payload.kind === 'Binary') {
 *         return new Float32Array(atob(msg.payload.data).split('').map(c => c.charCodeAt(0)));
 *       }
 *       return null;
 *     }
 *   });
 *
 *   // JSON stream
 *   const { data } = useStream<{ x: number; y: number }>("mouse-positions");
 */
export function useStream<T = unknown>(
  channelName: string,
  options: UseStreamOptions = {},
): UseStreamReturn<T> {
  const { autoSubscribe = true, bufferSize = 1, transform } = options;
  const ctx = useMtwContext();

  const [data, setData] = useState<T | null>(null);
  const [buffer, setBuffer] = useState<T[]>([]);
  const [active, setActive] = useState(false);
  const [error, setError] = useState<MtwError | null>(null);
  const [count, setCount] = useState(0);

  const channelRef = useRef<MtwChannel | null>(null);
  const transformRef = useRef(transform);
  const bufferSizeRef = useRef(bufferSize);
  transformRef.current = transform;
  bufferSizeRef.current = bufferSize;

  useEffect(() => {
    if (!ctx.connected || !channelName) return;

    const ch = ctx.getChannel(channelName);
    channelRef.current = ch;

    const unsubs: Unsubscribe[] = [];

    unsubs.push(
      ch.onMessage((msg) => {
        let extracted: T;

        if (transformRef.current) {
          extracted = transformRef.current(msg) as T;
        } else {
          // Default: extract payload data
          if (msg.payload.kind === 'Json') {
            extracted = msg.payload.data as T;
          } else if (msg.payload.kind === 'Text') {
            extracted = msg.payload.data as T;
          } else if (msg.payload.kind === 'Binary') {
            extracted = msg.payload.data as T;
          } else {
            extracted = null as T;
          }
        }

        setData(extracted);
        setCount((c) => c + 1);
        setBuffer((prev) => {
          const next = [...prev, extracted];
          if (next.length > bufferSizeRef.current) {
            return next.slice(next.length - bufferSizeRef.current);
          }
          return next;
        });
      }),
    );

    unsubs.push(
      ch.onError((err) => {
        setError(err);
      }),
    );

    if (autoSubscribe) {
      ch.subscribe()
        .then(() => setActive(true))
        .catch((err) => setError(err));
    }

    return () => {
      unsubs.forEach((u) => u());
      ch.unsubscribe().catch(() => {});
      setActive(false);
      channelRef.current = null;
    };
  }, [ctx.connected, channelName, autoSubscribe]); // eslint-disable-line react-hooks/exhaustive-deps

  const subscribe = useCallback(async () => {
    const ch = channelRef.current;
    if (ch) {
      await ch.subscribe();
      setActive(true);
    }
  }, []);

  const unsubscribe = useCallback(async () => {
    const ch = channelRef.current;
    if (ch) {
      await ch.unsubscribe();
      setActive(false);
    }
  }, []);

  const send = useCallback(
    (payload: string | Record<string, unknown> | Uint8Array) => {
      const ch = channelRef.current;
      if (!ch || !ch.active) {
        console.warn(`[mtw] Cannot send: not subscribed to ${channelName}`);
        return;
      }

      if (payload instanceof Uint8Array) {
        ch.publishBinary(payload);
      } else {
        ch.publish(payload as string);
      }
    },
    [channelName],
  );

  const clear = useCallback(() => {
    setData(null);
    setBuffer([]);
    setCount(0);
  }, []);

  return {
    data,
    buffer,
    active,
    error,
    subscribe,
    unsubscribe,
    send,
    clear,
    count,
  };
}
