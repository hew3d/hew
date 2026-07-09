/**
 * SceneRenderer — manages THREE.js meshes for live scene geometry.
 *
 * After any committed kernel mutation, call `refresh()` to:
 *   1. Pull object_ids() and object_mesh(id) for each object
 *   2. Rebuild the viewport meshes (flat-shaded faces + edge LineSegments)
 *   3. Dispose old GPU buffers
 *
 * Component placements draw as GPU-instanced batches: one THREE.InstancedMesh
 * (+ one instanced edge LineSegments) per (definition member, side bucket), so
 * draw calls scale with distinct members × material groups instead of with
 * placements. Per-instance state (selection color, isolation lighting,
 * transform preview) is handled by MATERIALIZING the affected placement out of
 * its batch into the classic per-instance Group path — see `_syncMaterialized`.
 *
 * Also manages sketch geometry for every document sketch (refreshAllSketches).
 */

import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { makeFatSegments, disposeFatSegments } from './fatLine'
import { srgbColorsToLinear } from './colorSpace'

/** Default neutral face color (matches DEFAULT_MATERIAL_RGBA in tessellate). */
const FACE_COLOR_DEFAULT = 0xcccccc
/** Edge color — dark for readability. */
const EDGE_COLOR = 0x1a1a1a
/** Orange highlight for selected object edges (kept for selection; face fill uses material color). */
const EDGE_COLOR_SELECTED = 0xffaa00
const SKETCH_LINE_COLOR = 0x2266cc
/** Sketch line width in px (matches the `makeFatSegments` call in `refreshAllSketches`). */
const SKETCH_LINE_WIDTH_PX = 2.2
/** Selected-sketch highlight width — deliberately bolder than the base
 * sketch-line width above so the orange overlay reads as clearly on top of
 * it rather than disappearing into (or under) it, worst case in light mode. */
const SKETCH_HIGHLIGHT_WIDTH_PX = 3.8
const SKETCH_REGION_COLOR = 0x88aadd
/** Normal translucency of a sketch region fill. */
const SKETCH_REGION_OPACITY = 0.4
/** Opacity of entities faded out by the active editing context (isolation). */
const DIMMED_OPACITY = 0.15
/** Muted grey for construction guides — distinct from edges/axes/sketch lines. */
const GUIDE_COLOR = 0x555555
/** Half-length of a rendered line guide (meters) — long enough to read as "infinite" at person scale. */
const GUIDE_LINE_HALF_LENGTH = 50
/** Half-size of a point guide's cross marker (meters). */
const GUIDE_POINT_MARKER_HALF_SIZE = 0.05
/**
 * Guide-line dash/gap size as a fraction of the camera-to-target distance.
 * For a perspective camera the on-screen size of a world length L is ∝ L /
 * distance, so scaling the dash with distance holds the dash pattern roughly
 * constant in pixels regardless of zoom (screen-constant, like the cursor).
 */
const GUIDE_DASH_SCREEN_K = 0.01

/**
 * Touched-entity hints for a targeted (incremental) refresh — see
 * `SceneRenderer.refreshTouched`. All fields optional; ids the renderer does
 * not currently draw are ignored, and creations/deletions the caller cannot
 * enumerate (e.g. a through-cut splitting a solid in two) are still caught by
 * an id-set diff against the wasm scene.
 */
export interface RefreshTouched {
  /** Touched object ids — world objects AND definition members (a def-member
   * id invalidates its shared geometry cache and rebuilds its instanced
   * batches, i.e. every placement). */
  objectIds?: bigint[]
  /** Touched instance ids (e.g. a transformed placement). */
  instanceIds?: bigint[]
  /** Touched component definition ids — invalidates every member's shared
   * geometry and rebuilds all placements of the definition. */
  componentIds?: bigint[]
}

/** Disposable group for one object's faces + edges */
interface ObjectMeshGroup {
  objectId: bigint
  facesMesh: THREE.Mesh
  edgesLines: THREE.LineSegments
  group: THREE.Group
}

/**
 * One MATERIALIZED instance: a THREE.Group holding per-instance meshes for
 * each member object at the instance pose. Only placements that need
 * per-instance state (selection color, isolation lighting, transform preview)
 * are materialized; everything else renders through `MemberBatch`.
 */
interface InstanceMeshGroup {
  instanceId: bigint
  /** Set of definition member object ids rendered inside this group */
  memberIds: bigint[]
  group: THREE.Group
  /** Face meshes, one per member, in the same order as memberIds */
  facesMeshes: THREE.Mesh[]
  edgesLines: THREE.LineSegments[]
}

/** Shared per-member typed arrays pulled once across the wasm boundary. */
interface MemberGeometry {
  positions: Float32Array
  normals: Float32Array
  indices: Uint32Array
  colors: Float32Array
  uvs: Float32Array
  groupMaterialIds: BigUint64Array
  groupStarts: Uint32Array
  groupCounts: Uint32Array
  edgePositions: Float32Array
  /** Whether the member object encloses a volume — drives double-sided
   * rendering for open (non-watertight) shells (see `_bucketTag`). */
  watertight: boolean
}

/**
 * Side bucket for one instanced batch: 'F' = watertight, normal pose
 * (FrontSide); 'B' = watertight, reflected pose (det < 0 flips winding, so
 * BackSide); 'D' = non-watertight member (DoubleSide — culling off, so normal
 * and reflected placements share one bucket).
 */
type BucketTag = 'F' | 'B' | 'D'

/**
 * One GPU batch: every non-materialized placement of one definition member
 * sharing a side bucket. Faces are a single THREE.InstancedMesh carrying the
 * member's material array + geometry groups (one instanced draw per material
 * group); edges are one LineSegments over an InstancedBufferGeometry whose
 * per-instance 3×4 pose rows (imRow0..2) are consumed by a patched
 * LineBasicMaterial (three.js has no instanced LineSegments).
 */
interface MemberBatch {
  memberId: bigint
  side: THREE.Side
  mesh: THREE.InstancedMesh
  edges: THREE.LineSegments
  /** Per-instance pose rows for the edge shader, one vec4 per row. */
  edgeRows: [THREE.InstancedBufferAttribute, THREE.InstancedBufferAttribute, THREE.InstancedBufferAttribute]
  /** Slot ownership: slot i belongs to instance slots[i]. A hidden or
   * materialized instance keeps its slot (written degenerate — zero linear
   * part draws nothing) so restoring it is a matrix write, not a rebuild. */
  slots: bigint[]
  slotOf: Map<bigint, number>
  /** Slots currently written degenerate (hidden/materialized). Bounds
   * computation skips these — a suppressed placement must not contribute
   * even its translation point to zoom-extents or culling volumes. */
  suppressedSlots: Set<number>
}

/** Renderer-side book-keeping for one placement, batched or materialized. */
interface InstanceRecord {
  instanceId: bigint
  componentId: bigint
  memberIds: bigint[]
  /** Full 4×4 pose built from the kernel's row-major 3×4. */
  matrix: THREE.Matrix4
  /** det(linear part) < 0 — reflected placements land in the 'B' bucket. */
  reflected: boolean
}

/** Scratch matrix for slot writes (module-level to avoid per-write alloc). */
const _slotMatrix = new THREE.Matrix4()
/** Scratch bounds for the live-slots-only batch bounds computation. */
const _boundsBox = new THREE.Box3()
const _boundsSphere = new THREE.Sphere()

export class SceneRenderer {
  private scene: THREE.Scene
  private wasmScene: WasmScene

  /** Parent group for all object geometry */
  readonly objectsGroup: THREE.Group
  /** Parent group for sketch lines/regions */
  readonly sketchGroup: THREE.Group
  /** Parent group for all instance geometry */
  readonly instancesGroup: THREE.Group
  /** Parent group for construction guide geometry */
  readonly guidesGroup: THREE.Group

  private objectGroups: Map<bigint, ObjectMeshGroup> = new Map()
  /** MATERIALIZED instances only (selected / isolation-lit / transform-preview
   * placements pulled out of their batch), keyed by instance id. All the
   * per-instance mutation paths (_applyInstanceColors, _setInstanceOpacity,
   * getInstanceGroup) operate on this map and therefore only ever touch
   * materialized placements. */
  private instanceGroups: Map<bigint, InstanceMeshGroup> = new Map()
  /** Every rendered placement, batched or materialized, keyed by instance id. */
  private instanceRecords: Map<bigint, InstanceRecord> = new Map()
  /** Instanced batches, keyed `${memberId}|${BucketTag}`. Batch geometry and
   * materials are owned per batch and disposed on rebuild/removal. */
  private batches: Map<string, MemberBatch> = new Map()
  /** Instances materialized on demand by `getInstanceGroup` (transform
   * preview). Cleared on the next refresh/refreshTouched — the commit that
   * follows a preview — so they restore to their batch. */
  private previewMaterialized: Set<bigint> = new Set()
  /**
   * Shared typed-array cache for definition member objects.
   * Keyed by member object id; the raw arrays are shared across every consumer
   * (batches, materialized groups), but each consumer creates its own
   * BufferAttribute wrappers (so that geometry.dispose() on one does not delete
   * GPU buffers shared with another — three.js ties GPU buffer lifetime to the
   * BufferAttribute, not to the underlying TypedArray).
   * Invalidated when a component definition is edited (refreshInstances re-builds it).
   */
  private memberGeometryCache: Map<bigint, MemberGeometry> = new Map()

