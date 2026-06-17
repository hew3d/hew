/**
 * SceneRenderer — manages THREE.js meshes for live scene geometry.
 *
 * After any committed kernel mutation, call `refresh()` to:
 *   1. Pull object_ids() and object_mesh(id) for each object
 *   2. Rebuild the viewport meshes (flat-shaded faces + edge LineSegments)
 *   3. Dispose old GPU buffers
 *
 * Also manages sketch geometry for every document sketch (refreshAllSketches).
 */

import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'

const FACE_COLOR = 0xa8c8e8
const FACE_COLOR_LEAKY = 0xe8a8a8   // reddish tint for non-watertight
const FACE_COLOR_SELECTED = 0xc8e0f8  // slightly brighter blue for selected
const FACE_COLOR_INSTANCE = 0xb8d4f0  // slightly distinct blue for instance members
const EDGE_COLOR = 0x1a1a1a
const EDGE_COLOR_SELECTED = 0xffaa00  // orange highlight for selected object edges
const SKETCH_LINE_COLOR = 0x2266cc
const SKETCH_REGION_COLOR = 0x88aadd
/** Normal translucency of a sketch region fill. */
const SKETCH_REGION_OPACITY = 0.35
/** Opacity of entities faded out by the active editing context (isolation). */
const DIMMED_OPACITY = 0.15

/** Disposable group for one object's faces + edges */
interface ObjectMeshGroup {
  objectId: bigint
  facesMesh: THREE.Mesh
  edgesLines: THREE.LineSegments
  group: THREE.Group
}

