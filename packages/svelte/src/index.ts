// =============================================================================
// @mtw/svelte — Svelte stores for mtwRequest
// =============================================================================

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

// Re-export commonly used types from @mtw/client
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
  MtwError,
  ToolHandler,
} from '@mtw/client';
