/**
 * RevolveTool — profile pick, axis pick (edge or two points, in the profile
 * plane only), angle by pointer or typing, segments, retype.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RevolveTool } from './RevolveTool'
import { segmentsPerTurn } from './arcMath'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { draftRef } from './planeUnits'

// The profile: a 1 x 1 square in the XZ plane (y = 0), from x = 1 to 2.
const RING = [1, 0, 0, 2, 0, 0, 2, 0, 1, 1, 0, 1]
const RAY: Ray = { origin: [1.5, -5, 0.5], direction: [0, 1, 0] }

const meshStub = () => ({
  positions: () => new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
  normals: () => new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]),
  indices: () => new Uint32Array([0, 1, 2]),
  edge_positions: () => new Float32Array([0, 0, 0, 1, 0, 0]),
  free: vi.fn(),
})

function key(k: string): KeyboardEvent {
  return { key: k, ctrlKey: false, metaKey: false } as KeyboardEvent
}
function typeText(tool: RevolveTool, text: string): void {
  for (const c of text) tool.onKey(key(c))
}
const snapAt = (x: number, y: number, z: number, extra: Partial<Snap> = {}): Snap => ({
  x, y, z, kind: 'endpoint', ...extra,
})

// The same square as a drawn region of free sketch 3 (region handle 4), with
// its four edges in one island, as `plane_units_json` lists it.
const SQUARE: Array<[number[], number[]]> = [
  [[1, 0, 0], [2, 0, 0]], [[2, 0, 0], [2, 0, 1]], [[2, 0, 1], [1, 0, 1]], [[1, 0, 1], [1, 0, 0]],
]
const edgeKey = (i: number) => `00000000000000e${i}`
const planeListing = (extrude: { sketch: number; region: number } | null = { sketch: 3, region: 4 }) =>
  JSON.stringify({
    units: [{
      key: '00000000000000a0',
      edges: SQUARE.map(([a, b], i) => ({ key: edgeKey(i), a, b, store: { kind: 'free', sketch: 3 }, curve: null })),
      curves: [],
    }],
    regions: [{
      key: '00000000000000a1',
      outer: SQUARE.map(([a]) => a),
      holes: [],
      over: null,
      extrude,
    }],
  })

function setup(over: Record<string, unknown> = {}, selection: readonly NodeRef[] = []) {
  let gen = 1n
  const scene = {
    pick_sketch_region: vi.fn(() => ({ sketch: () => 3n, region: () => 4n, free: vi.fn() })),
    pick_sketch_region_in_instance: vi.fn(),
    pick_face: vi.fn(),
    plane_units_json: vi.fn(() => planeListing()),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 1, 0])),
    region_boundary: vi.fn(() => new Float32Array(RING)),
    sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    edge_endpoints: vi.fn(),
    revolve_preview: vi.fn((_p: string, _s: string) => meshStub()),
    revolve: vi.fn((_p: string, _s: string) => { gen += 1n; return 50n }),
    revolve_in_instance: vi.fn(() => 51n),
    history_generation: vi.fn(() => gen),
    scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    ...over,
  }
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new RevolveTool(scene as unknown as WasmScene, preview, onCommit, onToast, onMeasurement, selection)
  return { scene, tool, preview, onCommit, onToast, onMeasurement }
}

/** Pick the profile and the Z axis through the origin via a sketch edge. */
function toAngleStage(t: ReturnType<typeof setup>) {
  t.tool.onPointerMove(null, RAY)
  t.tool.onPointerDown(null, RAY) // profile
  const edgeSnap = snapAt(0, 0, 0.5, { kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 2n })
  t.tool.onPointerMove(edgeSnap, RAY)
  t.tool.onPointerDown(edgeSnap, RAY) // axis edge
}

const lastSpec = (scene: ReturnType<typeof setup>['scene']) =>
  JSON.parse((scene.revolve.mock.calls.at(-1) as unknown[])[1] as string)