/**
 * One rendered instance: a THREE.Group holding clones of each member object's
 * mesh, positioned at the instance pose.  All member meshes are drawn from the
 * shared BufferGeometry cache so we never duplicate GPU buffers.
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

export class SceneRenderer {
  private scene: THREE.Scene
  private wasmScene: WasmScene

  /** Parent group for all object geometry */
  readonly objectsGroup: THREE.Group
  /** Parent group for sketch lines/regions */
  readonly sketchGroup: THREE.Group
  /** Parent group for all instance geometry */
  readonly instancesGroup: THREE.Group

  private objectGroups: Map<bigint, ObjectMeshGroup> = new Map()
  /** Rendered instance groups, keyed by instance id */
  private instanceGroups: Map<bigint, InstanceMeshGroup> = new Map()
  /**
   * Shared typed-array cache for definition member objects.
   * Keyed by member object id; the raw arrays are shared across all instances of
   * one def, but each instance creates its own BufferAttribute wrappers (so that
   * geometry.dispose() on one instance does not delete GPU buffers shared with
   * another instance — three.js ties GPU buffer lifetime to the BufferAttribute,
   * not to the underlying TypedArray).
   * Invalidated when a component definition is edited (refreshInstances re-builds it).
   */
  private memberGeometryCache: Map<bigint, { positions: Float32Array; normals: Float32Array; indices: Uint32Array; edgePositions: Float32Array }> = new Map()
  private sketchLines: THREE.LineSegments | null = null
  /** One fill mesh per sketch region, keyed by `${sketchHandle}:${regionHandle}`
   *  (region handles are per-sketch, so they can collide across sketches). */
  private sketchRegionMeshes: Map<string, THREE.Mesh> = new Map()
  /** The sketch currently being drawn into (tool target) — null if none. */
  private activeSketchHandle: bigint | null = null
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

    return new Map(this.watertightMap)
  }

  /**
   * Rebuild all instance groups from instance_ids(). Call after any mutation
   * that may add/remove instances or change definition geometry.
   *
   * Invalidates the member geometry cache so shared member BufferGeometries are
   * re-pulled from the kernel (which has invalidated its tessellation cache on
   * mutation). Then re-builds each instance group from scratch.
   */
  refreshInstances(): void {
    // Invalidate the shared member geometry cache — definition may have changed.
    this.memberGeometryCache.clear()

    const instanceIds = this.wasmScene.instance_ids()
    const newIds = new Set<bigint>(instanceIds)

    // Remove stale instances
    for (const oldId of [...this.instanceGroups.keys()]) {
      if (!newIds.has(oldId)) {
        this._removeInstanceGroup(oldId)
      }
    }

    // Add or update each instance
    for (let i = 0; i < instanceIds.length; i++) {
      this._refreshInstance(instanceIds[i])
    }

    this._applyInstanceIsolation()
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

  /** Build or re-build the THREE.js group for one instance. */
  private _refreshInstance(instanceId: bigint): void {
    // Always rebuild from scratch (cheap: member geometry comes from cache).
    if (this.instanceGroups.has(instanceId)) {
      this._removeInstanceGroup(instanceId)
    }

    const componentId = this.wasmScene.instance_def(instanceId)
    if (componentId === undefined) return

    const pose = this.wasmScene.instance_pose(instanceId)
    if (pose === undefined) return

    const memberIds = Array.from(this.wasmScene.component_member_objects(componentId))

    const group = new THREE.Group()
    group.name = `Instance_${instanceId}`

    // Build the THREE.Matrix4 from the row-major 3×4 pose.
    // THREE Matrix4 is column-major; set() takes row-major.
    const m4 = new THREE.Matrix4()
    m4.set(
      pose[0], pose[1], pose[2],  pose[3],
      pose[4], pose[5], pose[6],  pose[7],
      pose[8], pose[9], pose[10], pose[11],
      0,       0,       0,        1,
    )
    group.matrixAutoUpdate = false
    group.matrix.copy(m4)
    group.matrixWorldNeedsUpdate = true

    // Reflected pose: flip front/back face winding.
    const reflected = this._poseDet(pose) < 0
    const side = reflected ? THREE.BackSide : THREE.FrontSide

    const facesMeshes: THREE.Mesh[] = []
    const edgesLinesList: THREE.LineSegments[] = []

    for (const memberId of memberIds) {
      // Fetch and cache member typed arrays (raw data only — not BufferAttributes,
      // to avoid GPU buffer aliasing on dispose).
      let cached = this.memberGeometryCache.get(memberId)
      if (cached === undefined) {
        const mesh = this.wasmScene.object_mesh(memberId)
        try {
          cached = {
            positions: mesh.positions(),
            normals: mesh.normals(),
            indices: mesh.indices(),
            edgePositions: mesh.edge_positions(),
          }
          this.memberGeometryCache.set(memberId, cached)
        } finally {
          mesh.free()
        }
      }

      // Face mesh — each instance gets its own BufferAttribute wrappers wrapping
      // the shared TypedArrays.  geometry.dispose() on this instance frees only
      // this instance's GPU buffers without affecting others.
      const faceGeo = new THREE.BufferGeometry()
      faceGeo.setAttribute('position', new THREE.BufferAttribute(cached.positions, 3))
      faceGeo.setAttribute('normal', new THREE.BufferAttribute(cached.normals, 3))
      faceGeo.setIndex(new THREE.BufferAttribute(cached.indices, 1))
      const faceMat = new THREE.MeshPhongMaterial({
        color: FACE_COLOR_INSTANCE,
        flatShading: true,
        side,
      })
      const facesMesh = new THREE.Mesh(faceGeo, faceMat)
      facesMesh.name = `InstanceFace_${instanceId}_${memberId}`
      group.add(facesMesh)
      facesMeshes.push(facesMesh)

      // Edge lines
      const edgeGeo = new THREE.BufferGeometry()
      edgeGeo.setAttribute('position', new THREE.BufferAttribute(cached.edgePositions, 3))
      const edgeMat = new THREE.LineBasicMaterial({ color: EDGE_COLOR })
      const edgesLines = new THREE.LineSegments(edgeGeo, edgeMat)
      edgesLines.name = `InstanceEdge_${instanceId}_${memberId}`
      group.add(edgesLines)
      edgesLinesList.push(edgesLines)
    }

    this.instancesGroup.add(group)
    this.instanceGroups.set(instanceId, { instanceId, memberIds, group, facesMeshes, edgesLines: edgesLinesList })

    // Re-apply selection highlight if selected
    if (this.selectedInstanceIds.includes(instanceId)) {
      this._applyInstanceColors(instanceId, true)
    }
  }

  private _removeInstanceGroup(instanceId: bigint): void {
    const g = this.instanceGroups.get(instanceId)
    if (g === undefined) return

    // Dispose only materials (geometry is either shared from cache or per-instance)
    for (const mesh of g.facesMeshes) {
      // Only dispose the material — geometry attribute buffers are shared
      ;(mesh.material as THREE.Material).dispose()
      // The geometry references shared attributes; just dispose the container
      mesh.geometry.dispose()
    }
    for (const lines of g.edgesLines) {
      ;(lines.material as THREE.Material).dispose()
      lines.geometry.dispose()
    }

    this.instancesGroup.remove(g.group)
    this.instanceGroups.delete(instanceId)
    this.selectedInstanceIds = this.selectedInstanceIds.filter((id) => id !== instanceId)
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
      const edgePositions = mesh.edge_positions()

      // Face mesh
      const faceGeo = new THREE.BufferGeometry()
      faceGeo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
      faceGeo.setAttribute('normal', new THREE.BufferAttribute(normals, 3))
      faceGeo.setIndex(new THREE.BufferAttribute(indices, 1))
      const faceMat = new THREE.MeshPhongMaterial({
        color: watertight ? FACE_COLOR : FACE_COLOR_LEAKY,
        flatShading: true,
      })
      const facesMesh = new THREE.Mesh(faceGeo, faceMat)

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
    ;(g.facesMesh.material as THREE.Material).dispose()
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
   * extrusion, or scene undo/redo. `activeHandle`, if given, records which
   * sketch the tools should draw into next.
   *
   * All sketches' edges are merged into one LineSegments buffer; each region is
   * a triangle-fan fill keyed by `${sketchHandle}:${regionHandle}`.
   */
  refreshAllSketches(activeHandle?: bigint): void {
    if (activeHandle !== undefined) {
      this.activeSketchHandle = activeHandle
    }
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
      const geo = new THREE.BufferGeometry()
      geo.setAttribute(
        'position',
        new THREE.BufferAttribute(new Float32Array(allLinePositions), 3),
      )
      const mat = new THREE.LineBasicMaterial({
        color: SKETCH_LINE_COLOR,
        linewidth: 2,
      })
      this.sketchLines = new THREE.LineSegments(geo, mat)
      this.sketchGroup.add(this.sketchLines)
    }

    // Rebuilt sketch geometry starts at full strength; re-apply the fade.
    this._applySketchIsolation()
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

      const positions: number[] = []
      for (let t = 1; t < n - 1; t++) {
        positions.push(boundary[0], boundary[1], 0.001)
        positions.push(boundary[t * 3], boundary[t * 3 + 1], 0.001)
        positions.push(boundary[(t + 1) * 3], boundary[(t + 1) * 3 + 1], 0.001)
      }

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
   * Highlight exactly the given instances (orange edges, brighter faces).
   * Pass `[]` to clear the instance highlight.
   */
  setSelectedInstances(instanceIds: bigint[]): void {
    const next = new Set(instanceIds)
    for (const id of this.selectedInstanceIds) {
      if (!next.has(id)) {
        this._applyInstanceColors(id, false)
      }
    }
    for (const id of instanceIds) {
      this._applyInstanceColors(id, true)
    }
    this.selectedInstanceIds = [...instanceIds]
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
    this._applyIsolation()
  }

  /** Apply the context fade to all objects, instances, and sketches. */
  private _applyIsolation(): void {
    for (const [id, g] of this.objectGroups) {
      const dimmed = this.activeLitSet !== null && !this.activeLitSet.has(id)
      this._setObjectOpacity(g, dimmed ? DIMMED_OPACITY : 1)
    }
    this._applyInstanceIsolation()
    this._applySketchIsolation()
  }

  private _applyInstanceIsolation(): void {
    for (const [id, g] of this.instanceGroups) {
      const dimmed = this.activeLitSet !== null ||
        (this.activeLitInstanceSet !== null && !this.activeLitInstanceSet.has(id))
      this._setInstanceOpacity(g, dimmed ? DIMMED_OPACITY : 1)
    }
  }

  private _setInstanceOpacity(g: InstanceMeshGroup, opacity: number): void {
    for (const mesh of g.facesMeshes) {
      const mat = mesh.material as THREE.MeshPhongMaterial
      mat.opacity = opacity
      mat.transparent = opacity < 1
      mat.depthWrite = opacity >= 1
    }
    for (const lines of g.edgesLines) {
      const mat = lines.material as THREE.LineBasicMaterial
      mat.opacity = opacity
      mat.transparent = opacity < 1
    }
  }

  private _setObjectOpacity(g: ObjectMeshGroup, opacity: number): void {
    const faceMat = g.facesMesh.material as THREE.MeshPhongMaterial
    faceMat.opacity = opacity
    faceMat.transparent = opacity < 1
    faceMat.depthWrite = opacity >= 1
    const edgeMat = g.edgesLines.material as THREE.LineBasicMaterial
    edgeMat.opacity = opacity
    edgeMat.transparent = opacity < 1
  }

  /** Fade sketch lines + region fills whenever any context is active. */
  private _applySketchIsolation(): void {
    const inside = this.activeLitSet !== null
    if (this.sketchLines !== null) {
      const mat = this.sketchLines.material as THREE.LineBasicMaterial
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

    const watertight = this.watertightMap.get(objectId) ?? true
    const faceColor = selected
      ? FACE_COLOR_SELECTED
      : (watertight ? FACE_COLOR : FACE_COLOR_LEAKY)
    const edgeColor = selected ? EDGE_COLOR_SELECTED : EDGE_COLOR

    const faceMat = g.facesMesh.material as THREE.MeshPhongMaterial
    faceMat.color.setHex(faceColor)

    const edgeMat = g.edgesLines.material as THREE.LineBasicMaterial
    edgeMat.color.setHex(edgeColor)
  }

  private _applyInstanceColors(instanceId: bigint, selected: boolean): void {
    const g = this.instanceGroups.get(instanceId)
    if (g === undefined) return
    const faceColor = selected ? FACE_COLOR_SELECTED : FACE_COLOR_INSTANCE
    const edgeColor = selected ? EDGE_COLOR_SELECTED : EDGE_COLOR
    for (const mesh of g.facesMeshes) {
      ;(mesh.material as THREE.MeshPhongMaterial).color.setHex(faceColor)
    }
    for (const lines of g.edgesLines) {
      ;(lines.material as THREE.LineBasicMaterial).color.setHex(edgeColor)
    }
  }

  clearSketchRegion(): void {
    this._clearSketchRegions()
  }

  get currentSketchHandle(): bigint | null {
    return this.activeSketchHandle
  }

  /**
   * Look up the rendered THREE.Group for a given instance id (for preview
   * cloning in transform tools). Returns null if the instance is not rendered.
   */
  getInstanceGroup(instanceId: bigint): THREE.Group | null {
    return this.instanceGroups.get(instanceId)?.group ?? null
  }

  private _clearSketchLines(): void {
    if (this.sketchLines !== null) {
      this.sketchLines.geometry.dispose()
      ;(this.sketchLines.material as THREE.Material).dispose()
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
      this._removeInstanceGroup(id)
    }
    this.memberGeometryCache.clear()
    this._clearSketchLines()
    this._clearSketchRegions()
  }
}
