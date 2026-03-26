// =============================================================================
// @mtw/three — MtwScene
// =============================================================================
//
// Synchronizes Three.js scene state over a binary mtwRequest channel.
// Enables real-time collaborative 3D environments where multiple clients
// see the same scene state.
// =============================================================================

import type { MtwConnection } from '@mtw/client';
import {
  MtwChannel,
  type MtwMessage,
  type Unsubscribe,
  MtwError,
  createMessage,
  jsonPayload,
} from '@mtw/client';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Serialized representation of a Three.js object's transform. */
export interface ObjectTransform {
  position: [number, number, number];
  rotation: [number, number, number, number]; // quaternion: x, y, z, w
  scale: [number, number, number];
}

/** A synchronized scene object descriptor. */
export interface SyncedObject {
  /** Unique object ID (shared across clients). */
  id: string;
  /** Object type (e.g. "Mesh", "Light", "Camera"). */
  type: string;
  /** Transform data. */
  transform: ObjectTransform;
  /** Optional material/geometry properties. */
  properties?: Record<string, unknown>;
  /** Last update timestamp. */
  updatedAt: number;
  /** ID of the client that owns this object. */
  ownerId?: string;
}

/** Scene state update — sent over the wire. */
export interface SceneUpdate {
  /** Type of update. */
  action: 'add' | 'update' | 'remove' | 'snapshot';
  /** The objects involved. */
  objects: SyncedObject[];
  /** Timestamp. */
  timestamp: number;
  /** Sender connection ID. */
  senderId?: string;
}

/** Options for MtwScene. */
export interface MtwSceneOptions {
  /** Channel name for scene sync (default: "3d-sync"). */
  channel?: string;
  /** How often to send transform updates in ms (default: 50 = 20Hz). */
  syncRate?: number;
  /** Whether to interpolate remote object transforms (default: true). */
  interpolate?: boolean;
  /** Interpolation factor (0-1, default: 0.3). */
  interpolationFactor?: number;
  /** Whether this client can add/remove objects (default: true). */
  canEdit?: boolean;
}

/** Callback for scene events. */
export type SceneEventHandler = (update: SceneUpdate) => void;

// ---------------------------------------------------------------------------
// MtwScene
// ---------------------------------------------------------------------------

/**
 * MtwScene synchronizes Three.js scene state over a real-time binary channel.
 *
 * It maintains a shared object registry and broadcasts transform updates
 * to all connected clients. Objects can be added, updated, or removed by
 * any client with edit permissions.
 *
 * Usage:
 *   import { MtwScene } from '@mtw/three';
 *   import * as THREE from 'three';
 *
 *   const scene = new THREE.Scene();
 *   const mtwScene = new MtwScene(connection, { channel: '3d-sync' });
 *
 *   await mtwScene.join();
 *
 *   // Register a mesh for synchronization
 *   const cube = new THREE.Mesh(geometry, material);
 *   scene.add(cube);
 *   mtwScene.register('cube-1', cube);
 *
 *   // In your render loop:
 *   function animate() {
 *     mtwScene.tick(); // sends pending updates
 *     renderer.render(scene, camera);
 *     requestAnimationFrame(animate);
 *   }
 *
 *   // Listen for remote objects
 *   mtwScene.onObjectAdded((update) => {
 *     // Create Three.js objects for remote additions
 *   });
 */
export class MtwScene {
  private channel: MtwChannel;
  private options: Required<MtwSceneOptions>;
  private objects = new Map<string, SyncedObject>();
  private dirtyObjects = new Set<string>();
  private localRefs = new Map<string, { position: { x: number; y: number; z: number }; quaternion: { x: number; y: number; z: number; w: number }; scale: { x: number; y: number; z: number } }>();
  private lastSyncTime = 0;
  private joined = false;
  private unsubs: Unsubscribe[] = [];

  // Event handlers
  private addHandlers = new Set<SceneEventHandler>();
  private updateHandlers = new Set<SceneEventHandler>();
  private removeHandlers = new Set<SceneEventHandler>();