describe('RevolveTool', () => {
  it('walks pick profile, pick axis, angle, with a hint at each stage', () => {
    const t = setup()
    expect(t.tool.statusHint()).toMatch(/Click a drawn shape or a solid face/)
    t.tool.onPointerMove(null, RAY)
    expect(t.preview.children.length).toBeGreaterThan(0) // hover outline
    t.tool.onPointerDown(null, RAY)
    expect(t.tool.statusHint()).toBe('Click an edge or two points for the axis.')
    const edgeSnap = snapAt(0, 0, 0.5, { kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 2n })
    t.tool.onPointerDown(edgeSnap, RAY)
    expect(t.tool.statusHint()).toBe('Move to set the angle, click to finish, Enter for a full turn.')
    expect(t.scene.revolve_preview).toHaveBeenCalledTimes(1) // the full turn at once
    expect(JSON.parse(t.scene.revolve_preview.mock.calls[0][1]).angle).toBeCloseTo(Math.PI * 2, 12)
  })

  it('sends the profile as a region and the axis from the picked edge', () => {
    const t = setup()
    toAngleStage(t)
    t.tool.onKey(key('Enter'))
    expect(t.scene.revolve).toHaveBeenCalledTimes(1)
    const [profile] = t.scene.revolve.mock.calls[0] as [string, string]
    expect(JSON.parse(profile)).toEqual({ region: { sketch: '3', region: '4' } })
    const spec = lastSpec(t.scene)
    expect(spec.axis_point).toEqual([0, 0, 0])
    expect(spec.axis).toEqual([0, 0, 1])
    expect(spec.angle).toBeCloseTo(Math.PI * 2, 12)
    expect(spec.segments).toBe(segmentsPerTurn(2)) // the largest radius is 2 m
    expect(t.onCommit).toHaveBeenCalledWith(50n)
  })

  it('a two-point axis works: click, click', () => {
    const t = setup()
    t.tool.onPointerDown(null, RAY)
    t.tool.onPointerDown(snapAt(0, 0, 0), RAY)
    expect(t.tool.statusHint()).toBe('Click the second point of the axis.')
    t.tool.onPointerDown(snapAt(0, 0, 2), RAY)
    expect(t.tool.statusHint()).toMatch(/Move to set the angle/)
    t.tool.onKey(key('Enter'))
    expect(lastSpec(t.scene).axis).toEqual([0, 0, 1])
  })

  it('refuses an axis that is not in the profile plane, with the plain reason', () => {
    const t = setup()
    t.tool.onPointerDown(null, RAY)
    const vertical = snapAt(0, 0.5, 0.5, { kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 2n })
    t.scene.sketch_edge_endpoints.mockReturnValueOnce(new Float64Array([0, 0, 0, 0, 1, 0]))
    t.tool.onPointerMove(vertical, RAY)
    expect(t.tool.statusHint()).toBe('The axis has to lie in the plane of the shape. Move the axis onto that plane.')
    t.scene.sketch_edge_endpoints.mockReturnValueOnce(new Float64Array([0, 0, 0, 0, 1, 0]))
    t.tool.onPointerDown(vertical, RAY)
    expect(t.onToast).toHaveBeenCalledWith('The axis has to lie in the plane of the shape. Move the axis onto that plane.')
    expect(t.tool.statusHint()).not.toMatch(/Move to set the angle/)
    // A first point off the plane is refused at once.
    t.tool.onPointerDown(snapAt(0, 1, 0), RAY)
    expect(t.onToast).toHaveBeenCalledTimes(2)
  })

  it('the pointer sets the angle, snapping to 15 degree steps; a click finishes there', () => {
    const t = setup()
    toAngleStage(t)
    // Axis Z through (0,0,0); profile toward +X; looking down -Z.
    const at = (x: number, y: number): Ray => ({ origin: [x, y, 10], direction: [0, 0, -1] })
    t.tool.onPointerMove(null, at(0, 1)) // 90 degrees counterclockwise
    t.tool.onPointerDown(null, at(0, 1))
    expect(lastSpec(t.scene).angle).toBeCloseTo(Math.PI / 2, 9)
    expect(t.onCommit).toHaveBeenCalledWith(50n)
  })

  it('typed 90 / 90° / 90deg finish at that angle, and 24s sets segments first', () => {
    for (const text of ['90', '90°', '90deg']) {
      const t = setup()
      toAngleStage(t)
      typeText(t.tool, text)
      t.tool.onKey(key('Enter'))
      expect(lastSpec(t.scene).angle).toBeCloseTo(Math.PI / 2, 12)
    }
    const t = setup()
    toAngleStage(t)
    typeText(t.tool, '24s')
    t.tool.onKey(key('Enter'))
    expect(t.scene.revolve).not.toHaveBeenCalled() // still adjusting
    expect(JSON.parse(t.scene.revolve_preview.mock.calls.at(-1)![1]).segments).toBe(24)
    typeText(t.tool, '180')
    t.tool.onKey(key('Enter'))
    const spec = lastSpec(t.scene)
    expect(spec.segments).toBe(24)
    expect(spec.angle).toBeCloseTo(Math.PI, 12)
  })

  it('shows the angle and the typed text in the measurements box', () => {
    const t = setup()
    toAngleStage(t)
    expect(t.onMeasurement).toHaveBeenLastCalledWith('360°')
    typeText(t.tool, '45')
    expect(t.onMeasurement).toHaveBeenLastCalledWith('45°')
  })

  it('a kernel refusal toasts its copy and keeps the gesture alive', () => {
    const t = setup({
      revolve: vi.fn(() => { throw new Error('ProfileCrossesAxis: the profile crosses the axis') }),
    })
    toAngleStage(t)
    t.tool.onKey(key('Enter'))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onToast.mock.calls[0][1]).toBe('ProfileCrossesAxis')
    expect(t.onCommit).not.toHaveBeenCalled()
    expect(t.tool.statusHint()).toMatch(/Move to set the angle/)
  })

  it('a refused ghost explains why in the status line', () => {
    const t = setup({
      revolve_preview: vi.fn(() => { throw new Error('ProfileCrossesAxis: crosses') }),
    })
    toAngleStage(t)
    expect(t.tool.statusHint()).not.toMatch(/Move to set the angle/)
    expect(t.tool.statusHint().length).toBeGreaterThan(10)
  })

  it('Esc steps back one stage at a time', () => {
    const t = setup()
    toAngleStage(t)
    t.tool.onKey(key('Escape'))
    expect(t.tool.statusHint()).toBe('Click an edge or two points for the axis.')
    t.tool.onKey(key('Escape'))
    expect(t.tool.statusHint()).toMatch(/Click a drawn shape/)
    expect(t.preview.children.length).toBe(0)
  })

  it('after the commit, a typed angle or segment count redoes the revolve', () => {
    const t = setup()
    toAngleStage(t)
    t.tool.onKey(key('Enter')) // full turn
    expect(t.tool.capturesKey('9')).toBe(true)
    expect(t.tool.capturesKey('m')).toBe(false)
    typeText(t.tool, '90°')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(lastSpec(t.scene).angle).toBeCloseTo(Math.PI / 2, 12)
    typeText(t.tool, '32s')
    t.tool.onKey(key('Enter'))
    const spec = lastSpec(t.scene)
    expect(spec.segments).toBe(32)
    expect(spec.angle).toBeCloseTo(Math.PI / 2, 12)
    t.tool.disarmRetype()
    expect(t.tool.capturesKey('9')).toBe(false)
  })

  it('a solid face is a profile too, sent as a face', () => {
    const facePick = { object: () => 7n, face: () => 2n, instance: () => undefined, depth: () => 1, free: vi.fn() }
    const t = setup({
      pick_sketch_region: vi.fn(() => undefined),
      pick_face: vi.fn(() => facePick),
      face_boundary: vi.fn(() => new Float32Array(RING)),
      face_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 1, 0])),
      node_parent: vi.fn(() => undefined),
    })
    toAngleStage(t)
    t.tool.onKey(key('Enter'))
    expect(JSON.parse(t.scene.revolve.mock.calls[0][0])).toEqual({ face: { object: '7', face: '2' } })
  })

  it('inside a component, a face goes through revolve_in_instance', () => {
    const facePick = { object: () => 7n, face: () => 2n, instance: () => 9n, depth: () => 1, free: vi.fn() }
    const t = setup({
      pick_sketch_region_in_instance: vi.fn(() => undefined),
      pick_face: vi.fn(() => facePick),
      face_boundary: vi.fn(() => new Float32Array(RING)),
      face_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 1, 0])),
      instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])),
    })
    t.tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })
    toAngleStage(t)
    t.tool.onKey(key('Enter'))
    expect(t.scene.revolve_in_instance).toHaveBeenCalledTimes(1)
    expect(t.scene.revolve_in_instance.mock.calls[0].slice(0, 3)).toEqual([9n, 7n, 2n])
    expect(t.scene.revolve_preview).not.toHaveBeenCalled()
  })

  it('a click over nothing says what to aim at, once', () => {
    const t = setup({ pick_sketch_region: vi.fn(() => undefined), pick_face: vi.fn(() => undefined) })
    t.tool.onPointerDown(null, RAY)
    t.tool.onPointerDown(null, RAY)
    expect(t.onToast).toHaveBeenCalledTimes(1)
  })
})

