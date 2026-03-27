// =============================================================================
// @mtw/vue — useMtw composable
// =============================================================================
//
// Vue composable for managing the mtwRequest WebSocket connection.
// =============================================================================

import {
  ref,
  readonly,
  onMounted,
  onUnmounted,
  provide,
  inject,
  toRaw,
  type Ref,
  type InjectionKey,
} from 'vue';
import {
  MtwConnection,
  MtwChannel,
  MtwAgentClient,
  type ConnectOptions,
  type ConnectionState,
  type ConnMetadata,
  type MtwError,
} from '@matware/mtw-request-ts-client';

// ---------------------------------------------------------------------------
// Injection key
// ---------------------------------------------------------------------------

export interface MtwContext {
  connection: Ref<MtwConnection | null>;
  state: Ref<ConnectionState>;
  connected: Ref<boolean>;
  metadata: Ref<ConnMetadata | null>;
  error: Ref<MtwError | null>;
  reconnectAttempt: Ref<number>;
  connect: (options?: ConnectOptions) => Promise<ConnMetadata>;
  disconnect: () => Promise<void>;
  reconnect: () => Promise<void>;
  getChannel: (name: string) => MtwChannel;
  getAgent: (name: string) => MtwAgentClient;
}

export const MTW_INJECTION_KEY: InjectionKey<MtwContext> = Symbol('mtw');

/**
 * Inject the mtwRequest context provided by useMtwProvider().
 * Must be called inside a component that has useMtwProvider() in its ancestor.
 */
export function useMtwInject(): MtwContext {
  const ctx = inject(MTW_INJECTION_KEY);
  if (!ctx) {
    throw new Error(
      'useMtw() must be used in a component under a parent that calls useMtwProvider()',
    );
  }
  return ctx;
}

// ---------------------------------------------------------------------------
// useMtwProvider — call in root component
// ---------------------------------------------------------------------------

/**
 * Create and provide the mtwRequest connection context.
 *
 * Call this in your root/layout component. Child components use useMtw()
 * to access the connection.
 *
 * Usage:
 *   // App.vue setup
 *   const { connected, state } = useMtwProvider({
 *     url: 'ws://localhost:7741/ws',
 *     auth: { token: 'my-token' },
 *   });
 */
export function useMtwProvider(options?: ConnectOptions & { autoConnect?: boolean }) {
  const connection = ref<MtwConnection | null>(null);
  const state = ref<ConnectionState>('disconnected');
  const connected = ref(false);
  const metadata = ref<ConnMetadata | null>(null);
  const error = ref<MtwError | null>(null);
  const reconnectAttempt = ref(0);

  const channels = new Map<string, MtwChannel>();
  const agents = new Map<string, MtwAgentClient>();
  let unsubs: Array<() => void> = [];

  function cleanup() {
    unsubs.forEach((u) => u());
    unsubs = [];
  }

  async function connect(opts?: ConnectOptions): Promise<ConnMetadata> {
    cleanup();

    const connectOpts = opts ?? options ?? { url: 'ws://localhost:7741/ws' };
    const conn = new MtwConnection(connectOpts);
    connection.value = conn;

    unsubs.push(
      conn.on('stateChange', (s) => {
        state.value = s;
        connected.value = s === 'connected';
      }),
    );

    unsubs.push(
      conn.on('connected', (meta) => {
        metadata.value = meta;
        error.value = null;
        reconnectAttempt.value = 0;
      }),
    );

    unsubs.push(
      conn.on('reconnecting', (attempt) => {
        reconnectAttempt.value = attempt;
      }),
    );

    unsubs.push(
      conn.on('reconnected', (meta) => {
        metadata.value = meta;
        error.value = null;
        reconnectAttempt.value = 0;
      }),
    );

    unsubs.push(
      conn.on('error', (err) => {
        error.value = err;
      }),
    );

    const meta = await conn.connect();
    return meta;
  }

  async function disconnect(): Promise<void> {
    channels.forEach((ch) => ch.unsubscribe().catch(() => {}));
    channels.clear();
    agents.clear();

    if (connection.value) {
      await connection.value.close();
    }
    cleanup();
    connection.value = null;
    state.value = 'disconnected';
    connected.value = false;
    metadata.value = null;
  }

  async function reconnect(): Promise<void> {
    if (connection.value) {
      await connection.value.close();
      await connection.value.connect();
    }
  }

  function getChannel(name: string): MtwChannel {
    if (channels.has(name)) {
      return channels.get(name)!;
    }
    if (!connection.value) {
      throw new Error('Not connected');
    }
    const ch = new MtwChannel(toRaw(connection.value) as MtwConnection, name);
    channels.set(name, ch);
    return ch;
  }

  function getAgent(name: string): MtwAgentClient {
    if (agents.has(name)) {
      return agents.get(name)!;
    }
    if (!connection.value) {
      throw new Error('Not connected');
    }
    const agent = new MtwAgentClient(toRaw(connection.value) as MtwConnection, name);
    agents.set(name, agent);
    return agent;
  }

  const ctx: MtwContext = {
    connection: readonly(connection) as Ref<MtwConnection | null>,
    state: readonly(state) as Ref<ConnectionState>,
    connected: readonly(connected) as Ref<boolean>,
    metadata: readonly(metadata) as Ref<ConnMetadata | null>,
    error: readonly(error) as Ref<MtwError | null>,
    reconnectAttempt: readonly(reconnectAttempt) as Ref<number>,
    connect,
    disconnect,
    reconnect,
    getChannel,
    getAgent,
  };

  provide(MTW_INJECTION_KEY, ctx);

  // Auto-connect on mount
  if (options && (options as { autoConnect?: boolean }).autoConnect !== false) {
    onMounted(() => {
      connect().catch((err) => {
        error.value = err;
      });
    });
  }

  onUnmounted(() => {
    disconnect().catch(() => {});
  });

  return ctx;
}

/**
 * Access the mtwRequest connection from any descendant component.
 *
 * Usage:
 *   const { connected, state, metadata } = useMtw();
 */
export function useMtw(): MtwContext {
  return useMtwInject();
}