  /**
   * THREE.Texture cache, keyed by material id (as string). Built once per id
   * and shared across instances so we never duplicate GPU texture objects.
   */
  private textureCache: Map<string, THREE.Texture> = new Map()
  private sketchLines: LineSegments2 | null = null
  /** One fill mesh per sketch region, keyed by `${sketchHandle}:${regionHandle}`
   *  (region handles are per-sketch, so they can collide across sketches). */
  private sketchRegionMeshes: Map<string, THREE.Mesh> = new Map()
  /** Merged LineSegments for every dashed line guide. */
  private guideLines: THREE.LineSegments | null = null
  /** Last dash size applied to `guideLines` (screen-constant); -1 = none yet. */
  private lastGuideDashSize = -1
  /** Merged LineSegments for every point guide's cross marker. */
  private guideMarkers: THREE.LineSegments | null = null
  /** The currently selected guide, drawn as a bright overlay. */
  private selectedGuideId: bigint | null = null
  /** Highlight overlay (solid bright line/cross) for `selectedGuideId`. */
  private guideHighlight: THREE.LineSegments | null = null
  /** Currently selected sketch ids, drawn as a bright overlay. */
  private selectedSketchIds: bigint[] = []
  /**
   * Highlight overlay (solid bright lines) for `selectedSketchIds`. A fat
   * `LineSegments2` (not a plain `THREE.LineSegments`) — normal sketch lines
   * are already fat (`sketchLines`, ~2.2px via `makeFatSegments`), and WebGL
   * ignores `linewidth` on a plain `LineBasicMaterial` (always renders 1px),
   * so the old plain-line highlight was thinner than the lines it was meant
   * to highlight and read as nearly invisible, worst in light mode
   *.
   */
  private sketchHighlight: LineSegments2 | null = null
  /** Last known watertight state per object */
  private watertightMap: Map<bigint, boolean> = new Map()
  /** Currently selected object ids (ordered; may include non-object entities,
   *  which simply match no group). */
  private selectedObjectIds: bigint[] = []
  /** Currently selected instance ids */
  private selectedInstanceIds: bigint[] = []
  /**
   * Active lit set for isolation: null = top level (nothing dimmed);
   * Set<bigint> = the leaf object ids that stay fully lit; all others dim.
   * Sketches dim whenever the lit set is not null.
   */
  private activeLitSet: Set<bigint> | null = null
  /**
   * Active lit instance set for isolation: null = top level (no isolation);
   * Set<bigint> = instance ids that stay fully lit; all others dim.
   */
  private activeLitInstanceSet: Set<bigint> | null = null
  /** Object ids that are hidden (group.visible = false). Session-only. */
  private hiddenObjectIds: Set<bigint> = new Set()
  /** Instance ids that are hidden (group.visible = false). Session-only. */
  private hiddenInstanceIds: Set<bigint> = new Set()

  constructor(threeScene: THREE.Scene, wasmScene: WasmScene) {
    this.scene = threeScene
    this.wasmScene = wasmScene

    this.objectsGroup = new THREE.Group()
    this.objectsGroup.name = 'Objects'
    threeScene.add(this.objectsGroup)

    this.instancesGroup = new THREE.Group()
    this.instancesGroup.name = 'Instances'
    threeScene.add(this.instancesGroup)

    this.sketchGroup = new THREE.Group()
    this.sketchGroup.name = 'Sketch'
    threeScene.add(this.sketchGroup)

    this.guidesGroup = new THREE.Group()
    this.guidesGroup.name = 'Guides'
    threeScene.add(this.guidesGroup)
  }

  /** Rebuild all object geometry from the WASM scene. Returns watertight map. */
  refresh(): Map<bigint, boolean> {
    const ids = this.wasmScene.object_ids()

    // Determine which object IDs are new, existing, or removed
    const newIds = new Set<bigint>(ids)
    const existingIds = new Set(this.objectGroups.keys())

    // Remove stale objects
    for (const oldId of existingIds) {
      if (!newIds.has(oldId)) {
        this._removeObjectGroup(oldId)
      }
    }

    // Add or update each object
    for (let i = 0; i < ids.length; i++) {
      const id = ids[i]
      this._refreshObject(id)
    }

    // Also rebuild instance geometry (component defs share BufferGeometry
    // from the member cache, so we invalidate and rebuild after any mutation).
    this.refreshInstances()

    // Rebuilt objects start opaque; re-apply the isolation fade.
    this._applyIsolation()
    // Re-apply hidden visibility (groups are rebuilt by refresh).
    this._applyHidden()

    return new Map(this.watertightMap)
  }

  /**
   * Targeted (incremental) refresh: rebuild only the geometry a mutation
   * actually touched, leaving every other group's GPU buffers alone. On a
   * large document (hundreds of objects, thousands of instances) the full
   * `refresh()` re-clones every mesh across the wasm boundary and re-uploads
   * every GPU buffer for ANY document change — this path makes a single-object
   * edit O(touched) instead.
   *
   * `touched` carries the ids the caller knows about (see `RefreshTouched`).
   * Creations and deletions the caller cannot enumerate (a through-cut
   * yielding two objects, an Option-copy, a boolean consuming its operands)
   * are caught by diffing live wasm ids against the rendered groups — new ids
   * are built, stale ids removed, and only touched survivors are rebuilt.
   *
   * A touched **definition member** id (or a touched component id) drops the
   * shared member-geometry cache entry and rebuilds that member's instanced
   * batches (plus any materialized placement drawing it), so def edits
   * (push_pull_in_component, painting instanced geometry) propagate to all
   * instances — mirroring how the kernel's own reconcile lands every
   * placement in `instances_touched` on a def edit. A touched **instance** id
   * with an unchanged bucket is the fast path: an in-place matrix write into
   * its batch slots, no geometry re-pull, no GPU buffer rebuild.
   *
   * Callers with no touched-set (load/import/undo/redo/structural ops) must
   * keep using the full `refresh()`.
   *
   * Returns the (full) watertight map, like `refresh()`.
   */
  refreshTouched(touched: RefreshTouched): Map<bigint, boolean> {
    const touchedObjects = touched.objectIds ?? []
    const touchedInstances = touched.instanceIds ?? []
    const touchedComponents = touched.componentIds ?? []

    // ---- objects: diff live ids against rendered groups.
    const liveIds = new Set<bigint>(this.wasmScene.object_ids())
    for (const oldId of [...this.objectGroups.keys()]) {
      if (!liveIds.has(oldId)) {
        this._removeObjectGroup(oldId)
      }
    }
    const rebuiltObjects = new Set<bigint>()
    for (const id of liveIds) {
      if (!this.objectGroups.has(id)) {
        this._refreshObject(id)
        rebuiltObjects.add(id)
      }
    }
    for (const id of touchedObjects) {
      if (liveIds.has(id) && !rebuiltObjects.has(id)) {
        this._refreshObject(id)
        rebuiltObjects.add(id)
      }
    }

    // ---- shared definition geometry: a touched id that is cached as a def
    // member (or any member of a touched component) is stale — drop it so the
    // batch rebuilds below re-pull it from the kernel.
    const dirtyMembers = new Set<bigint>()
    for (const id of touchedObjects) {
      if (this.memberGeometryCache.has(id)) {
        this.memberGeometryCache.delete(id)
        dirtyMembers.add(id)
      }
    }
    for (const cid of touchedComponents) {
      for (const mid of this.wasmScene.component_member_objects(cid)) {
        this.memberGeometryCache.delete(mid)
        dirtyMembers.add(mid)
      }
    }

    // A commit ends any transform preview — restore preview-only
    // materializations to their batch (in _syncMaterialized below).
    this.previewMaterialized.clear()

    // ---- instances: id-set diff, then per-touched-instance pose fast path.
    // Batch membership changes (instance added/removed, def/member change,
    // reflectedness flip, dirty member geometry) rebuild the affected member
    // batches; a plain pose change is an in-place matrix write.
    const liveInstanceIds = new Set<bigint>(this.wasmScene.instance_ids())
    const membersToRebuild = new Set<bigint>(dirtyMembers)
    for (const oldId of [...this.instanceRecords.keys()]) {
      if (!liveInstanceIds.has(oldId)) {
        const rec = this.instanceRecords.get(oldId)
        if (rec !== undefined) {
          for (const m of rec.memberIds) membersToRebuild.add(m)
        }
        this._removeInstance(oldId)
      }
    }
    const addedInstances = new Set<bigint>()
    for (const iid of liveInstanceIds) {
      if (!this.instanceRecords.has(iid)) {
        const rec = this._pullInstanceRecord(iid)
        if (rec !== undefined) {
          for (const m of rec.memberIds) membersToRebuild.add(m)
        }
        addedInstances.add(iid)
      }
    }
    for (const iid of touchedInstances) {
      if (addedInstances.has(iid)) continue
      const prev = this.instanceRecords.get(iid)
      if (prev === undefined) continue
      const next = this._pullInstanceRecord(iid)
      if (next === undefined) {
        for (const m of prev.memberIds) membersToRebuild.add(m)
        this._removeInstance(iid)
        continue
      }
      const sameMembers =
        next.componentId === prev.componentId &&
        next.memberIds.length === prev.memberIds.length &&
        next.memberIds.every((m, i) => m === prev.memberIds[i])
      if (!sameMembers || next.reflected !== prev.reflected) {
        // Bucket membership changed — the slow (rebuild) path.
        for (const m of prev.memberIds) membersToRebuild.add(m)
        for (const m of next.memberIds) membersToRebuild.add(m)
        continue
      }
      // Fast path: same buckets, new pose — write the slot matrices in place.
      for (const m of next.memberIds) {
        if (membersToRebuild.has(m)) continue // rebuilt below anyway
        const key = this._batchKeyFor(m, next)
        const batch = key !== undefined ? this.batches.get(key) : undefined
        const slot = batch?.slotOf.get(iid)
        if (batch !== undefined && slot !== undefined) {
          this._writeSlot(batch, slot, next)
        }
      }
      // A materialized placement follows the new pose too.
      const g = this.instanceGroups.get(iid)
      if (g !== undefined) {
        g.group.matrix.copy(next.matrix)
        g.group.matrixWorldNeedsUpdate = true
      }
    }
    for (const m of membersToRebuild) {
      this._rebuildMemberBatches(m)
    }
    // Materialized placements drawing a rebuilt member re-pull from the
    // (refilled) shared cache — mirrors the kernel landing every placement in
    // instances_touched on a def edit.
    if (membersToRebuild.size > 0) {
      for (const iid of [...this.instanceGroups.keys()]) {
        const rec = this.instanceRecords.get(iid)
        if (rec !== undefined && rec.memberIds.some((m) => membersToRebuild.has(m))) {
          this._materialize(iid)
        }
      }
    }
    this._syncMaterialized()

    // Rebuilt groups start opaque/visible; re-apply isolation + hidden state
    // (cheap CPU-side property writes — no wasm calls, no GPU uploads).
    this._applyIsolation()
    this._applyHidden()

    return new Map(this.watertightMap)
  }