describe('RevolveTool — refusals and edges', () => {
  // The kernel's domain: an angle that is nonzero and at most a full turn,
  // 3 to 1024 segments. A fake kernel that enforces it lets the tool's own
  // refusal path run.
  const refusing = () => (_p: string, s: string): bigint => {
    const spec = JSON.parse(s)
    if (spec.angle === 0 || Math.abs(spec.angle) > Math.PI * 2 + 1e-6 || spec.segments < 3 || spec.segments > 1024) {
      throw new Error('AmountOutOfRange: the amount is outside the allowed range')
    }
    return 50n
  }
  const at = (x: number, y: number): Ray => ({ origin: [x, y, 10], direction: [0, 0, -1] })

  it('a segment count outside 3 to 1024 is refused and the stage stays armed', () => {
    for (const text of ['2s', '1025s']) {
      const t = setup({ revolve: vi.fn(refusing()) })
      toAngleStage(t)
      typeText(t.tool, text)
      t.tool.onKey(key('Enter'))
      expect(t.onToast).toHaveBeenCalledTimes(1)
      expect(t.onToast.mock.calls[0][0]).toMatch(/3 to 1024/)
      expect(t.onCommit).not.toHaveBeenCalled()
      expect(t.tool.statusHint()).toMatch(/Move to set the angle/)
      // The refused count is not kept: a full turn still takes the default facets.
      t.tool.onKey(key('Enter'))
      expect(lastSpec(t.scene).segments).toBe(segmentsPerTurn(2))
      expect(t.onCommit).toHaveBeenCalledWith(50n)
    }
  })

  it('an angle beyond a full turn is refused, the stage stays armed', () => {
    const t = setup({ revolve: vi.fn(refusing()) })
    toAngleStage(t)
    typeText(t.tool, '400')
    t.tool.onKey(key('Enter'))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onCommit).not.toHaveBeenCalled()
    expect(t.tool.statusHint()).toMatch(/Move to set the angle/)
    // The stage still takes a good angle afterward.
    typeText(t.tool, '90')
    t.tool.onKey(key('Enter'))
    expect(lastSpec(t.scene).angle).toBeCloseTo(Math.PI / 2, 12)
    expect(t.onCommit).toHaveBeenCalledWith(50n)
  })

  it('a typed segment count is confirmed in the status line until the next click; a refused retype confirms nothing', () => {
    const t = setup({ revolve: vi.fn(refusing()) })
    toAngleStage(t)
    typeText(t.tool, '24s')
    t.tool.onKey(key('Enter'))
    expect(t.tool.statusHint()).toBe('Segments set to 24.')
    // The Viewport re-sends the same ray after a key: not a move.
    t.tool.onPointerMove(null, RAY)
    expect(t.tool.statusHint()).toBe('Segments set to 24.')
    // A click commits and the hint moves on with the stage.
    t.tool.onPointerDown(null, RAY)
    expect(t.onCommit).toHaveBeenCalledWith(50n)
    expect(t.tool.statusHint()).not.toMatch(/Segments set to/)
    // In the retype window, a count the kernel refuses toasts and confirms nothing.
    t.scene.revolve.mockImplementation(() => {
      throw new Error('AmountOutOfRange: the amount is outside the allowed range')
    })
    typeText(t.tool, '48s')
    t.tool.onKey(key('Enter'))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.tool.statusHint()).not.toMatch(/Segments set to/)
  })

  it('a pointer reading within 3 degrees of a 15 degree step takes the step, and no farther', () => {
    const t = setup()
    toAngleStage(t)
    // The profile lies toward +X about the Z axis; a point at atan(y / 10) degrees.
    const deg = (d: number) => at(10, 10 * Math.tan((d * Math.PI) / 180))
    t.tool.onPointerMove(null, deg(32.9)) // 2.9 from 30
    t.tool.onPointerDown(null, deg(32.9))
    expect(lastSpec(t.scene).angle).toBeCloseTo((30 * Math.PI) / 180, 9)

    const u = setup()
    toAngleStage(u)
    u.tool.onPointerMove(null, deg(33.2)) // 3.2 from 30, 11.8 from 45: no step
    u.tool.onPointerDown(null, deg(33.2))
    expect(lastSpec(u.scene).angle).toBeCloseTo((33.2 * Math.PI) / 180, 9)
  })

  it('a reading just past the profile direction snaps to a full turn, never a zero angle', () => {
    const t = setup()
    toAngleStage(t)
    t.tool.onPointerMove(null, at(10, 0.0349)) // about 0.2 degrees
    t.tool.onPointerDown(null, at(10, 0.0349))
    expect(lastSpec(t.scene).angle).toBeCloseTo(Math.PI * 2, 9)
  })

  it('a reading just short of the profile direction also reads as a full turn', () => {
    const t = setup()
    toAngleStage(t)
    t.tool.onPointerMove(null, at(10, -0.0349)) // about 359.8 degrees
    t.tool.onPointerDown(null, at(10, -0.0349))
    expect(lastSpec(t.scene).angle).toBeCloseTo(Math.PI * 2, 9)
  })

  it('a click commits once; the double-click that follows adds nothing and the second press is swallowed', () => {
    const t = setup()
    toAngleStage(t)
    // The Viewport skips the second press of a double-click for a tool that has onDoubleClick.
    expect('onDoubleClick' in t.tool).toBe(true)
    t.tool.onPointerDown(null, at(0, 1))
    expect(t.scene.revolve).toHaveBeenCalledTimes(1)
    expect(t.tool.onDoubleClick()).toBe(false)
    expect(t.scene.revolve).toHaveBeenCalledTimes(1)
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('a stale retype (the model changed since) closes the window with a toast and touches nothing', () => {
    const t = setup()
    toAngleStage(t)
    t.tool.onKey(key('Enter'))
    t.scene.history_generation.mockReturnValue(999n) // something else happened
    typeText(t.tool, '45')
    t.tool.onKey(key('Enter'))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onToast.mock.calls[0][0]).toMatch(/Dimension entry ended/)
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.scene.revolve).toHaveBeenCalledTimes(1)
    expect(t.tool.capturesKey('9')).toBe(false)
  })

  describe('preselection', () => {
    const region = draftRef('draft-region', 'ctx', '00000000000000a1')
    const island = draftRef('draft-island', 'ctx', '00000000000000a0')
    const oneEdge = draftRef('draft-edge', 'ctx', edgeKey(0))
    const AXIS_HINT = 'Click an edge or two points for the axis.'
    const PICK_HINT = /Click a drawn shape or a solid face/

    it('a selected drawn region skips straight to the axis stage, with its outline shown', () => {
      const t = setup({}, [region])
      t.tool.activate()
      expect(t.tool.statusHint()).toBe(AXIS_HINT)
      expect(t.preview.children.length).toBeGreaterThan(0)
      expect(t.scene.pick_sketch_region).not.toHaveBeenCalled() // no profile click needed
    })

    it('revolves that region once the axis is picked (profile handles come from the listing)', () => {
      const t = setup({}, [region])
      const edgeSnap = snapAt(0, 0, 0.5, { kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 2n })
      t.tool.onPointerDown(edgeSnap, RAY)
      expect(t.tool.statusHint()).toMatch(/Move to set the angle/)
      t.tool.onKey(key('Enter'))
      const [profile] = t.scene.revolve.mock.calls[0] as [string, string]
      expect(JSON.parse(profile)).toEqual({ region: { sketch: '3', region: '4' } })
    })

    it('a selected island whose outline is exactly one closed region is the profile too', () => {
      expect(setup({}, [island]).tool.statusHint()).toBe(AXIS_HINT)
    })

    it('Esc from the preselected axis stage backs up to the profile pick', () => {
      const t = setup({}, [region])
      t.tool.onKey(key('Escape'))
      expect(t.tool.statusHint()).toMatch(PICK_HINT)
    })

    it('a preselected region inside a group context still starts at the axis stage', () => {
      const t = setup({}, [region])
      t.tool.setEditContext({ kind: 'group', id: 5n }) // the Viewport's priming call
      expect(t.tool.statusHint()).toBe(AXIS_HINT)
    })

    it('a context change before any gesture keeps the preselected profile', () => {
      const t = setup({}, [region])
      t.tool.setEditContext({ kind: 'group', id: 5n })
      t.tool.setEditContext({ kind: 'top' })
      expect(t.tool.statusHint()).toBe(AXIS_HINT)
    })

    it('a context change after the real gesture began still cancels', () => {
      const t = setup({}, [region])
      t.tool.setEditContext({ kind: 'group', id: 5n })
      const edgeSnap = snapAt(0, 0, 0.5, { kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 2n })
      t.tool.onPointerDown(edgeSnap, RAY) // axis picked: now at the angle stage
      expect(t.tool.statusHint()).toMatch(/Move to set the angle/)
      t.tool.setEditContext({ kind: 'top' })
      expect(t.tool.statusHint()).toMatch(PICK_HINT)
    })

    it('a selected solid does not preselect', () => {
      const t = setup({}, [{ kind: 'object', id: 7n }])
      expect(t.tool.statusHint()).toMatch(PICK_HINT)
    })

    it('two selections do not preselect', () => {
      const t = setup({}, [region, { kind: 'object', id: 7n }])
      expect(t.tool.statusHint()).toMatch(PICK_HINT)
      const two = setup({}, [region, island])
      expect(two.tool.statusHint()).toMatch(PICK_HINT)
    })

    it('a lone edge, or a region over a solid (no free-sketch handle), does not preselect', () => {
      expect(setup({}, [oneEdge]).tool.statusHint()).toMatch(PICK_HINT)
      const over = setup({ plane_units_json: vi.fn(() => planeListing(null)) }, [region])
      expect(over.tool.statusHint()).toMatch(PICK_HINT)
    })

    it('a stale selection (the listing no longer has the region) does not preselect', () => {
      const stale = draftRef('draft-region', 'ctx', '00000000000000ff')
      expect(setup({}, [stale]).tool.statusHint()).toMatch(PICK_HINT)
    })
  })
})
