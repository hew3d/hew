import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import type { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'
import {
  DrawPlaneCueLayer,
  OUTLINE_LINE,
  PATCH_LINE,
  PATCH_LINE_HELD,
  PATCH_MAX_PX,
  PATCH_MIN_PX,
  PATCH_SIZE_M,
  patchWorldSize,
  planeCueFor,
  type PlaneCueSpec,
} from './DrawPlaneCueLayer'
import { axisDrawPlane, drawPlaneThrough, groundDrawPlane, type DrawPlaneState } from '../tools/drawPlane'
import { DARK_AXIS_COLORS, PLANE_CUSTOM_COLORS } from './axisColors'
import type { V3 } from './geoHelpers'

const GREEN = axisDrawPlane(1, [0, 0, 0])
const AT: V3 = [0.3, 0, 0.5]
const LOOP: V3[] = [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]]

function state(over: Partial<DrawPlaneState> & { hover?: DrawPlaneState['hover'] }): DrawPlaneState {
  return { plane: GREEN, pip: 'green', hold: 'inferred', hover: { kind: 'space', point: AT }, ...over }
}

const lineOf = (layer: DrawPlaneCueLayer) => layer.group.children[0] as LineSegments2
const materialOf = (layer: DrawPlaneCueLayer) => lineOf(layer).material as LineMaterial

describe('planeCueFor — which cue the cursor gets (view-die §4)', () => {
  it('none without a draw tool or off the canvas', () => {
    expect(planeCueFor(null)).toBeNull()
    expect(planeCueFor(state({ hover: null }))).toBeNull()
    expect(planeCueFor(state({ hold: 'locked', hover: null }))).toBeNull()
  })

  it('none over the ground with no hold: the world grid is the cue', () => {
    expect(planeCueFor(state({ plane: groundDrawPlane(), pip: 'blue', hover: { kind: 'ground', point: [1, 2, 0] } }))).toBeNull()
  })

  it('none over a plain face or a sketch', () => {
    expect(planeCueFor(state({ hover: { kind: 'face', point: AT } }))).toBeNull()
    expect(planeCueFor(state({ hover: { kind: 'sketch', point: AT } }))).toBeNull()
  })

  it('a lean onto a face: that face\'s outline in the plane\'s pip', () => {
    expect(planeCueFor(state({ hover: { kind: 'lean', point: AT, faceLoop: LOOP } }))).toEqual({
      kind: 'outline', pip: 'green', normal: GREEN.normal, loop: LOOP,
    })
  })

  it('a lean onto a curved face (no planar loop) gets the small patch instead', () => {
    expect(planeCueFor(state({ hover: { kind: 'lean', point: AT } }))).toEqual({
      kind: 'patch', pip: 'green', plane: GREEN, through: AT, held: false,
    })
  })

  it('an extended lean off the object, and empty space on an inferred vertical plane: the small patch', () => {
    expect(planeCueFor(state({ hover: { kind: 'leanExtended', point: AT } }))).toMatchObject({ kind: 'patch', held: false, through: AT })
    expect(planeCueFor(state({ hover: { kind: 'space', point: AT } }))).toMatchObject({ kind: 'patch', held: false, through: AT })
  })

  it('empty space on the ground itself: none, even when a moved frame built the plane (not the literal ground)', () => {
    expect(planeCueFor(state({ plane: groundDrawPlane(), pip: 'blue', hover: { kind: 'space', point: [0, 0, 0] } }))).toBeNull()
    const shifted = axisDrawPlane(2, [2, 3, 0], { origin: [2, 3, 0], x: [0, 1, 0], y: [-1, 0, 0], z: [0, 0, 1] })
    expect(shifted.ground).toBe(false)
    expect(planeCueFor(state({ plane: shifted, pip: 'blue', hover: { kind: 'space', point: [2, 3, 0] } }))).toBeNull()
  })

  it('any held state: the small patch at the cursor, held weight, whatever is under it', () => {
    for (const hold of ['locked', 'pinned', 'anchored'] as const) {
      for (const kind of ['ground', 'face', 'sketch', 'space'] as const) {
        expect(planeCueFor(state({ hold, hover: { kind, point: AT } }))).toEqual({
          kind: 'patch', pip: 'green', plane: GREEN, through: AT, held: true,
        })
      }
    }
    // Held on the ground too: the lock, pin or gesture is the news.
    expect(planeCueFor(state({ plane: groundDrawPlane(), pip: 'blue', hold: 'locked', hover: { kind: 'space', point: [0, 0, 0] } })))
      .toMatchObject({ kind: 'patch', held: true })
  })
})

