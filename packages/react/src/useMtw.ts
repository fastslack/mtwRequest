// =============================================================================
// @mtw/react — useMtw hook
// =============================================================================
//
// Hook for accessing the mtwRequest connection state and methods.
// =============================================================================

import { useMtwContext } from './MtwProvider';
import type { MtwContextValue } from './MtwProvider';

export interface UseMtwReturn {
  /** Current connection state ('connecting' | 'connected' | 'disconnected' | ...) */
  state: MtwContextValue['state'];
  /** Whether the client is connected. */
  connected: boolean;
  /** Server-assigned connection metadata. */
  metadata: MtwContextValue['metadata'];
  /** Last connection error, if any. */
  error: MtwContextValue['error'];
  /** Current reconnect attempt number (0 when connected). */
  reconnectAttempt: number;
  /** The underlying MtwConnection instance. */
  connection: MtwContextValue['connection'];
  /** Manually trigger a reconnection. */
  reconnect: () => Promise<void>;
  /** Disconnect from the server. */
  disconnect: () => Promise<void>;
}

/**
 * Hook for accessing the mtwRequest connection.
 *
 * Must be used within an <MtwProvider>.
 *
 * Usage:
 *   function StatusBar() {
 *     const { connected, state, metadata, error } = useMtw();
 *
 *     if (!connected) return <div>Connecting...</div>;
 *     return <div>Connected as {metadata?.conn_id}</div>;
 *   }
 */
export function useMtw(): UseMtwReturn {
  const ctx = useMtwContext();

  return {
    state: ctx.state,
    connected: ctx.connected,
    metadata: ctx.metadata,
    error: ctx.error,
    reconnectAttempt: ctx.reconnectAttempt,
    connection: ctx.connection,
    reconnect: ctx.reconnect,
    disconnect: ctx.disconnect,
  };
}