  /**
   * Rebuild all instance batches from instance_ids(). Call after any mutation
   * that may add/remove instances or change definition geometry.
   *
   * Invalidates the member geometry cache so shared member arrays are
   * re-pulled from the kernel (which has invalidated its tessellation cache on
   * mutation), then re-builds every (member, bucket) batch from scratch and
   * re-materializes the placements that need per-instance state.
   */
  refreshInstances(): void {
    // Invalidate the shared member geometry cache — definition may have changed.
    this.memberGeometryCache.clear()
    this._disposeAllBatches()
    for (const id of [...this.instanceGroups.keys()]) {
      this._disposeMaterializedGroup(id)
    }
    this.instanceRecords.clear()
    // A full rebuild only happens on commit points — any preview is over.
    this.previewMaterialized.clear()

    const instanceIds = this.wasmScene.instance_ids()
    const members = new Set<bigint>()
    for (let i = 0; i < instanceIds.length; i++) {
      const rec = this._pullInstanceRecord(instanceIds[i])
      if (rec !== undefined) {
        for (const m of rec.memberIds) members.add(m)
      }
    }
    for (const m of members) {
      this._rebuildMemberBatches(m)
    }

    // Selected / isolation-lit placements come back out of their batches.
    this._syncMaterialized()
    this._applyInstanceIsolation()
    // Re-apply hidden visibility for materialized groups (batched placements
    // read hiddenInstanceIds during the slot writes above).
    for (const [id, g] of this.instanceGroups) {
      g.group.visible = !this.hiddenInstanceIds.has(id)
    }
  }

  /**
   * Compute the 3×3 determinant of the linear part of a row-major 3×4 pose.
   * Used to detect reflected poses (det < 0) which need winding reversal.
   */
  private _poseDet(pose: Float64Array): number {
    const a = pose[0], b = pose[1], c = pose[2]
    const d = pose[4], e = pose[5], f = pose[6]
    const g = pose[8], h = pose[9], i = pose[10]
    return a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
  }

  /** Pull one placement's def/pose/members into its InstanceRecord (creating
   * or overwriting it). Returns undefined (and leaves no record) when the
   * kernel no longer knows the instance. */
  private _pullInstanceRecord(instanceId: bigint): InstanceRecord | undefined {
    const componentId = this.wasmScene.instance_def(instanceId)
    if (componentId === undefined) return undefined
    const pose = this.wasmScene.instance_pose(instanceId)
    if (pose === undefined) return undefined
    const memberIds = Array.from(this.wasmScene.component_member_objects(componentId))
    // THREE Matrix4 is column-major; set() takes row-major.
    const matrix = new THREE.Matrix4().set(
      pose[0], pose[1], pose[2],  pose[3],
      pose[4], pose[5], pose[6],  pose[7],
      pose[8], pose[9], pose[10], pose[11],
      0,       0,       0,        1,
    )
    const rec: InstanceRecord = {
      instanceId,
      componentId,
      memberIds,
      matrix,
      // Reflected pose: flips face winding (watertight members only — an open
      // shell renders double-sided regardless of reflection).
      reflected: this._poseDet(pose) < 0,
    }
    this.instanceRecords.set(instanceId, rec)
    return rec
  }

  /** Fetch (and cache) one member's typed arrays — raw data only, not
   * BufferAttributes, to avoid GPU buffer aliasing on dispose. */
  private _getMemberGeometry(memberId: bigint): MemberGeometry {
    let cached = this.memberGeometryCache.get(memberId)
    if (cached === undefined) {
      const mesh = this.wasmScene.object_mesh(memberId)
      try {
        cached = {
          positions: mesh.positions(),
          normals: mesh.normals(),
          indices: mesh.indices(),
          // Baked vertex colors are sRGB; convert to linear so the
          // theme-aware light rig lights them correctly (as the object path
          // does), shared across every instanced placement of this member.
          colors: srgbColorsToLinear(mesh.colors()),
          uvs: mesh.uvs(),
          groupMaterialIds: BigUint64Array.from(mesh.group_material_ids()),
          groupStarts: mesh.group_starts(),
          groupCounts: mesh.group_counts(),
          edgePositions: mesh.edge_positions(),
          watertight: mesh.watertight(),
        }
        this.memberGeometryCache.set(memberId, cached)
      } finally {
        mesh.free()
      }
    }
    return cached
  }

  /** Bucket tag for one (member, placement) pair — see `BucketTag`. */
  private _bucketTag(watertight: boolean, reflected: boolean): BucketTag {
    return watertight ? (reflected ? 'B' : 'F') : 'D'
  }

  /** Batch key for one member as rendered by one placement, or undefined when
   * the member's geometry has not been pulled (no batch can exist either). */
  private _batchKeyFor(memberId: bigint, rec: InstanceRecord): string | undefined {
    const cached = this.memberGeometryCache.get(memberId)
    if (cached === undefined) return undefined
    return `${memberId}|${this._bucketTag(cached.watertight, rec.reflected)}`
  }

  /** Dispose and rebuild every batch of one definition member from the current
   * instance records (grouped by side bucket). */
  private _rebuildMemberBatches(memberId: bigint): void {
    for (const [key, batch] of [...this.batches]) {
      if (batch.memberId === memberId) this._disposeBatch(key)
    }
    const placements: InstanceRecord[] = []
    for (const rec of this.instanceRecords.values()) {
      if (rec.memberIds.includes(memberId)) placements.push(rec)
    }
    if (placements.length === 0) return

    const cached = this._getMemberGeometry(memberId)
    const byTag = new Map<BucketTag, InstanceRecord[]>()
    for (const rec of placements) {
      const tag = this._bucketTag(cached.watertight, rec.reflected)
      const list = byTag.get(tag)
      if (list === undefined) byTag.set(tag, [rec])
      else list.push(rec)
    }
    for (const [tag, recs] of byTag) {
      this._buildBatch(memberId, tag, recs, cached)
    }
  }

  /** Build one (member, bucket) batch: an InstancedMesh for faces and an
   * instanced-edge LineSegments, with one slot per placement. */
  private _buildBatch(memberId: bigint, tag: BucketTag, recs: InstanceRecord[], cached: MemberGeometry): void {
    const side = tag === 'F' ? THREE.FrontSide : tag === 'B' ? THREE.BackSide : THREE.DoubleSide

    // Fresh BufferAttribute wrappers over the shared TypedArrays — the batch
    // owns these GPU buffers, so disposing it cannot free another batch's.
    const faceGeo = new THREE.BufferGeometry()
    faceGeo.setAttribute('position', new THREE.BufferAttribute(cached.positions, 3))
    faceGeo.setAttribute('normal', new THREE.BufferAttribute(cached.normals, 3))
    faceGeo.setAttribute('color', new THREE.BufferAttribute(cached.colors, 3))
    faceGeo.setAttribute('uv', new THREE.BufferAttribute(cached.uvs, 2))
    faceGeo.setIndex(new THREE.BufferAttribute(cached.indices, 1))

    const materials = this._buildMaterialArray(
      cached.groupMaterialIds,
      cached.groupStarts,
      cached.groupCounts,
      faceGeo,
      side,
    )
    const mesh = new THREE.InstancedMesh(faceGeo, materials, recs.length)
    mesh.name = `InstanceBatch_${memberId}_${tag}`
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage)

    const edgeGeo = new THREE.InstancedBufferGeometry()
    edgeGeo.setAttribute('position', new THREE.BufferAttribute(cached.edgePositions, 3))
    edgeGeo.instanceCount = recs.length
    const mkRow = () => {
      const attr = new THREE.InstancedBufferAttribute(new Float32Array(recs.length * 4), 4)
      attr.setUsage(THREE.DynamicDrawUsage)
      return attr
    }
    const edgeRows: MemberBatch['edgeRows'] = [mkRow(), mkRow(), mkRow()]
    edgeGeo.setAttribute('imRow0', edgeRows[0])
    edgeGeo.setAttribute('imRow1', edgeRows[1])
    edgeGeo.setAttribute('imRow2', edgeRows[2])
    const edges = new THREE.LineSegments(edgeGeo, this._makeInstancedEdgeMaterial())
    edges.name = `InstanceBatchEdges_${memberId}_${tag}`
    // The base geometry's bounding sphere ignores the per-instance transforms
    // applied in the shader, so frustum culling would drop visible edges.
    edges.frustumCulled = false
    // The edge positions are the member's DEFINITION-space soup — the real
    // poses live only in the imRow0..2 instanced attributes the vertex shader
    // applies. Anything that reads geometry bounds (Box3.expandByObject in
    // zoom-extents / standard views treats a plain LineSegments as
    // geometry.boundingBox) would therefore frame a phantom region at the
    // definition origin. The sibling face InstancedMesh for this bucket
    // already computes exactly the true instance-aware bounds, so delegate
    // both bounds to it; computing lazily here (instead of copying eagerly on
    // every slot write) keeps pose writes O(1). `_writeSlot` invalidates the
    // mesh and edge bounds together, so every write path (build, pose fast
    // path, materialize/restore, hide/unhide) stays consistent.
    edgeGeo.computeBoundingBox = () => {
      if (mesh.boundingBox === null) mesh.computeBoundingBox()
      edgeGeo.boundingBox = (edgeGeo.boundingBox ?? new THREE.Box3()).copy(
        mesh.boundingBox as THREE.Box3,
      )
    }
    edgeGeo.computeBoundingSphere = () => {
      if (mesh.boundingSphere === null) mesh.computeBoundingSphere()
      edgeGeo.boundingSphere = (edgeGeo.boundingSphere ?? new THREE.Sphere()).copy(
        mesh.boundingSphere as THREE.Sphere,
      )
    }

