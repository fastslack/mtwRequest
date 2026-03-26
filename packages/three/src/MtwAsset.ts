// =============================================================================
// @mtw/three — MtwAsset
// =============================================================================
//
// Real-time asset streaming for Three.js. Enables streaming large assets
// (textures, models, audio) over mtwRequest binary channels with progress
// tracking and caching.
// =============================================================================

import type { MtwConnection } from '@mtw/client';
import {
  MtwChannel,
  type MtwMessage,
  type Unsubscribe,
  MtwError,
  createMessage,
  jsonPayload,
  binaryPayload,
} from '@mtw/client';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Asset metadata. */
export interface AssetMetadata {
  /** Unique asset ID. */
  id: string;
  /** Asset name (e.g. "terrain-texture.png"). */
  name: string;
  /** MIME type. */
  mimeType: string;
  /** Total size in bytes. */
  totalSize: number;
  /** Number of chunks this asset is split into. */
  totalChunks: number;
  /** Additional metadata. */
  properties?: Record<string, unknown>;
}

/** A single chunk of asset data. */
export interface AssetChunk {
  /** Asset ID this chunk belongs to. */
  assetId: string;
  /** Chunk index (0-based). */
  index: number;
  /** Base64-encoded binary data. */
  data: string;
  /** Byte size of this chunk. */
  size: number;
}

/** Asset download progress. */
export interface AssetProgress {
  assetId: string;
  receivedChunks: number;
  totalChunks: number;
  receivedBytes: number;
  totalBytes: number;
  /** Progress as 0-1. */
  progress: number;
}

/** Asset loading state. */
export type AssetState = 'pending' | 'loading' | 'complete' | 'error';

/** Options for MtwAsset. */
export interface MtwAssetOptions {
  /** Channel name for asset streaming (default: "assets"). */
  channel?: string;
  /** Chunk size in bytes (default: 64KB). */
  chunkSize?: number;
  /** Whether to cache completed assets in memory (default: true). */
  cache?: boolean;
  /** Maximum cache size in bytes (default: 100MB). */
  maxCacheSize?: number;
}

// ---------------------------------------------------------------------------
// MtwAsset
// ---------------------------------------------------------------------------

/**
 * MtwAsset provides real-time asset streaming over mtwRequest channels.
 *
 * Assets are split into chunks and streamed over a dedicated channel.
 * Multiple assets can be streamed concurrently. The receiver tracks
 * progress and reassembles the complete asset when all chunks arrive.
 *
 * Usage:
 *   import { MtwAsset } from '@mtw/three';
 *
 *   const assetManager = new MtwAsset(connection);
 *   await assetManager.connect();
 *
 *   // Request an asset
 *   const blob = await assetManager.load('texture-id-123');
 *   const texture = new THREE.TextureLoader().load(URL.createObjectURL(blob));
 *
 *   // Stream an asset to other clients
 *   const file = await fetch('/models/character.glb').then(r => r.arrayBuffer());
 *   await assetManager.publish('character-model', file, {
 *     name: 'character.glb',
 *     mimeType: 'model/gltf-binary',
 *   });
 *
 *   // Track progress
 *   assetManager.onProgress((progress) => {
 *     console.log(`${progress.assetId}: ${(progress.progress * 100).toFixed(0)}%`);
 *   });
 */
export class MtwAsset {
  private channel: MtwChannel;
  private options: Required<MtwAssetOptions>;
  private connected = false;
  private unsubs: Unsubscribe[] = [];

  // Asset state tracking
  private inProgress = new Map<
    string,
    {
      metadata: AssetMetadata;
      chunks: Map<number, Uint8Array>;
      receivedBytes: number;
      state: AssetState;
      resolve?: (blob: Blob) => void;
      reject?: (err: Error) => void;
    }
  >();

  // Cache
  private cache = new Map<string, Blob>();
  private cacheSize = 0;

  // Event handlers
  private progressHandlers = new Set<(progress: AssetProgress) => void>();
  private completeHandlers = new Set<(assetId: string, blob: Blob) => void>();
  private errorHandlers = new Set<(assetId: string, error: Error) => void>();

  constructor(
    private readonly connection: MtwConnection,
    options: MtwAssetOptions = {},
  ) {
    this.options = {
      channel: options.channel ?? 'assets',
      chunkSize: options.chunkSize ?? 64 * 1024,
      cache: options.cache ?? true,
      maxCacheSize: options.maxCacheSize ?? 100 * 1024 * 1024,
    };

    this.channel = new MtwChannel(connection, this.options.channel);
  }

