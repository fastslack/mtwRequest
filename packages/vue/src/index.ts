// =============================================================================
// @mtw/vue — Vue composables for mtwRequest
// =============================================================================

export { useMtwProvider, useMtw, useMtwInject, MTW_INJECTION_KEY } from './composables/useMtw';
export type { MtwContext } from './composables/useMtw';

export { useChannel } from './composables/useChannel';
export type { UseChannelOptions, UseChannelReturn } from './composables/useChannel';

export { useAgent } from './composables/useAgent';
export type { UseAgentOptions, UseAgentReturn, AgentMessage } from './composables/useAgent';

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
} from '@matware/mtw-request-ts-client';
