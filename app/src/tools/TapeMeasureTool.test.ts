/**
 * TapeMeasureTool unit tests — fake-WasmScene pattern like ArcTool.test.ts.
 * Focused on first-click mode selection: an edge snap (world Object OR
 * committed sketch edge) enters parallel-guide mode and commits
 * `add_guide_line`; anything else falls to measure mode.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { TapeMeasureTool } from './TapeMeasureTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeWasmScene() {
  const guideLines: number[][] = []
  const guidePoints: number[][] = []
  const scene = {
    // Both endpoint queries return a unit-X edge from (0,0,0) to (2,0,0).
    edge_endpoints: vi.fn((_o: bigint, _e: bigint) => new Float64Array([0, 0, 0, 2, 0, 0])),
    sketch_edge_endpoints: vi.fn((_s: bigint, _e: bigint) => new Float64Array([0, 0, 0, 2, 0, 0])),
    add_guide_line: vi.fn((ox: number, oy: number, oz: number, dx: number, dy: number, dz: number) => {
      guideLines.push([ox, oy, oz, dx, dy, dz])
    }),
    add_guide_point: vi.fn((x: number, y: number, z: number) => {
      guidePoints.push([x, y, z])
    }),
    // No committed sketches under the cursor in these fixtures (plane-lock /
    // sketch-hover-adopt is covered separately in TapeMeasureTool.plane.test.ts).
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => undefined),
  }
  return { scene: scene as unknown as WasmScene, guideLines, guidePoints }
}

function makeTool(scene: WasmScene) {
  const onGuideCreated = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new TapeMeasureTool(scene, new THREE.Group(), onGuideCreated, onToast, onMeasurement)
  return { tool, onGuideCreated, onToast, onMeasurement }
}

describe('TapeMeasureTool — parallel-guide mode entry', () => {
  it('an object-edge snap enters parallel mode and commits a guide line', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY) // pull sideways
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY) // commit

    expect(guideLines.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    // Direction is the edge direction (+X).
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })

  it('a SKETCH-edge snap enters parallel mode too (a rectangle sketch edge is the common case)', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 42n, element: 5n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY)

    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(0) // NOT the old bug: no stray guide point
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    const wasm = scene as unknown as { sketch_edge_endpoints: ReturnType<typeof vi.fn> }
    expect(wasm.sketch_edge_endpoints).toHaveBeenCalledWith(42n, 5n)
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })

  it('a consumed/stale sketch edge (endpoints undefined) falls back to measure mode', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    ;(scene as unknown as { sketch_edge_endpoints: ReturnType<typeof vi.fn> })
      .sketch_edge_endpoints.mockReturnValue(undefined)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 42n, element: 5n }),
      RAY,
    )
    // Second click in empty space: measure mode drops a guide point.
    tool.onPointerDown(makeSnap({ x: 3, y: 3, z: 0, kind: 'ground' }), RAY)

    expect(guideLines.length).toBe(0)
    expect(guidePoints.length).toBe(1)
  })

  it('a plain ground click (no edge provenance) stays in measure mode', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 3, z: 0, kind: 'ground' }), RAY)

    expect(guideLines.length).toBe(0)
    expect(guidePoints.length).toBe(1)
  })
})

describe('TapeMeasureTool — parallel guides from axes and guide lines (playtest: guide off an axis)', () => {
  it('an on-axis snap enters parallel mode and commits a guide parallel to the axis', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    // The kernel resolves the axis ANALYTICALLY: the snap carries the
    // on-line point and the axis direction, no element handle at all.
    tool.onPointerDown(
      makeSnap({ x: 2, y: 0, z: 0, kind: 'on-axis', direction: [1, 0, 0] }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 2, y: 1, z: 0 }), RAY) // pull off the axis
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 0 }), RAY) // commit

    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(0)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0]) // parallel to the red axis
    expect([ox, oy, oz]).toEqual([2, 1, 0]) // through the pulled-to point
  })

  it('a typed exact offset commits the axis-parallel guide at that distance', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0.05, y: 0, z: 0, kind: 'on-axis', direction: [1, 0, 0] }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 0.05, y: 0.005, z: 0 }), RAY) // cm-scale pull
    const key = (k: string) => ({ key: k, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent
    for (const k of ['0', '.', '0', '2']) tool.onKey(key(k))
    tool.onKey(key('Enter'))

    expect(guideLines.length).toBe(1)
    const [ox, oy, oz, dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
    expect(ox).toBeCloseTo(0.05, 12)
    expect(oy).toBeCloseTo(0.02, 12) // exactly 2 cm off the axis
    expect(oz).toBeCloseTo(0, 12)
  })

  it('an on-guide snap sources a parallel guide from the existing guide line', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 2, z: 0, kind: 'on-guide', direction: [0, 1, 0] }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1.5, y: 2, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 0 }), RAY)

    expect(guideLines.length).toBe(1)
    const [ox, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([0, 1, 0])
    expect(ox).toBeCloseTo(1.5, 12)
  })

  it('an on-axis snap WITHOUT a direction falls back to measure mode (no throw)', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'on-axis' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 1, z: 0, kind: 'ground' }), RAY)

    expect(guideLines.length).toBe(0)
    expect(guidePoints.length).toBe(1) // measure ended in empty space → point
  })

  it('the measure flow still drops a guide point in empty space', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 1, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 0, kind: 'ground' }), RAY)

    expect(guidePoints).toEqual([[2, 1, 0]])
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
  })
})
