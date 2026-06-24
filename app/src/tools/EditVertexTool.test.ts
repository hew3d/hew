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
} = {}): WasmScene {
  return {
    pick_sketch_vertex: vi.fn(() => opts.pick),
    sketch_lines: vi.fn(() => opts.lines ?? new Float64Array([])),
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