  /** Whether the asset channel is connected. */
  get isConnected(): boolean {
    return this.connected;
  }

  /** Number of assets currently being downloaded. */
  get activeDownloads(): number {
    return Array.from(this.inProgress.values()).filter((a) => a.state === 'loading').length;
  }

  /** Current cache size in bytes. */
  get currentCacheSize(): number {
    return this.cacheSize;
  }

  // -----------------------------------------------------------------------
  // Lifecycle
  // -----------------------------------------------------------------------

  /** Connect to the asset channel. */
  async connect(): Promise<void> {
    if (this.connected) return;

    this.unsubs.push(
      this.channel.onMessage((msg) => {
        this.handleIncoming(msg);
      }),
    );

    await this.channel.subscribe();
    this.connected = true;
  }

  /** Disconnect from the asset channel. */
  async disconnect(): Promise<void> {
    this.unsubs.forEach((u) => u());
    this.unsubs = [];

    await this.channel.unsubscribe();
    this.connected = false;

    // Reject any pending downloads
    for (const [id, state] of this.inProgress) {
      if (state.reject) {
        state.reject(new Error('Asset channel disconnected'));
      }
    }
    this.inProgress.clear();
  }

  // -----------------------------------------------------------------------
  // Loading
  // -----------------------------------------------------------------------

  /**
   * Request and download an asset by ID.
   * Returns a Blob containing the complete asset data.
   */
  async load(assetId: string): Promise<Blob> {
    // Check cache first
    if (this.options.cache && this.cache.has(assetId)) {
      return this.cache.get(assetId)!;
    }

    // Check if already in progress
    const existing = this.inProgress.get(assetId);
    if (existing && existing.state === 'loading') {
      return new Promise<Blob>((resolve, reject) => {
        existing.resolve = resolve;
        existing.reject = reject;
      });
    }

    // Send load request
    return new Promise<Blob>((resolve, reject) => {
      this.inProgress.set(assetId, {
        metadata: {
          id: assetId,
          name: '',
          mimeType: 'application/octet-stream',
          totalSize: 0,
          totalChunks: 0,
        },
        chunks: new Map(),
        receivedBytes: 0,
        state: 'pending',
        resolve,
        reject,
      });

      // Request the asset from the server/other clients
      const requestMsg = createMessage(
        'request',
        jsonPayload({ action: 'load', assetId }),
        { channel: this.options.channel },
      );
      this.connection.send(requestMsg);
    });
  }

  // -----------------------------------------------------------------------
  // Publishing
  // -----------------------------------------------------------------------

