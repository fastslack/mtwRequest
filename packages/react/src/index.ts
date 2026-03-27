// =============================================================================
// @mtw/react — React hooks and components for mtwRequest
// =============================================================================

export { MtwProvider, useMtwContext } from './MtwProvider';
export type { MtwProviderProps, MtwContextValue } from './MtwProvider';

export { useMtw } from './useMtw';
export type { UseMtwReturn } from './useMtw';

export { useChannel } from './useChannel';
export type { UseChannelOptions, UseChannelReturn } from './useChannel';

export { useAgent } from './useAgent';
export type { UseAgentOptions, UseAgentReturn, AgentMessage } from './useAgent';

export { useStream } from './useStream';
export type { UseStreamOptions, UseStreamReturn } from './useStream';

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
