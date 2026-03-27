// =============================================================================
// @matware/mtw-request-svelte — Svelte integration for mtwRequest
// =============================================================================

// ── Quick API (recommended) ──────────────────────────────────
// Connect once, use channels/agents as reactive stores everywhere.
//
//   import { mtw, channel, agent } from '@matware/mtw-request-svelte';
//
//   await mtw.connect({ url: 'ws://localhost:7741/ws' });
//   const dashboard = channel('dashboard');
//   const ai = agent('assistant');

export { mtw, channel, agent } from './stores/mtw';

// ── Advanced API ─────────────────────────────────────────────
// Fine-grained stores for complex use cases.

export { createConnectionStore, isConnected } from './stores/connection';
export type { ConnectionStoreState, ConnectionStore } from './stores/connection';

export { createChannelStore } from './stores/channel';
export type { ChannelStoreState, ChannelStore } from './stores/channel';

export { createAgentStore } from './stores/agent';
export type {
  AgentStoreState,
  AgentStore,
  AgentMessage,
  CreateAgentStoreOptions,
} from './stores/agent';

// ── Re-exported types from client ────────────────────────────
export type {
  MtwMessage,
  MsgType,
  Payload,
  ConnectOptions,
  AuthOptions,
  ConnectionState,
  ConnMetadata,
  AgentChunk,
  AgentToolCall,
  AgentResponse,
  AgentOptions,
  ChannelMember,
  ToolHandler,
} from '@matware/mtw-request-ts-client';

export { MtwError } from '@matware/mtw-request-ts-client';