  /**
   * Publish an asset to all clients on the channel.
   *
   * The data is split into chunks and streamed over the channel.
   */
  async publish(
    assetId: string,
    data: ArrayBuffer,
    info: { name: string; mimeType: string; properties?: Record<string, unknown> },
  ): Promise<void> {
    const totalSize = data.byteLength;
    const totalChunks = Math.ceil(totalSize / this.options.chunkSize);

    // Send metadata first
    const metadata: AssetMetadata = {
      id: assetId,
      name: info.name,
      mimeType: info.mimeType,
      totalSize,
      totalChunks,
      properties: info.properties,
    };

    this.channel.publish({
      type: 'asset_metadata',
      metadata,
    });

    // Send chunks
    const bytes = new Uint8Array(data);
    for (let i = 0; i < totalChunks; i++) {
      const start = i * this.options.chunkSize;
      const end = Math.min(start + this.options.chunkSize, totalSize);
      const chunkData = bytes.slice(start, end);

      // Convert to base64
      const base64 = btoa(
        Array.from(chunkData)
          .map((b) => String.fromCharCode(b))
          .join(''),
      );

      const chunk: AssetChunk = {
        assetId,
        index: i,
        data: base64,
        size: chunkData.length,
      };

      this.channel.publish({
        type: 'asset_chunk',
        chunk,
      });

      // Yield to event loop between chunks to avoid blocking
      if (i % 10 === 9) {
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    }
  }

  // -----------------------------------------------------------------------
  // Events
  // -----------------------------------------------------------------------

  /** Track download progress. */
  onProgress(handler: (progress: AssetProgress) => void): Unsubscribe {
    this.progressHandlers.add(handler);
    return () => this.progressHandlers.delete(handler);
  }

  /** Called when an asset download completes. */
  onComplete(handler: (assetId: string, blob: Blob) => void): Unsubscribe {
    this.completeHandlers.add(handler);
    return () => this.completeHandlers.delete(handler);
  }

  /** Called on download error. */
  onError(handler: (assetId: string, error: Error) => void): Unsubscribe {
    this.errorHandlers.add(handler);
    return () => this.errorHandlers.delete(handler);
  }

  /** Clear the asset cache. */
  clearCache(): void {
    this.cache.clear();
    this.cacheSize = 0;
  }

  // -----------------------------------------------------------------------
  // Internal
  // -----------------------------------------------------------------------

  private handleIncoming(msg: MtwMessage): void {
    if (msg.payload.kind !== 'Json') return;
    const data = msg.payload.data as Record<string, unknown>;

    if (data.type === 'asset_metadata') {
      this.handleMetadata(data.metadata as AssetMetadata);
    } else if (data.type === 'asset_chunk') {
      this.handleChunk(data.chunk as AssetChunk);
    }
  }

  private handleMetadata(metadata: AssetMetadata): void {
    const existing = this.inProgress.get(metadata.id);
    if (existing) {
      existing.metadata = metadata;
      existing.state = 'loading';
    } else {
      this.inProgress.set(metadata.id, {
        metadata,
        chunks: new Map(),
        receivedBytes: 0,
        state: 'loading',
      });
    }
  }

  private handleChunk(chunk: AssetChunk): void {
    const state = this.inProgress.get(chunk.assetId);
    if (!state) return;

    // Decode base64 to binary
    const binary = atob(chunk.data);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }

    state.chunks.set(chunk.index, bytes);
    state.receivedBytes += chunk.size;

    // Emit progress
    const progress: AssetProgress = {
      assetId: chunk.assetId,
      receivedChunks: state.chunks.size,
      totalChunks: state.metadata.totalChunks,
      receivedBytes: state.receivedBytes,
      totalBytes: state.metadata.totalSize,
      progress: state.metadata.totalChunks > 0
        ? state.chunks.size / state.metadata.totalChunks
        : 0,
    };
    this.progressHandlers.forEach((h) => h(progress));

    // Check if complete
    if (state.chunks.size >= state.metadata.totalChunks && state.metadata.totalChunks > 0) {
      this.assembleAsset(chunk.assetId, state);
    }
  }

  private assembleAsset(
    assetId: string,
    state: {
      metadata: AssetMetadata;
      chunks: Map<number, Uint8Array>;
      receivedBytes: number;
      state: AssetState;
      resolve?: (blob: Blob) => void;
      reject?: (err: Error) => void;
    },
  ): void {
    try {
      // Reassemble chunks in order
      const totalChunks = state.metadata.totalChunks;
      const allChunks: Uint8Array[] = [];

      for (let i = 0; i < totalChunks; i++) {
        const chunk = state.chunks.get(i);
        if (!chunk) {
          throw new Error(`Missing chunk ${i} for asset ${assetId}`);
        }
        allChunks.push(chunk);
      }

      // Concatenate into single buffer
      const totalLength = allChunks.reduce((sum, c) => sum + c.length, 0);
      const combined = new Uint8Array(totalLength);
      let offset = 0;
      for (const chunk of allChunks) {
        combined.set(chunk, offset);
        offset += chunk.length;
      }

      const blob = new Blob([combined], { type: state.metadata.mimeType });

      // Cache
      if (this.options.cache) {
        this.addToCache(assetId, blob);
      }

      state.state = 'complete';
      state.resolve?.(blob);

      this.completeHandlers.forEach((h) => h(assetId, blob));
      this.inProgress.delete(assetId);
    } catch (err) {
      state.state = 'error';
      const error = err instanceof Error ? err : new Error(String(err));
      state.reject?.(error);
      this.errorHandlers.forEach((h) => h(assetId, error));
      this.inProgress.delete(assetId);
    }
  }

  private addToCache(assetId: string, blob: Blob): void {
    // Evict old entries if cache is full
    while (this.cacheSize + blob.size > this.options.maxCacheSize && this.cache.size > 0) {
      const [oldestId] = this.cache.keys();
      const oldest = this.cache.get(oldestId);
      if (oldest) {
        this.cacheSize -= oldest.size;
        this.cache.delete(oldestId);
      }
    }

    this.cache.set(assetId, blob);
    this.cacheSize += blob.size;
  }
}
