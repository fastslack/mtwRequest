// =============================================================================
// @mtw/svelte — Connection store
// =============================================================================
//
// Writable Svelte store managing the mtwRequest WebSocket connection.
// =============================================================================

import { writable, derived, get, type Writable, type Readable } from 'svelte/store';
import {
  MtwConnection,
  type ConnectOptions,
  type ConnectionState,
  type ConnMetadata,
  type DisconnectInfo,
  type MtwError,
} from '@mtw/client';

// ---------------------------------------------------------------------------
// Store types
// ---------------------------------------------------------------------------

export interface ConnectionStoreState {
  /** The MtwConnection instance. */
  connection: MtwConnection | null;
  /** Current connection state. */
  state: ConnectionState;
  /** Server-assigned connection metadata. */
  metadata: ConnMetadata | null;
  /** Last error. */
  error: MtwError | null;
  /** Current reconnect attempt. */
  reconnectAttempt: number;
}

export interface ConnectionStore extends Readable<ConnectionStoreState> {
  /** Connect to the server. */
  connect: (options: ConnectOptions) => Promise<ConnMetadata>;
  /** Disconnect from the server. */
  disconnect: () => Promise<void>;
  /** Reconnect to the server. */
  reconnect: () => Promise<void>;
  /** Get the underlying MtwConnection. */
  getConnection: () => MtwConnection | null;
}

// ---------------------------------------------------------------------------
// Create connection store
// ---------------------------------------------------------------------------

/**
 * Create a Svelte store for managing the mtwRequest connection.
 *
 * Usage:
 *   <script>
 *     import { createConnectionStore } from '@mtw/svelte';
 *
 *     const connection = createConnectionStore();
 *
 *     onMount(async () => {
 *       await connection.connect({ url: 'ws://localhost:8080/ws' });
 *     });
 *
 *     onDestroy(() => {
 *       connection.disconnect();
 *     });
 *   </script>
 *
 *   {#if $connection.state === 'connected'}
 *     <p>Connected as {$connection.metadata?.conn_id}</p>
 *   {:else}
 *     <p>Status: {$connection.state}</p>
 *   {/if}
 */
export function createConnectionStore(): ConnectionStore {
  const store: Writable<ConnectionStoreState> = writable({
    connection: null,
    state: 'disconnected' as ConnectionState,
    metadata: null,
    error: null,
    reconnectAttempt: 0,
  });

  let unsubs: Array<() => void> = [];

  function cleanup() {
    unsubs.forEach((u) => u());
    unsubs = [];
  }

  async function connect(options: ConnectOptions): Promise<ConnMetadata> {
    cleanup();

    const conn = new MtwConnection(options);

    store.update((s) => ({
      ...s,
      connection: conn,
      state: 'connecting',
      error: null,
    }));

    unsubs.push(
      conn.on('stateChange', (state) => {
        store.update((s) => ({ ...s, state }));
      }),
    );

    unsubs.push(
      conn.on('connected', (metadata) => {
        store.update((s) => ({
          ...s,
          metadata,
          error: null,
          reconnectAttempt: 0,
        }));
      }),
    );

    unsubs.push(
      conn.on('reconnecting', (attempt) => {
        store.update((s) => ({ ...s, reconnectAttempt: attempt }));
      }),
    );

    unsubs.push(
      conn.on('reconnected', (metadata) => {
        store.update((s) => ({
          ...s,
          metadata,
          error: null,
          reconnectAttempt: 0,
        }));
      }),
    );

    unsubs.push(
      conn.on('error', (error) => {
        store.update((s) => ({ ...s, error }));
      }),
    );

    const metadata = await conn.connect();
    return metadata;
  }

  async function disconnect(): Promise<void> {
    const state = get(store);
    if (state.connection) {
      await state.connection.close();
    }
    cleanup();
    store.set({
      connection: null,
      state: 'disconnected',
      metadata: null,
      error: null,
      reconnectAttempt: 0,
    });
  }

  async function reconnect(): Promise<void> {
    const state = get(store);
    if (state.connection) {
      await state.connection.close();
      await state.connection.connect();
    }
  }

  function getConnection(): MtwConnection | null {
    return get(store).connection;
  }

  return {
    subscribe: store.subscribe,
    connect,
    disconnect,
    reconnect,
    getConnection,
  };
}

/**
 * Derived store: whether the connection is established.
 */
export function isConnected(store: ConnectionStore): Readable<boolean> {
  return derived(store, ($s) => $s.state === 'connected');
}