    const batch: MemberBatch = {
      memberId,
      side,
      mesh,
      edges,
      edgeRows,
      slots: recs.map((r) => r.instanceId),
      slotOf: new Map(recs.map((r, i) => [r.instanceId, i])),
      suppressedSlots: new Set(),
    }
    // Instance-aware bounds over LIVE slots only. three's stock
    // InstancedMesh bounds union every slot, and a degenerate slot still
    // contributes its translation point — a far-away hidden placement (real
    // models carry strays hundreds of metres out) would inflate zoom-extents
    // until the camera re-frames past its own far plane and the viewport
    // blanks. A suppressed placement's true bounds are the materialized
    // Group's (selection/preview) or nothing at all (hidden).
    const geoBounds = { box: null as THREE.Box3 | null, sphere: null as THREE.Sphere | null }
    mesh.computeBoundingBox = () => {
      if (geoBounds.box === null) {
        if (faceGeo.boundingBox === null) faceGeo.computeBoundingBox()
        geoBounds.box = faceGeo.boundingBox as THREE.Box3
      }
      const box = (mesh.boundingBox ??= new THREE.Box3())
      box.makeEmpty()
      for (let i = 0; i < mesh.count; i++) {
        if (batch.suppressedSlots.has(i)) continue
        mesh.getMatrixAt(i, _slotMatrix)
        box.union(_boundsBox.copy(geoBounds.box).applyMatrix4(_slotMatrix))
      }
    }
    mesh.computeBoundingSphere = () => {
      if (geoBounds.sphere === null) {
        if (faceGeo.boundingSphere === null) faceGeo.computeBoundingSphere()
        geoBounds.sphere = faceGeo.boundingSphere as THREE.Sphere
      }
      const sphere = (mesh.boundingSphere ??= new THREE.Sphere())
      sphere.makeEmpty()
      for (let i = 0; i < mesh.count; i++) {
        if (batch.suppressedSlots.has(i)) continue
        mesh.getMatrixAt(i, _slotMatrix)
        sphere.union(_boundsSphere.copy(geoBounds.sphere).applyMatrix4(_slotMatrix))
      }
    }
    recs.forEach((rec, i) => this._writeSlot(batch, i, rec))

