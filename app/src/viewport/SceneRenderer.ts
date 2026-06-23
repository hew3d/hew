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

/** Default neutral face color (matches DEFAULT_MATERIAL_RGBA in tessellate). */
const FACE_COLOR_DEFAULT = 0xcccccc
/** Edge color — dark for readability. */
const EDGE_COLOR = 0x1a1a1a
/** Orange highlight for selected object edges (kept for selection; face fill uses material color). */
const EDGE_COLOR_SELECTED = 0xffaa00
const SKETCH_LINE_COLOR = 0x2266cc
const SKETCH_REGION_COLOR = 0x88aadd
/** Normal translucency of a sketch region fill. */
const SKETCH_REGION_OPACITY = 0.35
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
  /** Parent group for construction guide geometry */
  readonly guidesGroup: THREE.Group

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
  private memberGeometryCache: Map<bigint, {
    positions: Float32Array
    normals: Float32Array
    indices: Uint32Array
    colors: Float32Array
    uvs: Float32Array
    groupMaterialIds: BigUint64Array
    groupStarts: Uint32Array
    groupCounts: Uint32Array
    edgePositions: Float32Array
  }> = new Map()

  /**
   * THREE.Texture cache, keyed by material id (as string). Built once per id
   * and shared across instances so we never duplicate GPU texture objects.
   */
  private textureCache: Map<string, THREE.Texture> = new Map()
  private sketchLines: THREE.LineSegments | null = null
  /** One fill mesh per sketch region, keyed by `${sketchHandle}:${regionHandle}`
   *  (region handles are per-sketch, so they can collide across sketches). */
  private sketchRegionMeshes: Map<string, THREE.Mesh> = new Map()
  /** Merged LineSegments for every dashed line guide. */
  private guideLines: THREE.LineSegments | null = null
  /** Last dash size applied to `guideLines` (screen-constant); -1 = none yet. */
  private lastGuideDashSize = -1
  /** Merged LineSegments for every point guide's cross marker. */
  private guideMarkers: THREE.LineSegments | null = null
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
    // Re-apply hidden visibility for instances (groups rebuilt above).
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
            colors: mesh.colors(),
            uvs: mesh.uvs(),
            groupMaterialIds: BigUint64Array.from(mesh.group_material_ids()),
            groupStarts: mesh.group_starts(),
            groupCounts: mesh.group_counts(),
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
      // Material may be a single material or an array (multi-material mesh).
      const mat = mesh.material
      if (Array.isArray(mat)) {
        for (const m of mat) {
          m.dispose()
        }
      } else {
        ;(mat as THREE.Material).dispose()
      }
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
              t.wrapS = THREE.RepeatWrapping
              t.wrapT = THREE.RepeatWrapping
              this.textureCache.set(midStr, t)
            }
          }
          tex = this.textureCache.get(midStr)
        }

        const color = info !== undefined
          ? new THREE.Color(info.r() / 255, info.g() / 255, info.b() / 255)
          : new THREE.Color(FACE_COLOR_DEFAULT / 0xffffff)

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
      const colors = mesh.colors()
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

      const faceMaterials = this._buildMaterialArray(
        groupMaterialIds,
        groupStarts,
        groupCounts,
        faceGeo,
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

  /** Re-apply group.visible for all object / instance groups. */
  private _applyHidden(): void {
    for (const [id, g] of this.objectGroups) {
      g.group.visible = !this.hiddenObjectIds.has(id)
    }
    for (const [id, g] of this.instanceGroups) {
      g.group.visible = !this.hiddenInstanceIds.has(id)
    }
  }

  /** Apply the context fade to all objects, instances, and sketches. */
  /** True when there is any solid geometry (objects or instances) to export. */
  hasExportableGeometry(): boolean {
    return this.objectGroups.size > 0 || this.instanceGroups.size > 0
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

    for (const [id, g] of this.instanceGroups) {
      const node = new THREE.Group()
      node.name = `Instance_${id}`
      node.matrixAutoUpdate = false
      node.matrix.copy(g.group.matrix)
      node.matrixWorldNeedsUpdate = true
      g.facesMeshes.forEach((fm, i) => {
        node.add(exportMesh(fm, `Instance_${id}_member_${g.memberIds[i]}`))
      })
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
    for (const [id, g] of this.instanceGroups) {
      const dimmed = this.activeLitSet !== null ||
        (this.activeLitInstanceSet !== null && !this.activeLitInstanceSet.has(id))
      this._setInstanceOpacity(g, dimmed ? DIMMED_OPACITY : 1)
    }
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
