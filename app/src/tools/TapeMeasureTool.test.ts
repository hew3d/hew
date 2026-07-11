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