describe('patchWorldSize — 0.5 m clamped to 36–72 px on screen', () => {
  it('0.5 m where that is between 36 and 72 px', () => {
    expect(patchWorldSize(0.01)).toBe(PATCH_SIZE_M) // 50 px
  })

  it('clamps at 36 px zoomed far out and 72 px zoomed far in', () => {
    expect(PATCH_MIN_PX).toBe(36)
    expect(PATCH_MAX_PX).toBe(72)
    expect(patchWorldSize(1) / 1).toBeCloseTo(36, 9) // 0.5 m would be 0.5 px
    expect(patchWorldSize(0.0001) / 0.0001).toBeCloseTo(72, 9) // 0.5 m would be 5000 px
  })

  it('a degenerate scale keeps the nominal size', () => {
    expect(patchWorldSize(0)).toBe(PATCH_SIZE_M)
    expect(patchWorldSize(Number.NaN)).toBe(PATCH_SIZE_M)
  })
})

describe('DrawPlaneCueLayer', () => {
  const patch = (over: Partial<Extract<PlaneCueSpec, { kind: 'patch' }>> = {}): PlaneCueSpec =>
    ({ kind: 'patch', pip: 'green', plane: GREEN, through: AT, held: false, ...over })

  it('update(null) leaves the group empty', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update(null)
    expect(layer.group.children).toHaveLength(0)
    expect(layer.current).toBeNull()
  })

  it('the small patch: one fat line set, 2×2 cells (three lines each way), depth-test off, at the hover', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update(patch())
    expect(layer.group.children).toHaveLength(1)
    const line = lineOf(layer)
    expect(line).toBeInstanceOf(LineSegments2)
    expect(materialOf(layer).depthTest).toBe(false)
    expect(materialOf(layer).transparent).toBe(true)
    // Six segments: three parallel to u, three parallel to v.
    const start = line.geometry.getAttribute('instanceStart')
    expect(start.count).toBe(6)
    expect(line.position.toArray()).toEqual(AT)
    expect(layer.patchSize).toBeCloseTo(PATCH_SIZE_M, 12)
    expect(layer.current).toEqual({ kind: 'patch', normal: GREEN.normal, through: AT, held: false })
  })

  it('no 4 m grid: the patch spans exactly its own side, centered on the hover', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update(patch())
    const line = lineOf(layer)
    line.updateMatrixWorld(true)
    const start = line.geometry.getAttribute('instanceStart')
    const end = line.geometry.getAttribute('instanceEnd')
    const pts: THREE.Vector3[] = []
    for (let i = 0; i < start.count; i++) {
      pts.push(new THREE.Vector3().fromBufferAttribute(start, i).applyMatrix4(line.matrixWorld))
      pts.push(new THREE.Vector3().fromBufferAttribute(end, i).applyMatrix4(line.matrixWorld))
    }
    const box = new THREE.Box3().setFromPoints(pts)
    const size = box.getSize(new THREE.Vector3())
    // In the green plane (y = 0): 0.5 m along x and z, nothing along y.
    expect(size.x).toBeCloseTo(PATCH_SIZE_M, 6)
    expect(size.z).toBeCloseTo(PATCH_SIZE_M, 6)
    expect(size.y).toBeCloseTo(0, 6)
    const center = box.getCenter(new THREE.Vector3())
    expect(center.toArray().map((c) => Math.round(c * 1e9) / 1e9)).toEqual(AT)
  })

  it('weight: 1 px at 0.4 alpha inferred, 1.5 px at 0.6 alpha held', () => {
    expect(PATCH_LINE).toEqual({ widthPx: 1, opacity: 0.4 })
    expect(PATCH_LINE_HELD).toEqual({ widthPx: 1.5, opacity: 0.6 })
    const layer = new DrawPlaneCueLayer()
    layer.update(patch())
    expect(materialOf(layer).linewidth).toBe(1)
    expect(materialOf(layer).opacity).toBeCloseTo(0.4, 9)
    layer.update(patch({ held: true }))
    expect(materialOf(layer).linewidth).toBe(1.5)
    expect(materialOf(layer).opacity).toBeCloseTo(0.6, 9)
    expect(layer.current?.held).toBe(true)
  })

  it('clamps on screen: never under 36 px zoomed out, never over 72 px zoomed in', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update(patch())
    const cam = new THREE.PerspectiveCamera(45, 1.5, 0.01, 1e6)
    const heightPx = 800
    const pxOf = (dist: number): number => {
      cam.position.set(AT[0], AT[1] - dist, AT[2])
      cam.lookAt(AT[0], AT[1], AT[2])
      cam.updateMatrixWorld(true)
      layer.applyScale(cam, heightPx)
      const worldPerPx = (2 * dist * Math.tan((45 * Math.PI) / 360)) / heightPx
      return layer.patchSize! / worldPerPx
    }
    expect(pxOf(1000)).toBeCloseTo(36, 6) // far: 0.5 m would be a speck
    expect(pxOf(0.5)).toBeCloseTo(72, 6) // near: 0.5 m would fill the view
    const mid = pxOf(8)
    expect(mid).toBeGreaterThan(36)
    expect(mid).toBeLessThan(72)
    expect(layer.patchSize).toBeCloseTo(PATCH_SIZE_M, 9)
  })

  it('applyScale is a no-op for the outline', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update({ kind: 'outline', pip: 'green', normal: GREEN.normal, loop: LOOP })
    expect(layer.applyScale(new THREE.PerspectiveCamera(), 800)).toBe(false)
    expect(layer.patchSize).toBeNull()
  })

  it('the outline: the loop closed, 1.5 px, depth-tested, its centroid reported', () => {
    expect(OUTLINE_LINE.widthPx).toBe(1.5)
    const layer = new DrawPlaneCueLayer()
    layer.update({ kind: 'outline', pip: 'green', normal: GREEN.normal, loop: LOOP })
    const line = lineOf(layer)
    expect(line.geometry.getAttribute('instanceStart').count).toBe(LOOP.length)
    expect(materialOf(layer).linewidth).toBe(1.5)
    expect(materialOf(layer).depthTest).toBe(true)
    expect(layer.current).toEqual({ kind: 'outline', normal: GREEN.normal, through: [0.5, 0, 0.5], held: false })
  })

  it('colored by the pip: the axis color, or the custom violet', () => {
    const layer = new DrawPlaneCueLayer()
    // The dark theme is the test environment's resolved theme.
    layer.update(patch({ pip: 'red', plane: axisDrawPlane(0, [0, 0, 0]) }))
    expect(materialOf(layer).color.getHex()).toBe(DARK_AXIS_COLORS[0])
    layer.update(patch({ pip: 'blue', plane: groundDrawPlane() }))
    expect(materialOf(layer).color.getHex()).toBe(DARK_AXIS_COLORS[2])
    const tilted = drawPlaneThrough([0, 0, 0], [Math.SQRT1_2, Math.SQRT1_2, 0])!
    layer.update(patch({ pip: 'custom', plane: tilted }))
    expect(materialOf(layer).color.getHex()).toBe(PLANE_CUSTOM_COLORS.dark)
  })

  it('a second update replaces the previous cue rather than accumulating, and clear() empties it', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update(patch())
    layer.update({ kind: 'outline', pip: 'green', normal: GREEN.normal, loop: LOOP })
    expect(layer.group.children).toHaveLength(1)
    layer.clear()
    expect(layer.group.children).toHaveLength(0)
    expect(layer.current).toBeNull()
  })
})
