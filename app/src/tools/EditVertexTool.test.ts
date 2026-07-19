import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { EditVertexTool } from './EditVertexTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

// A ray straight down the -Z axis from above the origin (tuple-shaped, as the
// real Viewport supplies — the tool indexes ray.origin[0..2]).
const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, preventDefault: () => {} } as unknown as KeyboardEvent
}

/** A fake `SketchVertexPickJs` returning the seeded handles/position. */
function makePick(sketch: bigint, vertex: bigint, pos: [number, number, number]) {
  return {
    sketch: () => sketch,
    vertex: () => vertex,
    x: () => pos[0],
    y: () => pos[1],
    z: () => pos[2],
    free: vi.fn(),
  }
}

/**
 * Minimal WasmScene stub — only the members EditVertexTool calls. `pick`
 * controls what `pick_sketch_vertex` returns; `moveThrows` makes the commit
 * reject (as the kernel does for a WouldRetopologize drag).
 */
function makeWasmScene(opts: {
  pick?: ReturnType<typeof makePick>
  lines?: Float64Array
  moveThrows?: boolean
  /** `sketch_plane` result for the picked vertex's sketch —
   *  `[px,py,pz,nx,ny,nz]` (default: ground, normal +Z). `undefined`
   *  simulates a stale handle. */
  sketchPlane?: [number, number, number, number, number, number] | undefined
} = {}): WasmScene {
  const sketchPlane = 'sketchPlane' in opts ? opts.sketchPlane : [0, 0, 0, 0, 0, 1]
  return {
    pick_sketch_vertex: vi.fn(() => opts.pick),
    sketch_lines: vi.fn(() => opts.lines ?? new Float64Array([])),
    sketch_plane: vi.fn(() => (sketchPlane !== undefined ? new Float64Array(sketchPlane) : undefined)),
    move_sketch_vertex: vi.fn(() => {
      if (opts.moveThrows) throw new Error('WouldRetopologize: the move would cross or merge sketch geometry')
    }),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new EditVertexTool(scene, preview, onCommit, onToast, onMeasurement)
  return { tool, preview, onCommit, onToast, onMeasurement }
}

describe('EditVertexTool — pick & grab', () => {
  it('grabbing a sketch vertex enters the drag stage and shows a ghost', () => {
    // One incident edge from the picked vertex (1,1,0) to (0,1,0).
    const lines = new Float64Array([1, 1, 0, 0, 1, 0])
    const scene = makeWasmScene({ pick: makePick(7n, 9n, [1, 1, 0]), lines })
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    expect(scene.pick_sketch_vertex).toHaveBeenCalled()
    // Ghost rebuilt from the incident edge.
    expect(preview.children.length).toBeGreaterThan(0)
  })

  it('a pick miss stays idle with no ghost', () => {
    const scene = makeWasmScene({ pick: undefined })
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 5, y: 5, z: 0 }), RAY)
    expect(preview.children).toHaveLength(0)
  })

  it('a stale sketch handle (sketch_plane undefined) is treated as a pick miss', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 9n, [1, 1, 0]), sketchPlane: undefined })
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    expect(scene.sketch_plane).toHaveBeenCalledWith(7n)
    expect(preview.children).toHaveLength(0)
  })
})

// Sketches on any plane (Phase 1, the sketch-planes design §3): the
// drag's destination snap must stay on the picked vertex's own sketch plane
// (queried once at pick time), not resolve to ground and get refused by the
// kernel with PointOffPlane.
describe('EditVertexTool — snapConstraint', () => {
  it('idle: returns null (no plane to offer)', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint()).toBeNull()
  })

  it('after picking a vertex on a rotated sketch, returns that sketch\'s plane', () => {
    const scene = makeWasmScene({
      pick: makePick(7n, 9n, [1, 1, 0]),
      sketchPlane: [0, 0, 0, 1, 0, 0], // vertical sketch, normal +X
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)

    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [1, 0, 0] },
    })
  })

  it('resets to null after commit/cancel', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 9n, [1, 1, 0]) })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    expect(tool.snapConstraint()).not.toBeNull()
    tool.cancel()
    expect(tool.snapConstraint()).toBeNull()
  })
})

describe('EditVertexTool — drag on a rotated sketch', () => {
  it('an on-plane destination commits move_sketch_vertex through the rotated plane', () => {
    // A vertical sketch (normal +X) — the vertex and its destination both
    // lie at x = 0, so a naive ground ([0,0,1]) constraint would have
    // refused nothing here (z is free either way); the meaningful check is
    // that snapConstraint reports the sketch's OWN plane, not [0,0,1], so a
    // caller doing on-plane snapping stays on x = 0 rather than z = 0.
    const lines = new Float64Array([0, 1, 1, 0, 1, 0])
    const scene = makeWasmScene({
      pick: makePick(7n, 9n, [0, 1, 1]),
      lines,
      sketchPlane: [0, 0, 0, 1, 0, 0],
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 1 }), RAY) // grab
    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [1, 0, 0] },
    })
    tool.onPointerDown(makeSnap({ x: 0, y: 1.5, z: 1.5 }), RAY) // place, on-plane (x=0)

    expect(scene.move_sketch_vertex).toHaveBeenCalledWith(7n, 9n, 0, 1.5, 1.5)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('EditVertexTool — commit', () => {
  it('the second click commits move_sketch_vertex at the snapped destination', () => {
    const lines = new Float64Array([1, 1, 0, 0, 1, 0])
    const scene = makeWasmScene({ pick: makePick(7n, 9n, [1, 1, 0]), lines })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // grab
    tool.onPointerDown(makeSnap({ x: 1.4, y: 0.8, z: 0 }), RAY) // place

    expect(scene.move_sketch_vertex).toHaveBeenCalledWith(7n, 9n, 1.4, 0.8, 0)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a click that does not move the vertex is a cancel, not a commit', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 9n, [1, 1, 0]) })
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // grab
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // same spot
    expect(scene.move_sketch_vertex).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('a refused drag toasts the kernel error and does NOT fire onCommit', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 9n, [0, 0, 0]), moveThrows: true })
    const { tool, onCommit, onToast, preview } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // grab
    tool.onPointerDown(makeSnap({ x: 3, y: 1, z: 0 }), RAY) // illegal drag

    expect(scene.move_sketch_vertex).toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    // Reset to idle: ghost cleared.
    expect(preview.children).toHaveLength(0)
  })
})

describe('EditVertexTool — cancel', () => {
  it('Escape clears the ghost and returns to idle', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 9n, [1, 1, 0]) })
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('Escape'))
    expect(preview.children).toHaveLength(0)
  })
})
