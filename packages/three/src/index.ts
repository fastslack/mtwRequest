// =============================================================================
// @mtw/three — Three.js real-time sync for mtwRequest
// =============================================================================

export { MtwScene } from './MtwScene';
export type {
  MtwSceneOptions,
  SyncedObject,
  ObjectTransform,
  SceneUpdate,
  SceneEventHandler,
} from './MtwScene';

export { MtwAsset } from './MtwAsset';
export type {
  MtwAssetOptions,
  AssetMetadata,
  AssetChunk,
  AssetProgress,
  AssetState,
} from './MtwAsset';

// Re-export connection types for convenience
export type {
  MtwMessage,
  ConnectOptions,
  AuthOptions,
  MtwError,
} from '@mtw/client';