  constructor(
    private readonly connection: MtwConnection,
    options: MtwSceneOptions = {},
  ) {
    this.options = {
      channel: options.channel ?? '3d-sync',
      syncRate: options.syncRate ?? 50,
      interpolate: options.interpolate ?? true,
      interpolationFactor: options.interpolationFactor ?? 0.3,
      canEdit: options.canEdit ?? true,
    };

    this.channel = new MtwChannel(connection, this.options.channel);
  }

  /** Whether this scene is joined and syncing. */
  get isJoined(): boolean {
    return this.joined;
  }

  /** Number of synced objects. */
  get objectCount(): number {
    return this.objects.size;
  }

  /** Get all synced objects. */
  get allObjects(): SyncedObject[] {
    return Array.from(this.objects.values());
  }

  // -----------------------------------------------------------------------
  // Lifecycle
  // -----------------------------------------------------------------------

  /**
   * Join the scene channel and start receiving updates.
   * Requests the current scene snapshot from the server.
   */
  async join(): Promise<void> {
    if (this.joined) return;

    // Subscribe to the channel
    this.unsubs.push(
      this.channel.onMessage((msg) => {
        this.handleIncoming(msg);
      }),
    );

    await this.channel.subscribe();
    this.joined = true;

    // Request snapshot from server/other clients
    const snapshotRequest = createMessage('request', jsonPayload({ action: 'snapshot' }), {
      channel: this.options.channel,
    });
    try {
      this.connection.send(snapshotRequest);
    } catch {
      // May fail if no other clients to respond
    }
  }

  /**
   * Leave the scene channel and stop syncing.
   */
  async leave(): Promise<void> {
    if (!this.joined) return;

    this.unsubs.forEach((u) => u());
    this.unsubs = [];

    await this.channel.unsubscribe();
    this.joined = false;
    this.objects.clear();
    this.dirtyObjects.clear();
    this.localRefs.clear();
  }

  // -----------------------------------------------------------------------
  // Object management
  // -----------------------------------------------------------------------

  /**
   * Register a Three.js object for synchronization.
   *
   * The object must have position, quaternion, and scale properties
   * (like any THREE.Object3D).
   */
  register(
    id: string,
    threeObject: { position: { x: number; y: number; z: number }; quaternion: { x: number; y: number; z: number; w: number }; scale: { x: number; y: number; z: number }; type?: string },
    properties?: Record<string, unknown>,
  ): void {
    this.localRefs.set(id, threeObject);

    const synced: SyncedObject = {
      id,
      type: threeObject.type ?? 'Object3D',
      transform: {
        position: [threeObject.position.x, threeObject.position.y, threeObject.position.z],
        rotation: [threeObject.quaternion.x, threeObject.quaternion.y, threeObject.quaternion.z, threeObject.quaternion.w],
        scale: [threeObject.scale.x, threeObject.scale.y, threeObject.scale.z],
      },
      properties,
      updatedAt: Date.now(),
      ownerId: this.connection.connectionId ?? undefined,
    };

    this.objects.set(id, synced);
    this.dirtyObjects.add(id);

    // Broadcast addition
    if (this.joined) {
      this.broadcastUpdate({
        action: 'add',
        objects: [synced],
        timestamp: Date.now(),
        senderId: this.connection.connectionId ?? undefined,
      });
    }
  }

  /**
   * Unregister an object from synchronization and notify other clients.
   */
  unregister(id: string): void {
    const obj = this.objects.get(id);
    if (!obj) return;

    this.objects.delete(id);
    this.localRefs.delete(id);
    this.dirtyObjects.delete(id);

    if (this.joined) {
      this.broadcastUpdate({
        action: 'remove',
        objects: [obj],
        timestamp: Date.now(),
        senderId: this.connection.connectionId ?? undefined,
      });
    }
  }

  /**
   * Get a synced object by ID.
   */
  getObject(id: string): SyncedObject | undefined {
    return this.objects.get(id);
  }

  // -----------------------------------------------------------------------
  // Tick — call this in your render loop
  // -----------------------------------------------------------------------

