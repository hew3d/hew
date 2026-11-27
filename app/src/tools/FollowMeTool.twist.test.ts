/**
 * Follow Me twist, end scale and stations (true-curves.md §5.2): typed
 * tokens set the options, the `_with` twins carry them, closed paths are
 * refused before the call, options reset per gesture, and the post-commit
 * retype window redoes the sweep.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { FollowMeTool } from './FollowMeTool'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { requiresPreview } from '../test/requiresPreview'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function key(k: string, extra: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return { key: k, ctrlKey: false, metaKey: false, ...extra } as KeyboardEvent
}

function typeText(tool: FollowMeTool, text: string): void {
  for (const c of text) tool.onKey(key(c))
}

interface Opts {
  edgePath?: boolean
  commitError?: string
}

function makeScene(o: Opts = {}) {
  let gen = 1n
  let next = 77n
  const impl = (): bigint => {
    if (o.commitError !== undefined) throw new Error(o.commitError)
    gen += 1n
    next += 1n
    return next
  }
  const scene = {
    pick_sketch_edge: vi.fn(() => (o.edgePath === false ? undefined : { sketch: () => 9n, edge: () => 1n, free: vi.fn() })),
    pick_face: vi.fn(() => ({ object: () => 30n, face: () => 31n, depth: () => 1, instance: () => undefined, free: vi.fn() })),
    pick_sketch_region: vi.fn(() => ({ sketch: () => 20n, region: () => 21n, free: vi.fn() })),
    pick_sketch_region_in_instance: vi.fn(),
    sketch_edge_island: vi.fn(() => 5n),
    sketch_island_edges: vi.fn(() => new BigUint64Array([1n])),
    sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
    sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0])),
    face_boundary: vi.fn(() => new Float32Array([0, 0, 1, 1, 0, 1, 1, 1, 1])),
    node_parent: vi.fn(() => undefined),
    history_generation: vi.fn(() => gen),
    scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    follow_me_along_edges: vi.fn(impl),
    follow_me_along_edges_with: vi.fn(impl),
    follow_me_around_face: vi.fn(impl),
    follow_me_around_face_with: vi.fn(impl),
  }
  return scene
}

function setup(o: Opts = {}) {
  const scene = makeScene(o)
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new FollowMeTool(scene as unknown as WasmScene, new THREE.Group(), onCommit, onToast, vi.fn(), [])
  return { scene, tool, onCommit, onToast }
}

describe('FollowMeTool — twist, end scale and stations', () => {
  // Only what the twist option drives is marked; the plain-sweep tests below
  // are about behavior a production build has too.
  const needsTwist = (ctx: Parameters<typeof requiresPreview>[0]) => requiresPreview(ctx, 'follow-me-twist')
  it('a typed twist and end size reach the _with twin as radians and a ratio', (ctx) => {
    needsTwist(ctx)
    const { scene, tool, onCommit } = setup()
    tool.onPointerDown(null, RAY) // path (edge island)
    typeText(tool, '30°')
    tool.onKey(key('Enter'))
    typeText(tool, '40%')
    tool.onKey(key('Enter'))
    typeText(tool, '8s')
    tool.onKey(key('Enter'))
    expect(tool.statusHint()).toContain('Twist 30° · End 40% · 8 stations')
    tool.onPointerDown(null, RAY) // profile → commit
    expect(scene.follow_me_along_edges).not.toHaveBeenCalledWith(expect.anything())
    const call = (scene.follow_me_along_edges_with as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[6]).toBeCloseTo(Math.PI / 6, 12)
    expect(call[7]).toBeCloseTo(0.4, 12)
    expect(call[8]).toBe(8)
    expect(onCommit).toHaveBeenCalledWith(78n)
  })

  it('the profile stage offers a twist and end size, typed as 90deg or 90d', (ctx) => {
    needsTwist(ctx)
    const { scene, tool } = setup()
    tool.onPointerDown(null, RAY) // path
    expect(tool.statusHint()).toContain('Type 30deg to twist it or 40% to shrink its end.')
    typeText(tool, '90d')
    tool.onKey(key('Enter'))
    expect(tool.statusHint()).toContain('(Twist 90°)')
    expect(tool.statusHint()).not.toContain('Type 30deg')
    tool.onPointerDown(null, RAY) // profile → commit
    const call = (scene.follow_me_along_edges_with as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[6]).toBeCloseTo(Math.PI / 2, 12)
  })

  it('plain Follow Me stays on the plain call', () => {
    const { scene, tool } = setup()
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_along_edges).toHaveBeenCalledTimes(1)
    expect(tool.statusHint()).not.toContain('Twist')
  })

  it('options reset at the start of the next gesture', (ctx) => {
    needsTwist(ctx)
    const { tool } = setup()
    tool.onPointerDown(null, RAY)
    typeText(tool, '30°')
    tool.onKey(key('Enter'))
    expect(tool.statusHint()).toContain('Twist 30°')
    tool.onKey(key('Escape')) // clears the twist, keeps the path
    expect(tool.statusHint()).not.toContain('Twist')
    tool.onKey(key('Escape')) // back to the path stage
    tool.onPointerDown(null, RAY)
    expect(tool.statusHint()).not.toContain('Twist')
  })

  it('a twist on a face-loop path is refused before any call, with the open-path copy', (ctx) => {
    needsTwist(ctx)
    const { scene, tool, onToast } = setup({ edgePath: false })
    tool.onPointerDown(null, RAY) // face loop path
    typeText(tool, '30°')
    tool.onKey(key('Enter'))
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_around_face_with).not.toHaveBeenCalled()
    expect(onToast.mock.calls[0][0]).toMatch(/open path/i)
    expect(onToast.mock.calls[0][1]).toBe('TwistNeedsOpenPath')
  })

  it('a twist on a closed path says so before the click, and Escape clears it but keeps the path', (ctx) => {
    needsTwist(ctx)
    const { scene, tool } = setup({ edgePath: false })
    tool.onPointerDown(null, RAY) // face loop path
    expect(tool.statusHint()).not.toContain('is closed')
    typeText(tool, '30deg')
    tool.onKey(key('Enter'))
    expect(tool.statusHint()).toContain('(Twist 30°)')
    expect(tool.statusHint()).toContain('This path is closed')
    tool.onKey(key('Escape'))
    expect(tool.statusHint()).not.toContain('Twist')
    expect(tool.statusHint()).toContain('highlighted path') // still at the profile stage
    // A plain sweep around the kept path goes through.
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_around_face).toHaveBeenCalledTimes(1)
    expect(scene.follow_me_around_face_with).not.toHaveBeenCalled()
  })

  it('an open path takes a twist with no warning', (ctx) => {
    needsTwist(ctx)
    const { tool } = setup()
    tool.onPointerDown(null, RAY)
    typeText(tool, '30deg')
    tool.onKey(key('Enter'))
    expect(tool.statusHint()).not.toContain('is closed')
  })

  it('a typed plain length still commits a partial sweep', () => {
    const { scene, tool } = setup()
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY) // hover the profile
    typeText(tool, '0.5')
    tool.onKey(key('Enter'))
    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[4]).toBeCloseTo(0.5, 9)
  })

  it('after the commit, a typed twist redoes the sweep through the retype window', (ctx) => {
    needsTwist(ctx)
    const { scene, tool, onCommit } = setup()
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY) // plain commit → 78
    expect(tool.capturesKey('4')).toBe(true) // a digit opens the window
    expect(tool.capturesKey('m')).toBe(false) // letters keep their global meaning
    typeText(tool, '45°')
    expect(tool.capturesKey('Enter')).toBe(true)
    tool.onKey(key('Enter'))
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    const call = (scene.follow_me_along_edges_with as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[6]).toBeCloseTo(Math.PI / 4, 12)
    expect(call[7]).toBe(1)
    expect(onCommit).toHaveBeenLastCalledWith(79n)
    // The status bar says what the retype did, until the pointer moves.
    expect(tool.statusHint()).toBe('Twist set to 45°.')
    // A second entry keeps the first option.
    typeText(tool, '50%')
    tool.onKey(key('Enter'))
    const second = (scene.follow_me_along_edges_with as ReturnType<typeof vi.fn>).mock.calls[1]
    expect(second[6]).toBeCloseTo(Math.PI / 4, 12)
    expect(second[7]).toBeCloseTo(0.5, 12)
    expect(tool.statusHint()).toBe('End size set to 50%.')
    tool.onPointerMove(null, { origin: [9, 9, 9], direction: [0, 0, -1] })
    expect(tool.statusHint()).not.toContain('set to')
  })

  it('a refused retype keeps the original sweep and tells why', (ctx) => {
    needsTwist(ctx)
    const { scene, tool, onToast } = setup()
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    ;(scene.follow_me_along_edges_with as ReturnType<typeof vi.fn>).mockImplementationOnce(() => {
      throw new Error('SweepSelfIntersects: no')
    })
    typeText(tool, '90°')
    tool.onKey(key('Enter'))
    expect(onToast).toHaveBeenCalled()
    expect(scene.scene_redo).toHaveBeenCalledTimes(1)
  })

  it('a click, Escape or explicit undo ends the retype window', (ctx) => {
    needsTwist(ctx)
    const { tool } = setup()
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    tool.onKey(key('Escape'))
    expect(tool.capturesKey('4')).toBe(false)
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    tool.disarmRetype()
    expect(tool.capturesKey('4')).toBe(false)
  })
  it('a zero end size is refused with a toast and sets nothing', (ctx) => {
    needsTwist(ctx)
    const { scene, tool, onToast } = setup()
    tool.onPointerDown(null, RAY) // path
    typeText(tool, '0%')
    tool.onKey(key('Enter'))
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/above 0%/)
    expect(tool.statusHint()).not.toContain('End')
    tool.onPointerDown(null, RAY) // profile -> plain commit
    expect(scene.follow_me_along_edges_with).not.toHaveBeenCalled()
    expect(scene.follow_me_along_edges).toHaveBeenCalledTimes(1)
  })

  it('a retype keeps the other option and the stations count', (ctx) => {
    needsTwist(ctx)
    const { scene, tool } = setup()
    tool.onPointerDown(null, RAY)
    typeText(tool, '8s')
    tool.onKey(key('Enter'))
    tool.onPointerDown(null, RAY) // commit with 8 stations
    typeText(tool, '30°')
    tool.onKey(key('Enter'))
    const calls = (scene.follow_me_along_edges_with as ReturnType<typeof vi.fn>).mock.calls
    expect(calls).toHaveLength(2) // the 8-station commit, then the retype
    expect(calls[1][6]).toBeCloseTo(Math.PI / 6, 12)
    expect(calls[1][7]).toBe(1)
    expect(calls[1][8]).toBe(8)
  })

  it('digits half-typed in the retype window do not leak into the next gesture', (ctx) => {
    needsTwist(ctx)
    const { scene, tool, onCommit } = setup()
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY) // plain commit
    expect(onCommit).toHaveBeenCalledTimes(1)
    typeText(tool, '4') // half-typed, no Enter
    tool.onPointerDown(null, RAY) // the next path: the window and its buffer end
    tool.onKey(key('Enter')) // at the profile stage with nothing typed
    expect(scene.follow_me_along_edges).toHaveBeenCalledTimes(1) // no 4 m sweep from the stale "4"
    expect(scene.follow_me_along_edges_with).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(scene.scene_undo).not.toHaveBeenCalled()
  })
})
