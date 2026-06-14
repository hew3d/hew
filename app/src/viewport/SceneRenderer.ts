/**
 * SceneRenderer — manages THREE.js meshes for live scene geometry.
 *
 * After any committed kernel mutation, call `refresh()` to:
 *   1. Pull object_ids() and object_mesh(id) for each object
 *   2. Rebuild the viewport meshes (flat-shaded faces + edge LineSegments)
 *   3. Dispose old GPU buffers
 *
 * Also manages sketch line geometry (refreshed via refreshSketch).
 */

import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'

const FACE_COLOR = 0xa8c8e8
const FACE_COLOR_LEAKY = 0xe8a8a8   // reddish tint for non-watertight
const FACE_COLOR_SELECTED = 0xc8e0f8  // slightly brighter blue for selected
const EDGE_COLOR = 0x1a1a1a
const EDGE_COLOR_SELECTED = 0xffaa00  // orange highlight for selected object edges
const SKETCH_LINE_COLOR = 0x2266cc
const SKETCH_REGION_COLOR = 0x88aadd

/** Disposable group for one object's faces + edges */
interface ObjectMeshGroup {
  objectId: bigint
  facesMesh: THREE.Mesh
  edgesLines: THREE.LineSegments
  group: THREE.Group
}

export class SceneRenderer {
  private scene: THREE.Scene
  private wasmScene: WasmScene

  /** Parent group for all object geometry */
  readonly objectsGroup: THREE.Group
  /** Parent group for sketch lines/regions */
  readonly sketchGroup: THREE.Group

  private objectGroups: Map<bigint, ObjectMeshGroup> = new Map()
  private sketchLines: THREE.LineSegments | null = null
  /** One fill mesh per sketch region (keyed by region handle) */
  private sketchRegionMeshes: Map<bigint, THREE.Mesh> = new Map()
  /** Current sketch handle — null if no active sketch */
  private activeSketchHandle: bigint | null = null
  /** Last known watertight state per object */
  private watertightMap: Map<bigint, boolean> = new Map()
  /** Currently selected object id, or null */
  private selectedObjectId: bigint | null = null

  constructor(threeScene: THREE.Scene, wasmScene: WasmScene) {
    this.scene = threeScene
    this.wasmScene = wasmScene

    this.objectsGroup = new THREE.Group()
    this.objectsGroup.name = 'Objects'
    threeScene.add(this.objectsGroup)

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

    return new Map(this.watertightMap)
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
      if (objectId === this.selectedObjectId) {
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
    // If the removed object was selected, clear the selection
    if (this.selectedObjectId === objectId) {
      this.selectedObjectId = null
    }
  }

  /** Update sketch line rendering. Call after any sketch mutation. */
  refreshSketch(sketchHandle: bigint): void {
    this.activeSketchHandle = sketchHandle
    this._clearSketchLines()

    const linePositions = this.wasmScene.sketch_lines(sketchHandle)
    if (linePositions.length > 0) {
      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.BufferAttribute(linePositions, 3))
      const mat = new THREE.LineBasicMaterial({
        color: SKETCH_LINE_COLOR,
        linewidth: 2,
      })
      this.sketchLines = new THREE.LineSegments(geo, mat)
      this.sketchGroup.add(this.sketchLines)
    }
  }

  /**
   * Refresh all sketch region fills for the given sketch.
   *
   * Iterates sketch_regions(), calls region_boundary() for each, and builds
   * a separate triangle-fan mesh per region. Old region meshes are disposed
   * first. For M1 rectangles the boundary is a simple convex polygon, so
   * fan from vertex 0 is correct.
   */
  showSketchRegion(sketchHandle: bigint): void {
    this._clearSketchRegions()

    const regionHandles = this.wasmScene.sketch_regions(sketchHandle)
    for (let i = 0; i < regionHandles.length; i++) {
      const regionHandle = regionHandles[i]
      const boundary = this.wasmScene.region_boundary(sketchHandle, regionHandle)
      // boundary is flat [x0,y0,z0, x1,y1,z1, ...]; n vertices
      const n = Math.floor(boundary.length / 3)
      if (n < 3) continue

      // Build triangle fan from vertex 0 — valid for convex polygons (M1 rectangles)
      const positions: number[] = []
      for (let t = 1; t < n - 1; t++) {
        // vertex 0
        positions.push(boundary[0], boundary[1], 0.001)
        // vertex t
        positions.push(boundary[t * 3], boundary[t * 3 + 1], 0.001)
        // vertex t+1
        positions.push(boundary[(t + 1) * 3], boundary[(t + 1) * 3 + 1], 0.001)
      }

      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3))
      const mat = new THREE.MeshBasicMaterial({
        color: SKETCH_REGION_COLOR,
        transparent: true,
        opacity: 0.35,
        side: THREE.DoubleSide,
        depthWrite: false,
      })
      const mesh = new THREE.Mesh(geo, mat)
      this.sketchGroup.add(mesh)
      this.sketchRegionMeshes.set(regionHandle, mesh)
    }
  }

  /**
   * Highlight the given object (orange edges, slightly brighter faces) and
   * restore the previously selected object to normal colors.  Pass null to
   * clear the selection without selecting anything new.
   */
  setSelected(objectId: bigint | null): void {
    // Deselect the previously selected object
    if (this.selectedObjectId !== null) {
      this._applyObjectColors(this.selectedObjectId, false)
    }

    this.selectedObjectId = objectId

    // Select the new object
    if (objectId !== null) {
      this._applyObjectColors(objectId, true)
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

  clearSketchRegion(): void {
    this._clearSketchRegions()
  }

  get currentSketchHandle(): bigint | null {
    return this.activeSketchHandle
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
    this._clearSketchLines()
    this._clearSketchRegions()
  }
}
