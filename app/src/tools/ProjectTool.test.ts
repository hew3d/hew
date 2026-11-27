/**
 * ProjectTool unit tests: the shape to project, the imprint hover and click,
 * the depth stage (pointer, typed, Alt for follow), commit, Escape keeping
 * the imprint, the retype window, and refusals. A fake scene stands in for wasm.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ProjectTool, nearestOnLine } from './ProjectTool'
import { projectSourceOf } from './projectSource'
import type { NodeRef } from '../panels/treeModel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import type { PlaneUnitsScene } from './planeUnits'

const RAY: Ray = { origin: [3, 3, 5], direction: [0, 0, -1] }
/** Sideways at the region's height: its closest point on the depth line is t = 0.2. */
const SIDE: Ray = { origin: [3, 0.5, 0.8], direction: [-1, 0, 0] }
const SKETCH: NodeRef = { kind: 'sketch', id: 3n }
const RING = new Float32Array([0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1])

function previewMesh() {
  return {
    positions: () => new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
    normals: () => new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]),
    indices: () => new Uint32Array([0, 1, 2]),
    edge_positions: () => new Float32Array(),
    free: vi.fn(),
  }
}

function makeScene(opts: { hit?: boolean; instance?: bigint; imprintThrows?: string; depthThrows?: string; previewThrows?: string } = {}) {
  let generation = 0n
  const depthCalls: unknown[][] = []
  const previewCalls: unknown[][] = []
  const scene = {
    pick_face: () =>
      opts.hit === false
        ? undefined
        : { object: () => 7n, face: () => 2n, instance: () => opts.instance, free: vi.fn() },
    node_parent: () => undefined,
    // The sketch sits above the unit cube, facing +z; the projection runs down.
    sketch_plane: () => new Float64Array([0, 0, 3, 0, 0, 1]),
    object_mesh: () => ({ positions: () => new Float32Array([0, 0, 0, 1, 1, 1]), free: vi.fn() }),
    face_boundary: () => RING,
    history_generation: () => generation,
    undo_depth: () => 1,
    scene_undo: vi.fn(() => {
      generation += 1n
      return { free: vi.fn() }
    }),
    scene_redo: vi.fn(() => ({ free: vi.fn() })),
    project_imprint_preview: vi.fn(() => {
      if (opts.imprintThrows !== undefined) throw new Error(opts.imprintThrows)
      return new Float32Array([0, 0, 1, 1, 0, 1])
    }),
    project_imprint: vi.fn(() => {
      if (opts.imprintThrows !== undefined) throw new Error(opts.imprintThrows)
      generation += 1n
      return new BigUint64Array([2n, 3n])
    }),
    project_depth_preview: vi.fn((...args: unknown[]) => {
      previewCalls.push(args)
      if (opts.previewThrows !== undefined) throw new Error(opts.previewThrows)
      return previewMesh()
    }),
    project_depth: vi.fn((...args: unknown[]) => {
      depthCalls.push(args)
      if (opts.depthThrows !== undefined) throw new Error(opts.depthThrows)
      generation += 1n
      return {
        objects_touched: () => new BigUint64Array([7n]),
        components_touched: () => new BigUint64Array(),
        instances_touched: () => new BigUint64Array(),
        free: vi.fn(),
      }
    }),
  }
  return { scene, depthCalls, previewCalls }
}

function makeTool(selection: NodeRef[] = [SKETCH], opts: Parameters<typeof makeScene>[0] = {}) {
  const { scene, depthCalls, previewCalls } = makeScene(opts)
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new ProjectTool(scene as unknown as WasmScene, preview, selection, onCommit, onToast)
  tool.setFaceEligibility(() => true)
  return { tool, scene, preview, depthCalls, previewCalls, onCommit, onToast }
}

const key = (k: string) => ({ key: k, repeat: false }) as KeyboardEvent
const type = (tool: ProjectTool, text: string) => {
  for (const k of text) tool.onKey(key(k))
}

describe('nearestOnLine', () => {
  it('finds where the pointer ray passes closest to the depth line', () => {
    expect(nearestOnLine([0.5, 0.5, 1], [0, 0, -1], SIDE)).toBeCloseTo(0.2, 9)
  })
  it('gives nothing for a ray parallel to the line', () => {
    expect(nearestOnLine([0.5, 0.5, 1], [0, 0, -1], RAY)).toBeNull()
  })
})