    this.instancesGroup.add(mesh)
    this.instancesGroup.add(edges)
    this.batches.set(`${memberId}|${tag}`, batch)
  }

  /**
   * LineBasicMaterial patched (onBeforeCompile) to transform each vertex by a
   * per-instance 3×4 pose carried in the imRow0..2 instanced attributes —
   * three.js has no instanced LineSegments, so the batch edge soup instances
   * in the vertex shader. `customProgramCacheKey` keeps the patched program
   * from colliding with the stock line program in the shader cache.
   */
  private _makeInstancedEdgeMaterial(): THREE.LineBasicMaterial {
    const mat = new THREE.LineBasicMaterial({ color: EDGE_COLOR })
    mat.onBeforeCompile = (shader) => {
      shader.vertexShader = shader.vertexShader
        .replace(
          '#include <common>',
          '#include <common>\nattribute vec4 imRow0;\nattribute vec4 imRow1;\nattribute vec4 imRow2;',
        )
        .replace(
          '#include <begin_vertex>',
          '#include <begin_vertex>\n\ttransformed = vec3( dot( imRow0, vec4( transformed, 1.0 ) ), dot( imRow1, vec4( transformed, 1.0 ) ), dot( imRow2, vec4( transformed, 1.0 ) ) );',
        )
    }
    mat.customProgramCacheKey = () => 'hew_instanced_edges'
    return mat
  }

  /**
   * Write one batch slot from the instance's current state: the live pose when
   * the placement draws batched, or a degenerate matrix (linear part zeroed,
   * translation kept) when hidden or materialized. Zero scale collapses every
   * primitive to a point, which rasterizes nothing; bounds skip suppressed
   * slots entirely (see the overrides in `_buildBatch`), so the kept
   * translation never leaks into zoom-extents or culling volumes.
   */
  private _writeSlot(batch: MemberBatch, slot: number, rec: InstanceRecord): void {
    const suppressed =
      this.hiddenInstanceIds.has(rec.instanceId) || this.instanceGroups.has(rec.instanceId)
    const e = rec.matrix.elements
    if (suppressed) {
      batch.suppressedSlots.add(slot)
      _slotMatrix.set(
        0, 0, 0, e[12],
        0, 0, 0, e[13],
        0, 0, 0, e[14],
        0, 0, 0, 1,
      )
    } else {
      batch.suppressedSlots.delete(slot)
      _slotMatrix.copy(rec.matrix)
    }
    batch.mesh.setMatrixAt(slot, _slotMatrix)
    const s = _slotMatrix.elements
    batch.edgeRows[0].setXYZW(slot, s[0], s[4], s[8], s[12])
    batch.edgeRows[1].setXYZW(slot, s[1], s[5], s[9], s[13])
    batch.edgeRows[2].setXYZW(slot, s[2], s[6], s[10], s[14])
    batch.mesh.instanceMatrix.needsUpdate = true
    for (const row of batch.edgeRows) row.needsUpdate = true
    // Invalidate object-level bounds so frustum culling / zoom-extents
    // recompute over the new instance matrices. The edge geometry's bounds
    // mirror the face mesh's (see the delegation in `_buildBatch`), so the
    // two invalidate together — no write path can leave them stale.
    batch.mesh.boundingBox = null
    batch.mesh.boundingSphere = null
    batch.edges.geometry.boundingBox = null
    batch.edges.geometry.boundingSphere = null
  }

  /** Re-write every batch slot owned by one placement (visibility change,
   * materialize/restore). No-op for slots whose batch is gone (rebuild will
   * re-seed them). */
  private _refreshSlots(instanceId: bigint): void {
    const rec = this.instanceRecords.get(instanceId)
    if (rec === undefined) return
    for (const memberId of rec.memberIds) {
      const key = this._batchKeyFor(memberId, rec)
      const batch = key !== undefined ? this.batches.get(key) : undefined
      const slot = batch?.slotOf.get(instanceId)
      if (batch !== undefined && slot !== undefined) {
        this._writeSlot(batch, slot, rec)
      }
    }
  }

  /**
   * Reconcile which placements are materialized: desired = selected ∪
   * isolation-lit ∪ preview-materialized; everything else renders batched.
   * Restoring drops the classic group and un-degenerates the batch slots;
   * materializing zeroes them (slot ownership is kept either way).
   */
  private _syncMaterialized(): void {
    const desired = new Set<bigint>()
    for (const id of this.selectedInstanceIds) {
      if (this.instanceRecords.has(id)) desired.add(id)
    }
    if (this.activeLitInstanceSet !== null) {
      for (const id of this.activeLitInstanceSet) {
        if (this.instanceRecords.has(id)) desired.add(id)
      }
    }
    for (const id of [...this.previewMaterialized]) {
      if (this.instanceRecords.has(id)) desired.add(id)
      else this.previewMaterialized.delete(id)
    }
    for (const id of [...this.instanceGroups.keys()]) {
      if (!desired.has(id)) this._restoreToBatch(id)
    }
    for (const id of desired) {
      if (!this.instanceGroups.has(id)) this._materialize(id)
    }
  }

  /** Pull one placement out of its batches into the classic per-instance
   * Group path (`Instance_${id}`), re-applying its per-instance state. */
  private _materialize(instanceId: bigint): void {
    const rec = this.instanceRecords.get(instanceId)
    if (rec === undefined) return
    if (this.instanceGroups.has(instanceId)) {
      this._disposeMaterializedGroup(instanceId)
    }

    const group = new THREE.Group()
    group.name = `Instance_${instanceId}`
    group.matrixAutoUpdate = false
    group.matrix.copy(rec.matrix)
    group.matrixWorldNeedsUpdate = true

    const facesMeshes: THREE.Mesh[] = []
    const edgesLinesList: THREE.LineSegments[] = []
    for (const memberId of rec.memberIds) {
      const cached = this._getMemberGeometry(memberId)
      // Open (non-watertight) shells have inward-wound faces on some
      // triangles, so a single-sided material renders them invisible from the
      // "wrong" side; render those double-sided.
      //
      // Watertight members stay FrontSide EVEN FOR A REFLECTED POSE: here the
      // pose rides on `group.matrix`, and WebGLRenderer already reverses the
      // front-face winding for any Mesh whose world matrix has a negative
      // determinant. Adding BackSide on top double-flips and renders the
      // solid inside-out (per-face paint vanishes behind the culled faces).
      // Only the BATCH path needs the explicit 'B' (BackSide) bucket — its
      // per-slot poses live in a shader attribute the renderer's determinant
      // check cannot see (see `_bucketTag`).
      const side = cached.watertight ? THREE.FrontSide : THREE.DoubleSide

      // Face mesh — its own BufferAttribute wrappers over the shared
      // TypedArrays, so geometry.dispose() frees only this group's GPU buffers.
      const faceGeo = new THREE.BufferGeometry()
      faceGeo.setAttribute('position', new THREE.BufferAttribute(cached.positions, 3))
      faceGeo.setAttribute('normal', new THREE.BufferAttribute(cached.normals, 3))
      faceGeo.setAttribute('color', new THREE.BufferAttribute(cached.colors, 3))
      faceGeo.setAttribute('uv', new THREE.BufferAttribute(cached.uvs, 2))
      faceGeo.setIndex(new THREE.BufferAttribute(cached.indices, 1))

      const faceMaterials = this._buildMaterialArray(
        cached.groupMaterialIds,
        cached.groupStarts,
        cached.groupCounts,
        faceGeo,
        side,
      )
      const facesMesh = new THREE.Mesh(faceGeo, faceMaterials)
      facesMesh.name = `InstanceFace_${instanceId}_${memberId}`
      group.add(facesMesh)
      facesMeshes.push(facesMesh)

      const edgeGeo = new THREE.BufferGeometry()
      edgeGeo.setAttribute('position', new THREE.BufferAttribute(cached.edgePositions, 3))
      const edgeMat = new THREE.LineBasicMaterial({ color: EDGE_COLOR })
      const edgesLines = new THREE.LineSegments(edgeGeo, edgeMat)
      edgesLines.name = `InstanceEdge_${instanceId}_${memberId}`
      group.add(edgesLines)
      edgesLinesList.push(edgesLines)
    }

    this.instancesGroup.add(group)
    this.instanceGroups.set(instanceId, {
      instanceId,
      memberIds: rec.memberIds,
      group,
      facesMeshes,
      edgesLines: edgesLinesList,
    })

    // The batch keeps the slot but stops drawing it.
    this._refreshSlots(instanceId)

    // Re-apply the per-instance state that forced materialization.
    if (this.selectedInstanceIds.includes(instanceId)) {
      this._applyInstanceColors(instanceId, true)
    }
    group.visible = !this.hiddenInstanceIds.has(instanceId)
    const dimmed = this.activeLitSet !== null ||
      (this.activeLitInstanceSet !== null && !this.activeLitInstanceSet.has(instanceId))
    const g = this.instanceGroups.get(instanceId)
    if (g !== undefined) {
      this._setInstanceOpacity(g, dimmed ? DIMMED_OPACITY : 1)
    }
  }

  /** Return a materialized placement to its batch slots. */
  private _restoreToBatch(instanceId: bigint): void {
    this._disposeMaterializedGroup(instanceId)
    this._refreshSlots(instanceId)
  }

  /** Dispose one materialized group's GPU wrappers (selection list untouched —
   * restore-to-batch must not clear a live selection). */
  private _disposeMaterializedGroup(instanceId: bigint): void {
    const g = this.instanceGroups.get(instanceId)
    if (g === undefined) return

    for (const mesh of g.facesMeshes) {
      // Material may be a single material or an array (multi-material mesh).
      const mat = mesh.material
      if (Array.isArray(mat)) {
        for (const m of mat) {
          m.dispose()
        }
      } else {
        ;(mat as THREE.Material).dispose()
      }
      // The geometry references shared TypedArrays; dispose frees only the
      // wrappers' GPU buffers.
      mesh.geometry.dispose()
    }
    for (const lines of g.edgesLines) {
      ;(lines.material as THREE.Material).dispose()
      lines.geometry.dispose()
    }

    this.instancesGroup.remove(g.group)
    this.instanceGroups.delete(instanceId)
  }

  /** Drop one placement entirely (kernel no longer has it). The caller
   * rebuilds the member batches that carried its slots. */
  private _removeInstance(instanceId: bigint): void {
    this._disposeMaterializedGroup(instanceId)
    this.instanceRecords.delete(instanceId)
    this.previewMaterialized.delete(instanceId)
    this.selectedInstanceIds = this.selectedInstanceIds.filter((id) => id !== instanceId)
  }

  /** Dispose one batch's GPU resources (geometry, materials, instance
   * attributes). Textures stay in `textureCache`. */
  private _disposeBatch(key: string): void {
    const batch = this.batches.get(key)
    if (batch === undefined) return
    batch.mesh.geometry.dispose()
    const mat = batch.mesh.material
    if (Array.isArray(mat)) {
      for (const m of mat) m.dispose()
    } else {
      ;(mat as THREE.Material).dispose()
    }
    batch.mesh.dispose() // frees the instanceMatrix GPU buffer
    batch.edges.geometry.dispose()
    ;(batch.edges.material as THREE.Material).dispose()
    this.instancesGroup.remove(batch.mesh)
    this.instancesGroup.remove(batch.edges)
    this.batches.delete(key)
  }

  private _disposeAllBatches(): void {
    for (const key of [...this.batches.keys()]) {
      this._disposeBatch(key)
    }
  }

  /**
   * Build the parallel THREE.Material[] for a multi-material mesh.
   *
   * For each group: calls `geometry.addGroup(start, count, i)` and creates a
   * `MeshPhongMaterial` with either a solid `vertexColors` (material id ==
   * `u64::MAX` = default) or the palette color (and an image `map` if the
   * material has a texture, cached by material id). For the default group
   * (id == `u64::MAX`) the material uses `vertexColors: true` so the per-vertex
   * colors from the tessellator drive the shading; all other groups also use
   * `vertexColors: true` since the color is baked in already.
   */
  private _buildMaterialArray(
    groupMaterialIds: BigUint64Array,
    groupStarts: Uint32Array,
    groupCounts: Uint32Array,
    geometry: THREE.BufferGeometry,
    side: THREE.Side = THREE.FrontSide,
  ): THREE.MeshPhongMaterial[] {
    const materials: THREE.MeshPhongMaterial[] = []
    const SENTINEL = BigInt('18446744073709551615') // u64::MAX

    for (let i = 0; i < groupMaterialIds.length; i++) {
      const mid = groupMaterialIds[i]
      geometry.addGroup(groupStarts[i], groupCounts[i], i)

      if (mid === SENTINEL) {
        // Default group — use per-vertex colors from tessellator.
        materials.push(
          new THREE.MeshPhongMaterial({
            vertexColors: true,
            flatShading: true,
            side,
          }),
        )
      } else {
        const midStr = mid.toString()
        const info = this.wasmScene.material_info(mid)
        let tex: THREE.Texture | undefined = undefined
        if (info !== undefined && info.has_texture()) {
          if (!this.textureCache.has(midStr)) {
            const bytes = this.wasmScene.material_texture_bytes(mid)
            if (bytes !== undefined) {
              // Sniff MIME type from magic bytes: JPEG starts with FF, PNG with 89 50.
              const mime = bytes.length >= 2 && bytes[0] === 0xff && bytes[1] === 0xd8
                ? 'image/jpeg'
                : 'image/png'
              const blob = new Blob([new Uint8Array(bytes)], { type: mime })
              const url = URL.createObjectURL(blob)
              const loader = new THREE.TextureLoader()
              // Revoke the object URL once the image has been decoded.
              const t = loader.load(url, () => URL.revokeObjectURL(url))
              // Image textures are sRGB-encoded; without declaring it three
              // samples them as linear and the output re-encode washes them out.
              t.colorSpace = THREE.SRGBColorSpace
              t.wrapS = THREE.RepeatWrapping
              t.wrapT = THREE.RepeatWrapping
              this.textureCache.set(midStr, t)
            }
          }
          tex = this.textureCache.get(midStr)
        }

        // Palette colors are sRGB bytes; setRGB with an explicit color space
        // converts to linear (the Color(r, g, b) constructor would not).
        const color = info !== undefined
          ? new THREE.Color().setRGB(info.r() / 255, info.g() / 255, info.b() / 255, THREE.SRGBColorSpace)
          : new THREE.Color(FACE_COLOR_DEFAULT)

        // Per-material opacity (glass etc.): alpha < 255 → render transparent.
        // Stored as userData.baseOpacity so isolation dimming multiplies into it
        // instead of clobbering it back to opaque.
        const baseOpacity = info !== undefined ? info.a() / 255 : 1
        const m = new THREE.MeshPhongMaterial({
          vertexColors: tex === undefined, // use vertex colors when no texture
          color: tex !== undefined ? color : undefined,
          map: tex,
          flatShading: true,
          side,
          transparent: baseOpacity < 1,
          opacity: baseOpacity,
          depthWrite: baseOpacity >= 1,
        })
        m.userData.baseOpacity = baseOpacity
        materials.push(m)
        info?.free?.()
      }
    }

    // If palette is empty for some reason, fall back to a single default mat.
    if (materials.length === 0) {
      geometry.addGroup(0, Infinity, 0)
      materials.push(
        new THREE.MeshPhongMaterial({
          color: FACE_COLOR_DEFAULT,
          flatShading: true,
          side,
        }),
      )
    }

    return materials
  }

  private _refreshObject(objectId: bigint): void {
    // Always remove the old group and rebuild — tessellation cache means this
    // is cheap (docs/DEVELOPMENT.md B4: cache invalidated on mutation).
    if (this.objectGroups.has(objectId)) {
      this._removeObjectGroup(objectId)
    }

    const mesh = this.wasmScene.object_mesh(objectId)
    try {
      const watertight = mesh.watertight()
      this.watertightMap.set(objectId, watertight)

      const positions = mesh.positions()
      const normals = mesh.normals()
      const indices = mesh.indices()
      const colors = srgbColorsToLinear(mesh.colors())
      const uvs = mesh.uvs()
      const groupMaterialIds = BigUint64Array.from(mesh.group_material_ids())
      const groupStarts = mesh.group_starts()
      const groupCounts = mesh.group_counts()
      const edgePositions = mesh.edge_positions()

      // Face mesh — multi-material: one BufferGeometry with addGroup per material.
      const faceGeo = new THREE.BufferGeometry()
      faceGeo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
      faceGeo.setAttribute('normal', new THREE.BufferAttribute(normals, 3))
      faceGeo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
      faceGeo.setAttribute('uv', new THREE.BufferAttribute(uvs, 2))
      faceGeo.setIndex(new THREE.BufferAttribute(indices, 1))

      // Open (non-watertight) shells have inward-wound faces on some
      // triangles, so a single-sided material renders them invisible from
      // the "wrong" side (looks like an empty wireframe) — render those
      // double-sided. Standalone objects are never reflected (only instance
      // poses can be), so watertight ones keep the plain default FrontSide.
      const side = watertight ? THREE.FrontSide : THREE.DoubleSide
      const faceMaterials = this._buildMaterialArray(
        groupMaterialIds,
        groupStarts,
        groupCounts,
        faceGeo,
        side,
      )
      const facesMesh = new THREE.Mesh(faceGeo, faceMaterials)

      // Edge lines
      const edgeGeo = new THREE.BufferGeometry()
      edgeGeo.setAttribute('position', new THREE.BufferAttribute(edgePositions, 3))
      const edgeMat = new THREE.LineBasicMaterial({ color: EDGE_COLOR })
      const edgesLines = new THREE.LineSegments(edgeGeo, edgeMat)

      const group = new THREE.Group()
      group.name = `Object_${objectId}`
      group.add(facesMesh)
      group.add(edgesLines)

      this.objectsGroup.add(group)
      this.objectGroups.set(objectId, { objectId, facesMesh, edgesLines, group })

      // Re-apply selection highlight if this object is selected
      if (this.selectedObjectIds.includes(objectId)) {
        this._applyObjectColors(objectId, true)
      }
    } finally {
      mesh.free()
    }
  }

  private _removeObjectGroup(objectId: bigint): void {
    const g = this.objectGroups.get(objectId)
    if (g === undefined) return

    g.facesMesh.geometry.dispose()
    const mat = g.facesMesh.material
    if (Array.isArray(mat)) {
      for (const m of mat) {
        m.dispose()
      }
    } else {
      ;(mat as THREE.Material).dispose()
    }
    g.edgesLines.geometry.dispose()
    ;(g.edgesLines.material as THREE.Material).dispose()
    this.objectsGroup.remove(g.group)
    this.objectGroups.delete(objectId)
    this.watertightMap.delete(objectId)
    // If the removed object was selected, drop it from the selection
    this.selectedObjectIds = this.selectedObjectIds.filter((id) => id !== objectId)
  }

  /**
   * Rebuild lines and region fills for EVERY sketch in the document (the
   * document holds many first-class sketches). Call after any sketch mutation,
   * extrusion, or scene undo/redo.
   *
   * All sketches' edges are merged into one LineSegments buffer; each region is
   * a triangle-fan fill keyed by `${sketchHandle}:${regionHandle}`.
   */
  refreshAllSketches(): void {
    this._clearSketchLines()
    this._clearSketchRegions()

    const allLinePositions: number[] = []
    for (const sketchHandle of this.wasmScene.sketch_ids()) {
      const linePositions = this.wasmScene.sketch_lines(sketchHandle)
      for (let i = 0; i < linePositions.length; i++) {
        allLinePositions.push(linePositions[i])
      }
      this._buildRegionFills(sketchHandle)
    }

    if (allLinePositions.length > 0) {
      // Fat lines (LineSegments2) — a plain THREE.LineSegments ignores
      // `linewidth` on WebGL and renders 1px, which read as almost-invisible
      // sketch edges (Refinement pass). `transparent: true` so the
      // sketch-isolation fade can animate opacity. Resolution is kept current
      // by the fat-line registry (makeFatSegments registers the material;
      // Viewport calls updateFatLineResolutions on mount/resize).
      this.sketchLines = makeFatSegments(new Float32Array(allLinePositions), {
        color: SKETCH_LINE_COLOR,
        widthPx: SKETCH_LINE_WIDTH_PX,
        transparent: true,
      })
      this.sketchGroup.add(this.sketchLines)
    }

    // Rebuilt sketch geometry starts at full strength; re-apply the fade.
    this._applySketchIsolation()
    // Re-assert the selection overlay after a rebuild (a deleted selected
    // sketch's highlight drops out; a surviving one's geometry may have moved).
    this._rebuildSketchHighlight()
  }

  /**
   * Build translucent fill meshes for one sketch's still-extrudable regions.
   * For M1 rectangles each boundary is a convex polygon, so a triangle fan from
   * vertex 0 is correct. Consumed regions are absent from `sketch_regions`, so
   * they leave no stray fill.
   */
  private _buildRegionFills(sketchHandle: bigint): void {
    const regionHandles = this.wasmScene.sketch_regions(sketchHandle)
    for (let i = 0; i < regionHandles.length; i++) {
      const regionHandle = regionHandles[i]
      const boundary = this.wasmScene.region_boundary(sketchHandle, regionHandle)
      // boundary is flat [x0,y0,z0, x1,y1,z1, ...]; n vertices
      const n = Math.floor(boundary.length / 3)
      if (n < 3) continue

      const verts3: THREE.Vector3[] = []
      for (let i = 0; i < n; i++) {
        verts3.push(new THREE.Vector3(boundary[i * 3], boundary[i * 3 + 1], boundary[i * 3 + 2]))
      }

      // Robust planar triangulation. The old code fanned from vertex 0, which is
      // only correct for CONVEX polygons — a non-convex region (an L-shaped or
      // freehand polyline sketch) produced overlapping/absent triangles, so its
      // fill "only sometimes" appeared (Refinement pass). Newell's method gives
      // the polygon normal (works on the ground OR on a face), we project into
      // that plane, ear-clip via THREE.ShapeUtils, then emit triangles from the
      // original 3D verts lifted slightly along the normal to avoid z-fighting.
      const normal = new THREE.Vector3()
      for (let i = 0; i < n; i++) {
        const cur = verts3[i]
        const nxt = verts3[(i + 1) % n]
        normal.x += (cur.y - nxt.y) * (cur.z + nxt.z)
        normal.y += (cur.z - nxt.z) * (cur.x + nxt.x)
        normal.z += (cur.x - nxt.x) * (cur.y + nxt.y)
      }
      if (normal.lengthSq() < 1e-12) continue
      normal.normalize()

      const u = new THREE.Vector3()
      if (Math.abs(normal.z) < 0.9) u.set(0, 0, 1).cross(normal).normalize()
      else u.set(1, 0, 0).cross(normal).normalize()
      const v = new THREE.Vector3().crossVectors(normal, u).normalize()

      const pts2 = verts3.map((p) => new THREE.Vector2(p.dot(u), p.dot(v)))
      const tris = THREE.ShapeUtils.triangulateShape(pts2, [])

      const lift = normal.clone().multiplyScalar(0.001)
      const positions: number[] = []
      for (const tri of tris) {
        for (const idx of tri) {
          const p = verts3[idx]
          positions.push(p.x + lift.x, p.y + lift.y, p.z + lift.z)
        }
      }
      if (positions.length === 0) continue

      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3))
      const mat = new THREE.MeshBasicMaterial({
        color: SKETCH_REGION_COLOR,
        transparent: true,
        opacity: SKETCH_REGION_OPACITY,
        side: THREE.DoubleSide,
        depthWrite: false,
      })
      const mesh = new THREE.Mesh(geo, mat)
      this.sketchGroup.add(mesh)
      this.sketchRegionMeshes.set(`${sketchHandle}:${regionHandle}`, mesh)
    }
  }

  /**
   * Rebuild all construction-guide geometry from the WASM scene.
   * Call after any guide mutation (add/delete/delete-all) — mirrors
   * `refreshAllSketches`'s "clear and rebuild from scratch" approach since
   * guide counts are small and this is cheap.
   *
   * Line guides render as a long dashed segment (±[GUIDE_LINE_HALF_LENGTH])
   * through `origin` along `direction`; point guides render as a small
   * dashed-free cross marker centered at `position`. Both use the muted
   * construction color GUIDE_COLOR, distinct from edges/axes/sketch lines.
   */
  refreshGuides(): void {
    this._clearGuides()

    const linePositions: number[] = []
    const markerPositions: number[] = []

    for (const guideId of this.wasmScene.guide_ids()) {
      const kind = this.wasmScene.guide_kind(guideId)
      const geometry = this.wasmScene.guide_geometry(guideId)
      if (kind === undefined || geometry === undefined) continue

      if (kind === 'line') {
        // [ox, oy, oz, dx, dy, dz]
        const [ox, oy, oz, dx, dy, dz] = geometry
        const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
        if (len < 1e-9) continue
        const nx = (dx / len) * GUIDE_LINE_HALF_LENGTH
        const ny = (dy / len) * GUIDE_LINE_HALF_LENGTH
        const nz = (dz / len) * GUIDE_LINE_HALF_LENGTH

        // One solid segment per guide; the dashed look comes from a
        // LineDashedMaterial whose dash/gap sizes are kept screen-constant
        // (updateGuideDashScale) so dashes don't balloon to metres when zoomed
        // out. computeLineDistances() below makes the dashing work.
        linePositions.push(
          ox - nx, oy - ny, oz - nz,
          ox + nx, oy + ny, oz + nz,
        )
      } else if (kind === 'point') {
        // [x, y, z] — a small 3-axis cross marker.
        const [x, y, z] = geometry
        const k = GUIDE_POINT_MARKER_HALF_SIZE
        markerPositions.push(
          x - k, y, z,  x + k, y, z,
          x, y - k, z,  x, y + k, z,
          x, y, z - k,  x, y, z + k,
        )
      }
    }

    if (linePositions.length > 0) {
      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(linePositions), 3))
      // dashSize/gapSize are placeholders — updateGuideDashScale() sets the
      // screen-constant values every frame from the camera distance.
      const mat = new THREE.LineDashedMaterial({ color: GUIDE_COLOR, dashSize: 0.05, gapSize: 0.05 })
      this.guideLines = new THREE.LineSegments(geo, mat)
      this.guideLines.computeLineDistances()
      this.guidesGroup.add(this.guideLines)
    }

    if (markerPositions.length > 0) {
      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(markerPositions), 3))
      const mat = new THREE.LineBasicMaterial({ color: GUIDE_COLOR })
      this.guideMarkers = new THREE.LineSegments(geo, mat)
      this.guidesGroup.add(this.guideMarkers)
    }

    // Re-assert the selection overlay after a rebuild (the selected guide's
    // geometry may have changed; a deleted one drops out).
    this._rebuildGuideHighlight()
  }

  /** Dispose and clear the current guide line/marker geometry, if any. */
  private _clearGuides(): void {
    if (this.guideLines !== null) {
      this.guideLines.geometry.dispose()
      ;(this.guideLines.material as THREE.Material).dispose()
      this.guidesGroup.remove(this.guideLines)
      this.guideLines = null
      this.lastGuideDashSize = -1
    }
    if (this.guideMarkers !== null) {
      this.guideMarkers.geometry.dispose()
      ;(this.guideMarkers.material as THREE.Material).dispose()
      this.guidesGroup.remove(this.guideMarkers)
      this.guideMarkers = null
    }
  }

  /** Mark `id` as the selected guide and redraw its bright overlay;
   * `null` clears it. The caller schedules the render. */
  setSelectedGuide(id: bigint | null): void {
    this.selectedGuideId = id
    this._rebuildGuideHighlight()
  }

  /** (Re)build the bright overlay for the selected guide — a solid line/cross in
   * the selection color, drawn on top so it reads as picked over the dashed
   * grey. Cleared if nothing is selected or the guide no longer exists. */
  private _rebuildGuideHighlight(): void {
    if (this.guideHighlight !== null) {
      this.guideHighlight.geometry.dispose()
      ;(this.guideHighlight.material as THREE.Material).dispose()
      this.guidesGroup.remove(this.guideHighlight)
      this.guideHighlight = null
    }
    if (this.selectedGuideId === null) return
    const kind = this.wasmScene.guide_kind(this.selectedGuideId)
    const geometry = this.wasmScene.guide_geometry(this.selectedGuideId)
    if (kind === undefined || geometry === undefined) return

    const pts: number[] = []
    if (kind === 'line') {
      const [ox, oy, oz, dx, dy, dz] = geometry
      const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
      if (len < 1e-9) return
      const nx = (dx / len) * GUIDE_LINE_HALF_LENGTH
      const ny = (dy / len) * GUIDE_LINE_HALF_LENGTH
      const nz = (dz / len) * GUIDE_LINE_HALF_LENGTH
      pts.push(ox - nx, oy - ny, oz - nz, ox + nx, oy + ny, oz + nz)
    } else if (kind === 'point') {
      const [x, y, z] = geometry
      const k = GUIDE_POINT_MARKER_HALF_SIZE
      pts.push(x - k, y, z, x + k, y, z, x, y - k, z, x, y + k, z, x, y, z - k, x, y, z + k)
    } else {
      return
    }
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
    const mat = new THREE.LineBasicMaterial({ color: EDGE_COLOR_SELECTED, depthTest: false })
    this.guideHighlight = new THREE.LineSegments(geo, mat)
    this.guideHighlight.renderOrder = 999
    this.guidesGroup.add(this.guideHighlight)
  }

  /** Show/hide all construction guides (View ▸ Guides toggle). */
  setGuidesVisible(visible: boolean): void {
    this.guidesGroup.visible = visible
  }

  /**
   * Keep the dashed guide lines at a constant on-screen dash size regardless
   * of zoom (mirrors CueLayer's screen-constant cursor). `cameraDistance` is
   * the orbit camera-to-target distance. Call once per frame from the render
   * loop; cheap and a no-op when there are no line guides.
   */
  updateGuideDashScale(cameraDistance: number): void {
    if (this.guideLines === null) return
    const size = Math.max(cameraDistance, 0.001) * GUIDE_DASH_SCREEN_K
    // Skip sub-1% changes (static camera) so we don't churn the material every
    // frame; on a real change, bump needsUpdate so three.js re-uploads the dash
    // uniforms (a property change alone isn't reliably picked up; the program
    // cache key is unchanged so this does NOT recompile the shader).
    if (this.lastGuideDashSize > 0 && Math.abs(size - this.lastGuideDashSize) < this.lastGuideDashSize * 0.01) {
      return
    }
    this.lastGuideDashSize = size
    const mat = this.guideLines.material as THREE.LineDashedMaterial
    mat.dashSize = size
    mat.gapSize = size
    mat.needsUpdate = true
  }

  /**
   * Highlight exactly the given objects (orange edges, brighter faces) and
   * restore any others to normal colors. Ids that match no object group (e.g. a
   * selected sketch) are simply ignored. Pass `[]` to clear the highlight.
   */
  setSelected(objectIds: bigint[]): void {
    const next = new Set(objectIds)
    // Restore objects that are no longer selected.
    for (const id of this.selectedObjectIds) {
      if (!next.has(id)) {
        this._applyObjectColors(id, false)
      }
    }
    // Highlight the current selection.
    for (const id of objectIds) {
      this._applyObjectColors(id, true)
    }
    this.selectedObjectIds = [...objectIds]
  }

  /**
   * Highlight exactly the given instances (orange edges). Selection is
   * per-instance state, so selected placements materialize out of their
   * batches; deselected ones restore (the orange goes with the group).
   * Pass `[]` to clear the instance highlight.
   */
  setSelectedInstances(instanceIds: bigint[]): void {
    this.selectedInstanceIds = [...instanceIds]
    this._syncMaterialized()
    // Instances materialized for another reason (isolation-lit, preview)
    // still need their edge color to track selection membership.
    for (const id of this.instanceGroups.keys()) {
      this._applyInstanceColors(id, this.selectedInstanceIds.includes(id))
    }
  }

  /**
   * Highlight exactly the given sketches as a bright overlay drawn on
   * top of the normal sketch-line color, mirroring `setSelectedGuide`'s
   * "separate overlay object" approach rather than recoloring the merged
   * per-sketch-agnostic `sketchLines` buffer in place. Pass `[]` to clear.
   */
  setSelectedSketches(sketchIds: bigint[]): void {
    this.selectedSketchIds = [...sketchIds]
    this._rebuildSketchHighlight()
  }

  /** (Re)build the bright overlay for the selected sketches — solid lines in
   * the selection color, drawn on top so a selected sketch reads as picked
   * over the normal blue. Cleared if nothing is selected or all selected
   * sketches are gone (e.g. deleted). */
  private _rebuildSketchHighlight(): void {
    if (this.sketchHighlight !== null) {
      disposeFatSegments(this.sketchHighlight)
      this.sketchGroup.remove(this.sketchHighlight)
      this.sketchHighlight = null
    }
    if (this.selectedSketchIds.length === 0) return

    const positions: number[] = []
    for (const id of this.selectedSketchIds) {
      let linePositions: Float32Array | number[]
      try {
        linePositions = this.wasmScene.sketch_lines(id)
      } catch {
        continue // stale/deleted handle — simply contributes nothing
      }
      for (let i = 0; i < linePositions.length; i++) {
        positions.push(linePositions[i])
      }
    }
    if (positions.length === 0) return

    // Fat line (LineSegments2), like the base sketch lines — a plain
    // THREE.LineSegments/LineBasicMaterial ignores `linewidth` on WebGL and
    // renders 1px, which is thinner than the 2.2px fat sketch lines it's
    // meant to highlight and reads as nearly invisible. The fat-line registry
    // (makeFatSegments + updateFatLineResolutions on resize) keeps it sized.
    this.sketchHighlight = makeFatSegments(new Float32Array(positions), {
      color: EDGE_COLOR_SELECTED,
      widthPx: SKETCH_HIGHLIGHT_WIDTH_PX,
      depthTest: false,
    })
    this.sketchHighlight.renderOrder = 999
    this.sketchGroup.add(this.sketchHighlight)
  }

  /**
   * Set the active isolation lit set, or null for top level (nothing dimmed).
   * `lit` is a Set of leaf object ids that stay fully bright; all other objects
   * and all sketches dim. Idempotent; safe to call with the current value.
   *
   * `litInstances` is a parallel set for instance ids (null = no restriction).
   */
  setActiveContext(lit: Set<bigint> | null, litInstances: Set<bigint> | null = null): void {
    this.activeLitSet = lit
    this.activeLitInstanceSet = litInstances
    // Lit instances need per-instance opacity — materialize them; formerly
    // lit ones restore to their batch.
    this._syncMaterialized()
    this._applyIsolation()
  }

  /**
   * Update the hidden set and apply visibility. Hidden objects are invisible and
   * non-pickable (Viewport filters them out of pick results). Call after every
   * hiddenKeys change in App.
   */
  setHidden(hiddenObjectIds: bigint[], hiddenInstanceIds: bigint[]): void {
    this.hiddenObjectIds = new Set(hiddenObjectIds)
    this.hiddenInstanceIds = new Set(hiddenInstanceIds)
    this._applyHidden()
  }

  /** Re-apply visibility for all objects and instances. Batched instances
   * hide by degenerating their slot matrices (no materialization needed);
   * materialized ones use group.visible. */
  private _applyHidden(): void {
    for (const [id, g] of this.objectGroups) {
      g.group.visible = !this.hiddenObjectIds.has(id)
    }
    for (const [id, g] of this.instanceGroups) {
      g.group.visible = !this.hiddenInstanceIds.has(id)
    }
    for (const id of this.instanceRecords.keys()) {
      this._refreshSlots(id)
    }
  }

  /** Apply the context fade to all objects, instances, and sketches. */
  /** True when there is any solid geometry (objects or instances) to export. */
  hasExportableGeometry(): boolean {
    return this.objectGroups.size > 0 || this.instanceRecords.size > 0
  }

  /**
   * Build a throwaway THREE scene graph holding only the solid geometry —
   * object + instance *face* meshes, never edges, sketch, or guides — for
   * handing to a glTF exporter.
   *
   * Face materials are rebuilt as clean `MeshStandardMaterial` (metalness 0 /
   * roughness 1) carrying the true base color / texture / per-vertex colors,
   * with opacity reset to each material's real alpha (`userData.baseOpacity`)
   * so transient isolation dimming never leaks into the export. Selection
   * highlight only recolors edges (excluded here), so it cannot leak either.
   * Geometry buffers and textures are *shared by reference* with the live scene
   * (the exporter only reads them); the returned materials are fresh and must
   * be released by the caller via `disposeExportScene`.
   *
   * Instance nodes carry their pose as a node transform, so the exporter emits
   * real glTF node hierarchy + per-instance transforms.
   */
  buildExportScene(): THREE.Group {
    const root = new THREE.Group()
    root.name = 'HewModel'
    // Hew's world (and this three.js scene — camera.up = +Z) is Z-up, but glTF
    // is defined Y-up. Rotate the export root −90° about X so the emitted file is
    // a spec-compliant Y-up document (correct in Blender and round-tripping
    // through our own Y-up importer, which applies the inverse y_up_to_z_up).
    root.matrixAutoUpdate = false
    root.matrix.makeRotationX(-Math.PI / 2)
    root.matrixWorldNeedsUpdate = true

    const toStandard = (m: THREE.MeshPhongMaterial): THREE.MeshStandardMaterial => {
      const baseOpacity = (m.userData.baseOpacity as number | undefined) ?? 1
      return new THREE.MeshStandardMaterial({
        color: m.color.clone(),
        map: m.map ?? null,
        vertexColors: m.vertexColors,
        side: m.side,
        metalness: 0,
        roughness: 1,
        transparent: baseOpacity < 1,
        opacity: baseOpacity,
      })
    }

    const exportMesh = (src: THREE.Mesh, name: string): THREE.Mesh => {
      const mat = src.material
      const newMat = Array.isArray(mat)
        ? mat.map((m) => toStandard(m as THREE.MeshPhongMaterial))
        : toStandard(mat as THREE.MeshPhongMaterial)
      const out = new THREE.Mesh(src.geometry, newMat)
      out.name = name
      return out
    }

    for (const [id, g] of this.objectGroups) {
      root.add(exportMesh(g.facesMesh, `Object_${id}`))
    }

    // Node-per-instance with geometry shared by reference — the batch exists
    // for every rendered placement (materializing only zeroes its slot), so
    // its geometry + materials are the export source for batched AND
    // materialized instances alike.
    for (const [id, rec] of this.instanceRecords) {
      const node = new THREE.Group()
      node.name = `Instance_${id}`
      node.matrixAutoUpdate = false
      node.matrix.copy(rec.matrix)
      node.matrixWorldNeedsUpdate = true
      for (const memberId of rec.memberIds) {
        const key = this._batchKeyFor(memberId, rec)
        const batch = key !== undefined ? this.batches.get(key) : undefined
        if (batch === undefined) continue
        node.add(exportMesh(batch.mesh, `Instance_${id}_member_${memberId}`))
      }
      root.add(node)
    }

    return root
  }

  /**
   * Release the fresh materials created by `buildExportScene`. Geometry and
   * textures are shared with the live scene and are intentionally left intact.
   */
  disposeExportScene(root: THREE.Group): void {
    root.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      if (mesh.isMesh !== true) return
      const mat = mesh.material
      if (Array.isArray(mat)) {
        for (const m of mat) m.dispose()
      } else {
        mat.dispose()
      }
    })
  }

  private _applyIsolation(): void {
    for (const [id, g] of this.objectGroups) {
      const dimmed = this.activeLitSet !== null && !this.activeLitSet.has(id)
      this._setObjectOpacity(g, dimmed ? DIMMED_OPACITY : 1)
    }
    this._applyInstanceIsolation()
    this._applySketchIsolation()
  }

  private _applyInstanceIsolation(): void {
    // Batched placements are never in a lit set (lit instances materialize in
    // _syncMaterialized), so batch dimming is uniform: dim whenever any
    // isolation context is active.
    const batchDimmed = this.activeLitSet !== null || this.activeLitInstanceSet !== null
    for (const batch of this.batches.values()) {
      this._setBatchOpacity(batch, batchDimmed ? DIMMED_OPACITY : 1)
    }
    for (const [id, g] of this.instanceGroups) {
      const dimmed = this.activeLitSet !== null ||
        (this.activeLitInstanceSet !== null && !this.activeLitInstanceSet.has(id))
      this._setInstanceOpacity(g, dimmed ? DIMMED_OPACITY : 1)
    }
  }

  private _setBatchOpacity(batch: MemberBatch, opacity: number): void {
    const setFaceMatOpacity = (m: THREE.MeshPhongMaterial) => {
      const eff = opacity * ((m.userData.baseOpacity as number | undefined) ?? 1)
      m.opacity = eff
      m.transparent = eff < 1
      m.depthWrite = eff >= 1
    }
    const mat = batch.mesh.material
    if (Array.isArray(mat)) {
      for (const m of mat) {
        setFaceMatOpacity(m as THREE.MeshPhongMaterial)
      }
    } else {
      setFaceMatOpacity(mat as THREE.MeshPhongMaterial)
    }
    const edgeMat = batch.edges.material as THREE.LineBasicMaterial
    edgeMat.opacity = opacity
    edgeMat.transparent = opacity < 1
  }

  private _setInstanceOpacity(g: InstanceMeshGroup, opacity: number): void {
    const setFaceMatOpacity = (m: THREE.MeshPhongMaterial) => {
      const eff = opacity * ((m.userData.baseOpacity as number | undefined) ?? 1)
      m.opacity = eff
      m.transparent = eff < 1
      m.depthWrite = eff >= 1
    }
    for (const mesh of g.facesMeshes) {
      const mat = mesh.material
      if (Array.isArray(mat)) {
        for (const m of mat) {
          setFaceMatOpacity(m as THREE.MeshPhongMaterial)
        }
      } else {
        setFaceMatOpacity(mat as THREE.MeshPhongMaterial)
      }
    }
    for (const lines of g.edgesLines) {
      const mat = lines.material as THREE.LineBasicMaterial
      mat.opacity = opacity
      mat.transparent = opacity < 1
    }
  }

  private _setObjectOpacity(g: ObjectMeshGroup, opacity: number): void {
    const mat = g.facesMesh.material
    const setFaceMatOpacity = (m: THREE.MeshPhongMaterial) => {
      const eff = opacity * ((m.userData.baseOpacity as number | undefined) ?? 1)
      m.opacity = eff
      m.transparent = eff < 1
      m.depthWrite = eff >= 1
    }
    if (Array.isArray(mat)) {
      for (const m of mat) {
        setFaceMatOpacity(m as THREE.MeshPhongMaterial)
      }
    } else {
      setFaceMatOpacity(mat as THREE.MeshPhongMaterial)
    }
    const edgeMat = g.edgesLines.material as THREE.LineBasicMaterial
    edgeMat.opacity = opacity
    edgeMat.transparent = opacity < 1
  }

  /** Fade sketch lines + region fills whenever any context is active. */
  private _applySketchIsolation(): void {
    const inside = this.activeLitSet !== null
    if (this.sketchLines !== null) {
      const mat = this.sketchLines.material as LineMaterial
      mat.opacity = inside ? DIMMED_OPACITY : 1
      mat.transparent = inside
    }
    for (const mesh of this.sketchRegionMeshes.values()) {
      const mat = mesh.material as THREE.MeshBasicMaterial
      mat.opacity = inside ? DIMMED_OPACITY * 0.5 : SKETCH_REGION_OPACITY
    }
  }

  private _applyObjectColors(objectId: bigint, selected: boolean): void {
    const g = this.objectGroups.get(objectId)
    if (g === undefined) return

    // Face fill is owned by material color — only edge color changes for selection.
    const edgeColor = selected ? EDGE_COLOR_SELECTED : EDGE_COLOR
    const edgeMat = g.edgesLines.material as THREE.LineBasicMaterial
    edgeMat.color.setHex(edgeColor)
  }

  private _applyInstanceColors(instanceId: bigint, selected: boolean): void {
    const g = this.instanceGroups.get(instanceId)
    if (g === undefined) return
    // Face fill is owned by material color — only edge color changes for selection.
    const edgeColor = selected ? EDGE_COLOR_SELECTED : EDGE_COLOR
    for (const lines of g.edgesLines) {
      ;(lines.material as THREE.LineBasicMaterial).color.setHex(edgeColor)
    }
  }

  clearSketchRegion(): void {
    this._clearSketchRegions()
  }

  /**
   * Look up the rendered THREE.Group for a given instance id (for preview
   * cloning in transform tools), MATERIALIZING the placement out of its batch
   * on demand — the commit's refresh restores it. Returns null if the
   * instance is not rendered.
   */
  getInstanceGroup(instanceId: bigint): THREE.Group | null {
    const existing = this.instanceGroups.get(instanceId)
    if (existing !== undefined) return existing.group
    if (!this.instanceRecords.has(instanceId)) return null
    this.previewMaterialized.add(instanceId)
    this._materialize(instanceId)
    return this.instanceGroups.get(instanceId)?.group ?? null
  }

  /** The rendered group for one object id — map-backed, unlike a name walk
   * over `objectsGroup` (marquee hit-testing calls this per candidate). */
  getObjectGroup(objectId: bigint): THREE.Group | null {
    return this.objectGroups.get(objectId)?.group ?? null
  }

  private _clearSketchLines(): void {
    if (this.sketchLines !== null) {
      disposeFatSegments(this.sketchLines)
      this.sketchGroup.remove(this.sketchLines)
      this.sketchLines = null
    }
  }

  private _clearSketchRegions(): void {
    for (const mesh of this.sketchRegionMeshes.values()) {
      mesh.geometry.dispose()
      ;(mesh.material as THREE.Material).dispose()
      this.sketchGroup.remove(mesh)
    }
    this.sketchRegionMeshes.clear()
  }

  /** Dispose all GPU resources */
  dispose(): void {
    for (const id of [...this.objectGroups.keys()]) {
      this._removeObjectGroup(id)
    }
    for (const id of [...this.instanceGroups.keys()]) {
      this._disposeMaterializedGroup(id)
    }
    this._disposeAllBatches()
    this.instanceRecords.clear()
    this.previewMaterialized.clear()
    this.memberGeometryCache.clear()
    // Dispose and revoke cached textures.
    for (const tex of this.textureCache.values()) {
      tex.dispose()
    }
    this.textureCache.clear()
    this._clearSketchLines()
    this._clearSketchRegions()
    this._clearGuides()
  }
}