  /**
   * Tick the scene sync. Call this in your requestAnimationFrame loop.
   *
   * This:
   * 1. Reads transforms from local Three.js objects
   * 2. Detects changes (dirty objects)
   * 3. Batches and sends updates at the configured sync rate
   */
  tick(): void {
    const now = Date.now();

    // Read local transforms and detect changes
    for (const [id, ref] of this.localRefs) {
      const obj = this.objects.get(id);
      if (!obj) continue;

      const newTransform: ObjectTransform = {
        position: [ref.position.x, ref.position.y, ref.position.z],
        rotation: [ref.quaternion.x, ref.quaternion.y, ref.quaternion.z, ref.quaternion.w],
        scale: [ref.scale.x, ref.scale.y, ref.scale.z],
      };

      // Check if transform changed
      const t = obj.transform;
      if (
        t.position[0] !== newTransform.position[0] ||
        t.position[1] !== newTransform.position[1] ||
        t.position[2] !== newTransform.position[2] ||
        t.rotation[0] !== newTransform.rotation[0] ||
        t.rotation[1] !== newTransform.rotation[1] ||
        t.rotation[2] !== newTransform.rotation[2] ||
        t.rotation[3] !== newTransform.rotation[3] ||
        t.scale[0] !== newTransform.scale[0] ||
        t.scale[1] !== newTransform.scale[1] ||
        t.scale[2] !== newTransform.scale[2]
      ) {
        obj.transform = newTransform;
        obj.updatedAt = now;
        this.dirtyObjects.add(id);
      }
    }

    // Send batch update at configured rate
    if (this.dirtyObjects.size > 0 && now - this.lastSyncTime >= this.options.syncRate) {
      const dirtyObjs = Array.from(this.dirtyObjects)
        .map((id) => this.objects.get(id))
        .filter((o): o is SyncedObject => o !== undefined);

      if (dirtyObjs.length > 0) {
        this.broadcastUpdate({
          action: 'update',
          objects: dirtyObjs,
          timestamp: now,
          senderId: this.connection.connectionId ?? undefined,
        });
      }

      this.dirtyObjects.clear();
      this.lastSyncTime = now;
    }
  }

  // -----------------------------------------------------------------------
  // Event handlers
  // -----------------------------------------------------------------------

  /** Called when a remote client adds an object. */
  onObjectAdded(handler: SceneEventHandler): Unsubscribe {
    this.addHandlers.add(handler);
    return () => this.addHandlers.delete(handler);
  }

  /** Called when a remote client updates an object. */
  onObjectUpdated(handler: SceneEventHandler): Unsubscribe {
    this.updateHandlers.add(handler);
    return () => this.updateHandlers.delete(handler);
  }

  /** Called when a remote client removes an object. */
  onObjectRemoved(handler: SceneEventHandler): Unsubscribe {
    this.removeHandlers.add(handler);
    return () => this.removeHandlers.delete(handler);
  }

  // -----------------------------------------------------------------------
  // Internal
  // -----------------------------------------------------------------------

  private broadcastUpdate(update: SceneUpdate): void {
    try {
      this.channel.publish(update as unknown as Record<string, unknown>);
    } catch {
      // Ignore send errors
    }
  }

  private handleIncoming(msg: MtwMessage): void {
    if (msg.payload.kind !== 'Json') return;

    const update = msg.payload.data as SceneUpdate;
    if (!update || !update.action) return;

    // Ignore our own updates
    const myId = this.connection.connectionId;
    if (update.senderId && update.senderId === myId) return;

    switch (update.action) {
      case 'add':
        for (const obj of update.objects) {
          this.objects.set(obj.id, obj);
        }
        this.addHandlers.forEach((h) => h(update));
        break;

      case 'update':
        for (const obj of update.objects) {
          // Only update if we don't own this object locally
          if (!this.localRefs.has(obj.id)) {
            this.objects.set(obj.id, obj);
          }
        }
        this.updateHandlers.forEach((h) => h(update));
        break;

      case 'remove':
        for (const obj of update.objects) {
          if (!this.localRefs.has(obj.id)) {
            this.objects.delete(obj.id);
          }
        }
        this.removeHandlers.forEach((h) => h(update));
        break;

      case 'snapshot':
        // Full scene state — replace remote objects
        for (const obj of update.objects) {
          if (!this.localRefs.has(obj.id)) {
            this.objects.set(obj.id, obj);
          }
        }
        this.addHandlers.forEach((h) => h(update));
        break;
    }
  }
}