describe('projectSourceOf', () => {
  it('takes a whole sketch with no region list', () => {
    const scene = { plane_units_json: () => '{}' } as unknown as PlaneUnitsScene
    expect(projectSourceOf(scene, [SKETCH])).toEqual({ sketch: 3n, regions: [] })
  })
  it('finds nothing in a selection of solids', () => {
    const scene = { plane_units_json: () => '{}' } as unknown as PlaneUnitsScene
    expect(projectSourceOf(scene, [{ kind: 'object', id: 1n }])).toBeNull()
  })
})

describe('ProjectTool', () => {
  it('asks for a closed shape first', () => {
    const { tool, onToast } = makeTool([])
    expect(tool.statusHint()).toBe('Click the closed shape to project.')
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledWith('Click a closed drawn shape to project, then the object to draw it on.')
  })

  it('with nothing selected, a click picks the shape under the pointer, the next the object; Esc backs up', () => {
    const { tool, scene, preview, onCommit } = makeTool([])
    const shape = { sketch: () => 9n, region: () => 4n, free: vi.fn() }
    const s = scene as unknown as Record<string, unknown>
    s.pick_sketch_region = vi.fn(() => shape)
    s.region_boundary = vi.fn(() => RING)
    tool.onPointerMove(null, RAY)
    expect(preview.children.length).toBeGreaterThan(0)
    tool.onPointerDown(null, RAY)
    expect(scene.project_imprint).not.toHaveBeenCalled()
    expect(tool.statusHint()).toBe('Click the object to draw the shape on it. Esc picks another shape.')
    expect(tool.hasArmedGesture()).toBe(true)

    tool.onKey(key('Escape'))
    expect(tool.statusHint()).toBe('Click the closed shape to project.')
    tool.onPointerMove(null, RAY)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)
    expect(scene.project_imprint_preview).toHaveBeenCalledWith(7n, 9n, BigUint64Array.from([4n]))
    tool.onPointerDown(null, RAY)
    expect(scene.project_imprint).toHaveBeenCalledWith(7n, 9n, BigUint64Array.from([4n]))
    expect(onCommit).toHaveBeenCalled()
    expect(tool.statusHint()).toContain('Depth: straight (Alt for follow)')
  })

  it('a click picks the shape under it without a hover first; an undo or a selection drops it', () => {
    const { tool, scene } = makeTool([])
    const s = scene as unknown as Record<string, unknown>
    s.pick_sketch_region = vi.fn(() => ({ sketch: () => 9n, region: () => 4n, free: vi.fn() }))
    s.region_boundary = vi.fn(() => RING)
    tool.onPointerDown(null, RAY)
    expect(tool.statusHint()).toBe('Click the object to draw the shape on it. Esc picks another shape.')
    tool.disarmRetype()
    expect(tool.statusHint()).toBe('Click the closed shape to project.')
    expect(tool.hasArmedGesture()).toBe(false)
    tool.onPointerDown(null, RAY)
    tool.setSelection([SKETCH])
    expect(tool.hasArmedGesture()).toBe(false)
    tool.onPointerDown(null, RAY)
    expect(scene.project_imprint).toHaveBeenCalledWith(7n, 3n, expect.any(BigUint64Array))
  })

  it('hovering the solid previews the outline; clicking draws it and starts the depth stage', () => {
    const { tool, scene, preview, onCommit } = makeTool()
    tool.onPointerMove(null, RAY)
    expect(scene.project_imprint_preview).toHaveBeenCalledWith(7n, 3n, expect.any(BigUint64Array))
    expect(preview.children.length).toBeGreaterThan(0)
    expect(tool.statusHint()).toBe('Click the object to draw the shape on it.')
    tool.onPointerDown(null, RAY)
    expect(scene.project_imprint).toHaveBeenCalledOnce()
    expect(onCommit).toHaveBeenCalledWith({ objectIds: [7n], componentIds: [], instanceIds: [] })
    expect(tool.statusHint()).toContain('Depth: straight (Alt for follow)')
    expect(tool.capturesKey('5')).toBe(true)
  })

  it('moving along the projection sets the depth and shows a ghost; a click commits it', () => {
    const { tool, previewCalls, depthCalls, onCommit } = makeTool()
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, SIDE)
    const last = previewCalls[previewCalls.length - 1]
    expect(last.slice(0, 2)).toEqual([7n, 3n])
    expect(last[3]).toBeCloseTo(0.2, 9)
    expect(last[4]).toBe(0)
    tool.onPointerDown(null, SIDE)
    expect(depthCalls).toHaveLength(1)
    expect(depthCalls[0][3]).toBeCloseTo(0.2, 9)
    expect(onCommit).toHaveBeenCalledTimes(2)
    // The depth stage is over; the retype window (a new depth by typing) is open.
    expect(tool.statusHint()).toBe('Click the object to draw the shape on it.')
    expect(tool.capturesKey('5')).toBe(true)
  })

  it('Alt switches to follow and back', () => {
    const { tool, previewCalls } = makeTool()
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, SIDE)
    tool.onKey(key('Alt'))
    expect(tool.statusHint()).toContain('Depth: follow the surface (Alt for straight)')
    expect(previewCalls[previewCalls.length - 1][4]).toBe(1)
    tool.onKey(key('Alt'))
    expect(previewCalls[previewCalls.length - 1][4]).toBe(0)
  })

  it('a typed depth wins over the pointer; a negative one embosses', () => {
    const { tool, depthCalls } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '2mm')
    tool.onPointerMove(null, SIDE)
    tool.onKey(key('Enter'))
    expect(depthCalls[0][3]).toBeCloseTo(0.002, 9)

    const again = makeTool()
    again.tool.onPointerDown(null, RAY)
    type(again.tool, '-3mm')
    again.tool.onKey(key('Enter'))
    expect(again.depthCalls[0][3]).toBeCloseTo(-0.003, 9)
  })

  it('refuses to commit before a depth is set', () => {
    const { tool, depthCalls, onToast } = makeTool()
    tool.onPointerDown(null, RAY)
    tool.onKey(key('Enter'))
    expect(depthCalls).toHaveLength(0)
    expect(onToast).toHaveBeenCalledWith('Move or type a depth first.')
  })

  it('Escape in the depth stage keeps the imprint and stops', () => {
    const { tool, depthCalls, scene } = makeTool()
    tool.onPointerDown(null, RAY)
    tool.onKey(key('Escape'))
    expect(depthCalls).toHaveLength(0)
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('disarming (an undo) ends the depth stage, whose imprint it removes', () => {
    const { tool, preview, depthCalls } = makeTool()
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, SIDE)
    expect(tool.hasArmedGesture()).toBe(true)
    tool.disarmRetype()
    expect(tool.hasArmedGesture()).toBe(false)
    expect(tool.capturingInput()).toBe(false)
    expect(preview.children).toHaveLength(0)
    tool.onKey(key('Enter'))
    expect(depthCalls).toHaveLength(0)
  })

  it('typing a new depth after the commit redoes it as one step', () => {
    const { tool, scene, depthCalls } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '2mm')
    tool.onKey(key('Enter'))
    type(tool, '5mm')
    tool.onKey(key('Enter'))
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(depthCalls).toHaveLength(2)
    expect(depthCalls[1][3]).toBeCloseTo(0.005, 9)
  })

  it('says to move the shape when it lines up exactly with an edge', () => {
    const { tool, onToast } = makeTool([SKETCH], { imprintThrows: 'ShapeDegenerate: lines up' })
    tool.onPointerMove(null, RAY)
    expect(tool.statusHint()).toBe('Move the shape slightly; it lines up exactly with an edge.')
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledWith('Move the shape slightly; it lines up exactly with an edge.', 'ShapeDegenerate')
  })

  it('follow-the-surface says nothing about a corner it cannot thicken at the pointer depth, but a typed depth does', () => {
    const { tool, onToast } = makeTool([SKETCH], { previewThrows: 'UnsupportedShellCorner: no' })
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, SIDE)
    tool.onKey(key('Alt'))
    expect(tool.statusHint()).not.toContain("Can't thicken this corner")
    type(tool, '3mm')
    expect(tool.statusHint()).toContain("Can't thicken this corner")
    expect(onToast).not.toHaveBeenCalled()
  })

  it('toasts a refused depth and stays in the depth stage', () => {
    const { tool, onToast } = makeTool([SKETCH], { depthThrows: 'WouldSever: cuts in two' })
    tool.onPointerDown(null, RAY)
    type(tool, '2mm')
    tool.onKey(key('Enter'))
    expect(onToast).toHaveBeenCalledOnce()
    expect(onToast.mock.calls[0][1]).toBe('WouldSever')
    expect(tool.hasArmedGesture()).toBe(true)
  })

  it('refuses a solid inside a component at the top level', () => {
    const { tool, onToast, scene } = makeTool([SKETCH], { instance: 9n })
    tool.setFaceEligibility(null)
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledOnce()
    expect(scene.project_imprint).not.toHaveBeenCalled()
  })
})
