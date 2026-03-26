// =============================================================================
// @mtw/react — MtwProvider
// =============================================================================
//
// React context provider that manages the mtwRequest connection lifecycle
// and makes it available to all child components via hooks.
// =============================================================================

import React, {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  useCallback,
  type ReactNode,
} from 'react';
import {
  MtwConnection,
  MtwChannel,
  MtwAgentClient,
  type ConnectOptions,
  type ConnectionState,
  type ConnMetadata,
  type DisconnectInfo,
  type MtwError,
} from '@mtw/client';

// ---------------------------------------------------------------------------
// Context types
// ---------------------------------------------------------------------------

export interface MtwContextValue {
  /** The underlying MtwConnection instance. */
  connection: MtwConnection | null;
  /** Current connection state. */
  state: ConnectionState;
  /** Whether the client is connected. */
  connected: boolean;
  /** Server-assigned connection metadata. */
  metadata: ConnMetadata | null;
  /** Connection error, if any. */
  error: MtwError | null;
  /** Current reconnect attempt number (0 when connected). */
  reconnectAttempt: number;
  /** Manually trigger a reconnection. */
  reconnect: () => Promise<void>;
  /** Disconnect from the server. */
  disconnect: () => Promise<void>;
  /** Get or create a channel subscription. */
  getChannel: (name: string) => MtwChannel;
  /** Get or create an agent client. */
  getAgent: (name: string) => MtwAgentClient;
}

const MtwContext = createContext<MtwContextValue | null>(null);

// ---------------------------------------------------------------------------
// useMtwContext — internal hook to access context
// ---------------------------------------------------------------------------

export function useMtwContext(): MtwContextValue {
  const ctx = useContext(MtwContext);
  if (!ctx) {
    throw new Error(
      'useMtw() / useChannel() / useAgent() must be used within an <MtwProvider>',
    );
  }
  return ctx;
}

// ---------------------------------------------------------------------------
// MtwProvider props
// ---------------------------------------------------------------------------

export interface MtwProviderProps {
  /** Connection options (url, auth, reconnect settings, etc.) */
  options?: ConnectOptions;
  /** Shorthand for options.url */
  url?: string;
  /** Shorthand for options.auth */
  auth?: ConnectOptions['auth'];
  /** Whether to connect immediately on mount (default: true) */
  autoConnect?: boolean;
  /** Called when the connection is established. */
  onConnected?: (metadata: ConnMetadata) => void;
  /** Called when disconnected. */
  onDisconnected?: (info: DisconnectInfo) => void;
  /** Called on connection error. */
  onError?: (error: MtwError) => void;
  /** Child components. */
  children: ReactNode;
}

// ---------------------------------------------------------------------------
// MtwProvider component
// ---------------------------------------------------------------------------

export function MtwProvider({
  options,
  url,
  auth,
  autoConnect = true,
  onConnected,
  onDisconnected,
  onError,
  children,
}: MtwProviderProps) {
  const [state, setState] = useState<ConnectionState>('disconnected');
  const [metadata, setMetadata] = useState<ConnMetadata | null>(null);
  const [error, setError] = useState<MtwError | null>(null);
  const [reconnectAttempt, setReconnectAttempt] = useState(0);

  const connectionRef = useRef<MtwConnection | null>(null);
  const channelsRef = useRef(new Map<string, MtwChannel>());
  const agentsRef = useRef(new Map<string, MtwAgentClient>());

  // Stable refs for callbacks
  const onConnectedRef = useRef(onConnected);
  const onDisconnectedRef = useRef(onDisconnected);
  const onErrorRef = useRef(onError);
  onConnectedRef.current = onConnected;
  onDisconnectedRef.current = onDisconnected;
  onErrorRef.current = onError;

  // Build connect options
  const connectOptions: ConnectOptions = options ?? {
    url: url ?? 'ws://localhost:8080/ws',
    auth,
  };
  const connectOptionsRef = useRef(connectOptions);
  connectOptionsRef.current = connectOptions;

  // Initialize connection
  useEffect(() => {
    const conn = new MtwConnection(connectOptionsRef.current);
    connectionRef.current = conn;

    // Wire up event handlers
    const unsubs = [
      conn.on('stateChange', (newState) => {
        setState(newState);
      }),
      conn.on('connected', (meta) => {
        setMetadata(meta);
        setError(null);
        setReconnectAttempt(0);
        onConnectedRef.current?.(meta);
      }),
      conn.on('disconnected', (info) => {
        onDisconnectedRef.current?.(info);
      }),
      conn.on('reconnecting', (attempt) => {
        setReconnectAttempt(attempt);
      }),
      conn.on('reconnected', (meta) => {
        setMetadata(meta);
        setError(null);
        setReconnectAttempt(0);
        onConnectedRef.current?.(meta);
      }),
      conn.on('error', (err) => {
        setError(err);
        onErrorRef.current?.(err);
      }),
    ];

    // Auto-connect
    if (autoConnect) {
      conn.connect().catch((err) => {
        setError(err);
        onErrorRef.current?.(err);
      });
    }

    return () => {
      unsubs.forEach((unsub) => unsub());
      // Clean up channels
      channelsRef.current.forEach((ch) => ch.unsubscribe().catch(() => {}));
      channelsRef.current.clear();
      agentsRef.current.clear();
      conn.close().catch(() => {});
      connectionRef.current = null;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const reconnect = useCallback(async () => {
    const conn = connectionRef.current;
    if (conn) {
      await conn.close();
      await conn.connect();
    }
  }, []);

  const disconnect = useCallback(async () => {
    const conn = connectionRef.current;
    if (conn) {
      await conn.close();
    }
  }, []);

  const getChannel = useCallback((name: string): MtwChannel => {
    if (channelsRef.current.has(name)) {
      return channelsRef.current.get(name)!;
    }
    const conn = connectionRef.current;
    if (!conn) {
      throw new Error('Connection not initialized');
    }
    const ch = new MtwChannel(conn, name);
    channelsRef.current.set(name, ch);
    return ch;
  }, []);

  const getAgent = useCallback((name: string): MtwAgentClient => {
    if (agentsRef.current.has(name)) {
      return agentsRef.current.get(name)!;
    }
    const conn = connectionRef.current;
    if (!conn) {
      throw new Error('Connection not initialized');
    }
    const agent = new MtwAgentClient(conn, name);
    agentsRef.current.set(name, agent);
    return agent;
  }, []);

  const contextValue: MtwContextValue = {
    connection: connectionRef.current,
    state,
    connected: state === 'connected',
    metadata,
    error,
    reconnectAttempt,
    reconnect,
    disconnect,
    getChannel,
    getAgent,
  };

  return React.createElement(MtwContext.Provider, { value: contextValue }, children);
}
