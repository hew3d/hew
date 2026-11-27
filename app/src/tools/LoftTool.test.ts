/**
 * LoftTool — picking shapes in order, taking them back, the apex, rails,
 * the ghost, typed steps, start grips, and refusals.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LoftTool } from './LoftTool'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import type { Snap } from './types'

const meshStub = () => ({
  positions: () => new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
  normals: () => new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]),
  indices: () => new Uint32Array([0, 1, 2]),
  edge_positions: () => new Float32Array([0, 0, 0, 1, 0, 0]),
  free: vi.fn(),
})

const SQUARE = [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0]
const rayAt = (x: number): Ray => ({ origin: [x, 0, 5], direction: [0, 0, -1] })
function key(k: string): KeyboardEvent {
  return { key: k, ctrlKey: false, metaKey: false } as KeyboardEvent
}

/** Regions are chosen by the ray's x: 1 -> region 11, 2 -> region 12. */
function setup(over: Record<string, unknown> = {}) {
  const scene = {
    pick_sketch_region: vi.fn((ox: number) =>
      ox === 1 || ox === 2 ? { sketch: () => 7n, region: () => BigInt(10 + ox), free: vi.fn() } : undefined),
    pick_sketch_region_in_instance: vi.fn(),
    pick_face: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    region_boundary: vi.fn(() => new Float32Array(SQUARE)),
    pick_sketch_edge: vi.fn(() => undefined),
    sketch_edge_island: vi.fn(),
    sketch_island_edges: vi.fn(),
    sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0])),
    loft_preview: vi.fn((_j: string) => meshStub()),
    loft: vi.fn((_j: string) => 60n),
    loft_start_points: vi.fn((_j: string) => new Float64Array([4, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 4, 0, 0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 1])),
    ...over,
  }
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new LoftTool(scene as unknown as WasmScene, preview, onCommit, onToast)
  return { scene, tool, preview, onCommit, onToast }
}
const lastSpec = (fn: ReturnType<typeof vi.fn>) => JSON.parse(fn.mock.calls.at(-1)![0] as string)

describe('LoftTool', () => {
  it('starts with the plain-words hint', () => {
    const t = setup()
    expect(t.tool.statusHint()).toBe(
      'Click profiles in order. Click a point to end at an apex. Alt-click a path to add a rail. Enter to loft.',
    )
    expect(t.tool.capturingInput()).toBe(false)
  })

  it('previews after two shapes and lofts them in click order on Enter', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(2))
    expect(t.scene.loft_preview).not.toHaveBeenCalled()
    t.tool.onPointerDown(null, rayAt(1))
    expect(t.scene.loft_preview).toHaveBeenCalled()
    expect(lastSpec(t.scene.loft_preview).sources).toEqual([
      { region: { sketch: '7', region: '12' } },
      { region: { sketch: '7', region: '11' } },
    ])
    t.tool.onKey(key('Enter'))
    expect(t.scene.loft).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenCalledWith(60n)
    expect(t.tool.capturingInput()).toBe(false)
  })

  it('clicking a picked shape again takes it out; Esc takes back the latest, then cancels', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.onPointerDown(null, rayAt(1)) // shape 1 comes back out
    t.tool.onPointerDown(null, rayAt(2)) // and so does 2
    expect(t.tool.capturingInput()).toBe(false)
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.onKey(key('Escape')) // takes back shape 2
    expect(t.tool.capturingInput()).toBe(true)
    t.tool.onKey(key('Escape')) // takes back shape 1
    expect(t.tool.capturingInput()).toBe(false)
  })

  it('Esc clears a half-typed value first and leaves the picks alone', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.onKey(key('8'))
    t.tool.onKey(key('Escape')) // clears the 8 only
    t.tool.onKey(key('Enter'))
    expect(t.scene.loft).toHaveBeenCalledTimes(1)
    expect(lastSpec(t.scene.loft).sources).toHaveLength(2)
  })

  it('a click on a vertex point ends the loft at an apex; no more shapes after it', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    const apex: Snap = { x: 0.5, y: 0.5, z: 2, kind: 'endpoint' }
    t.tool.onPointerMove(apex, rayAt(9))
    expect(t.tool.statusHint()).toMatch(/end the loft at an apex/)
    t.tool.onPointerDown(apex, rayAt(9))
    expect(lastSpec(t.scene.loft_preview).sources[1]).toEqual({ vertex: [0.5, 0.5, 2] })
    t.tool.onPointerDown(null, rayAt(2))
    expect(t.onToast.mock.calls.at(-1)![0]).toMatch(/already ends at a point/)
  })

  it('a click on an open drawn chain says the shape must be closed', () => {
    const t = setup({
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 3n, edge: () => 1n, free: vi.fn() })),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n])),
    })
    t.tool.onPointerDown(null, rayAt(9))
    expect(t.onToast).toHaveBeenCalledWith('Loft needs closed shapes. Close this one first.')
    expect(t.scene.loft_preview).not.toHaveBeenCalled()
  })

  it('Alt-click adds a rail (at most two) that goes into the spec', () => {
    const t = setup({
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 3n, edge: () => 1n, free: vi.fn() })),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n, 2n])),
    })
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.setAltModifier(true)
    t.tool.onPointerDown(null, rayAt(9))
    expect(lastSpec(t.scene.loft_preview).rails).toEqual([{ edges: { sketch: '3', edges: ['1', '2'] } }])
    t.tool.onPointerDown(null, rayAt(9))
    t.tool.onPointerDown(null, rayAt(9))
    expect(t.onToast).toHaveBeenCalledWith('A loft can follow at most two paths.')
  })

  it('typed 8s sets the stations and keeps going', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    expect(t.tool.capturesKey('8')).toBe(true)
    expect(t.tool.capturesKey('s')).toBe(false) // a bare letter is still a shortcut
    for (const c of '8s') t.tool.onKey(key(c))
    t.tool.onKey(key('Enter'))
    expect(t.scene.loft).not.toHaveBeenCalled()
    expect(lastSpec(t.scene.loft_preview).stations).toBe(8)
    t.tool.onKey(key('Enter'))
    expect(lastSpec(t.scene.loft).stations).toBe(8)
  })

  it('dragging a start grip to another corner sets that shape\'s start offset', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    // The first shape's grip sits at its first start point (0,0,0).
    t.tool.onPointerDown(null, { origin: [0, 0, 5], direction: [0, 0, -1] })
    expect(t.tool.statusHint()).toMatch(/Drag the grip/)
    t.tool.onPointerMove(null, { origin: [1, 1, 5], direction: [0, 0, -1] }) // nearest corner (1,1)
    t.tool.onPointerUp()
    expect(lastSpec(t.scene.loft_preview).start_offsets).toEqual([2, 0])
  })

  it('a kernel refusal while previewing shows in the status line', () => {
    const t = setup({ loft_preview: vi.fn(() => { throw new Error('SelfIntersecting: crossing') }) })
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    expect(t.tool.statusHint()).toMatch(/pass through itself/)
  })

  it('too few shapes on Enter says so; a refused loft toasts and keeps the picks', () => {
    const t = setup({ loft: vi.fn(() => { throw new Error('OpenProfile: open') }) })
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onKey(key('Enter'))
    expect(t.onToast.mock.calls[0][0]).toMatch(/at least two/)
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.onKey(key('Enter'))
    expect(t.onToast.mock.calls.at(-1)![1]).toBe('OpenProfile')
    expect(t.tool.capturingInput()).toBe(true)
  })
})

describe('LoftTool — refusals and grips', () => {
  const vertex: Snap = { x: 0.5, y: 0.5, z: 2, kind: 'endpoint' }
  /** A rail per x: a different chain of drawn edges each time. */
  const railScene = () => {
    let chain = 0n
    return setup({
      // The picked edge is one of its own chain's, as in a real scene.
      pick_sketch_edge: vi.fn(() => { chain += 1n; return { sketch: () => 3n, edge: () => chain * 10n, free: vi.fn() } }),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([chain * 10n, chain * 10n + 1n])),
    })
  }

  it('an apex before any profile is refused: nothing picked, nothing previewed', () => {
    const t = setup()
    t.tool.onPointerMove(vertex, rayAt(9))
    expect(t.tool.statusHint()).not.toMatch(/end the loft at an apex/)
    t.tool.onPointerDown(vertex, rayAt(9))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalledWith('Click the shapes first, then the point to end at.')
    expect(t.scene.loft_preview).not.toHaveBeenCalled()
    expect(t.tool.capturingInput()).toBe(false)
    t.tool.onKey(key('Enter'))
    expect(t.scene.loft).not.toHaveBeenCalled()
  })

  it('a point clicked on an open chain before any shape still says shapes come first', () => {
    const t = setup({
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 3n, edge: () => 1n, free: vi.fn() })),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n])),
    })
    t.tool.onPointerDown(vertex, rayAt(9))
    expect(t.onToast).toHaveBeenCalledWith('Click the shapes first, then the point to end at.')
    expect(t.onToast.mock.calls[0][0]).not.toMatch(/closed shapes/)
  })

  it('a shape with a hole is refused at the pick, with the hole copy and code', () => {
    // Region 11 is a ring (a hole inside); region 12 is solid. The count
    // comes from the kernel region itself, so a ring drawn on a solid's
    // face (no extrude entry in the plane listing) is refused the same way.
    const holeCount = vi.fn((_s: bigint, r: bigint) => (r === 11n ? 1 : 0))
    const t = setup({ region_hole_count: holeCount })
    t.tool.onPointerDown(null, rayAt(1))
    expect(holeCount).toHaveBeenCalledWith(7n, 11n)
    expect(t.onToast).toHaveBeenCalledWith(
      'Loft needs shapes without holes. Shell the lofted solid to hollow it.',
      'LoftProfileHasHole',
    )
    // Nothing was picked: the next solid shape is the first one.
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.onKey(key('Enter'))
    expect(t.onToast.mock.calls.at(-1)![0]).toMatch(/at least two/)
    expect(t.scene.loft_preview).not.toHaveBeenCalled()
  })

  it('a solid face with a hole through it is refused at the pick, with the hole copy and code', () => {
    // A box with a circle pushed through the top: the top face keeps a hole.
    const faces: Record<string, number> = { '31': 1, '32': 0 }
    const holeCount = vi.fn((_o: bigint, f: bigint) => faces[String(f)] ?? 0)
    const t = setup({
      pick_sketch_region: vi.fn(() => undefined),
      pick_face: vi.fn((ox: number) => ({
        object: () => 5n,
        face: () => BigInt(30 + ox),
        instance: () => undefined,
        free: vi.fn(),
      })),
      face_boundary: vi.fn(() => new Float32Array(SQUARE)),
      face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
      face_hole_count: holeCount,
    })
    t.tool.setFaceEligibility(() => true)
    t.tool.onPointerDown(null, rayAt(1)) // top face, one hole
    expect(holeCount).toHaveBeenCalledWith(5n, 31n)
    expect(t.onToast).toHaveBeenCalledWith(
      'Loft needs shapes without holes. Shell the lofted solid to hollow it.',
      'LoftProfileHasHole',
    )
    // Nothing was picked: a hole-free face is the first one.
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.onKey(key('Enter'))
    expect(t.onToast.mock.calls.at(-1)![0]).toMatch(/at least two/)
    expect(t.scene.loft_preview).not.toHaveBeenCalled()
  })

  it('a third rail toasts and is not added', () => {
    const t = railScene()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.setAltModifier(true)
    t.tool.onPointerDown(null, rayAt(9)) // chain 10, 11
    t.tool.onPointerDown(null, rayAt(9)) // chain 20, 21
    expect(t.onToast).not.toHaveBeenCalled()
    expect(lastSpec(t.scene.loft_preview).rails).toHaveLength(2)
    t.tool.onPointerDown(null, rayAt(9)) // a third
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalledWith('A loft can follow at most two paths.')
    expect(lastSpec(t.scene.loft_preview).rails).toEqual([
      { edges: { sketch: '3', edges: ['10', '11'] } },
      { edges: { sketch: '3', edges: ['20', '21'] } },
    ])
  })

  it('a rail with nothing under the cursor says what to aim at, once', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.setAltModifier(true)
    t.tool.onPointerDown(null, rayAt(9))
    t.tool.onPointerDown(null, rayAt(9))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalledWith('Alt-click a drawn line or curve to add a rail.')
    expect(lastSpec(t.scene.loft_preview).rails).toEqual([])
  })

  it('a second grip drag sets the new start corner and leaves the other shape alone', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    const at = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
    // First drag: the first shape's grip from (0,0) to (1,1), start offset 2.
    t.tool.onPointerDown(null, at(0, 0))
    t.tool.onPointerMove(null, at(1, 1))
    t.tool.onPointerUp()
    expect(lastSpec(t.scene.loft_preview).start_offsets).toEqual([2, 0])
    // The grip now sits at (1,1); a second drag to (0,1) lands on corner 3.
    t.tool.onPointerDown(null, at(1, 1))
    t.tool.onPointerMove(null, at(0, 1))
    t.tool.onPointerUp()
    expect(lastSpec(t.scene.loft_preview).start_offsets).toEqual([3, 0])
    // And back to the first corner.
    t.tool.onPointerDown(null, at(0, 1))
    t.tool.onPointerMove(null, at(0, 0))
    t.tool.onPointerUp()
    expect(lastSpec(t.scene.loft_preview).start_offsets).toEqual([0, 0])
  })

  it('a rail picked before the shapes is taken off, with a toast, when a later shape misses it', () => {
    const railEdge = {
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 3n, edge: () => 1n, free: vi.fn() })),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n, 2n])),
    }
    // The rail passes through region 11 but misses region 12.
    const loft_preview = vi.fn((j: string) => {
      const spec = JSON.parse(j)
      const hasRail = spec.rails.length > 0
      if (hasRail && JSON.stringify(spec.sources).includes('"12"')) throw new Error('RailMissesProfile: nope')
      return meshStub()
    })
    const t = setup({ ...railEdge, loft_preview })
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.setAltModifier(true)
    t.tool.onPointerDown(null, rayAt(9)) // rail, with one shape: nothing to check against yet
    t.tool.setAltModifier(false)
    expect(t.onToast).not.toHaveBeenCalled()
    t.tool.onPointerDown(null, rayAt(2)) // this shape misses the rail
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onToast.mock.calls[0][0]).toMatch(/taken off the loft/)
    expect(t.onToast.mock.calls[0][1]).toBe('RailMissesProfile')
    // The rail is gone, the loft previews and the start grips are back.
    expect(lastSpec(loft_preview).rails).toEqual([])
    expect(t.tool.statusHint()).not.toMatch(/RailMisses|nope/)
    // Esc takes back the shape, not a rail that is no longer there.
    t.tool.onKey(key('Escape'))
    t.tool.onPointerDown(null, rayAt(1)) // shape 1 comes back out
    expect(t.tool.capturingInput()).toBe(false)
  })

  it('start offsets dragged before a rail was picked are not sent with the rail', () => {
    const t = setup({
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 3n, edge: () => 1n, free: vi.fn() })),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n, 2n])),
    })
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    t.tool.onPointerDown(null, { origin: [0, 0, 5], direction: [0, 0, -1] })
    t.tool.onPointerMove(null, { origin: [1, 1, 5], direction: [0, 0, -1] })
    t.tool.onPointerUp()
    expect(lastSpec(t.scene.loft_preview).start_offsets).toEqual([2, 0])
    t.tool.setAltModifier(true)
    t.tool.onPointerDown(null, rayAt(9))
    expect(lastSpec(t.scene.loft_preview).rails).toHaveLength(1)
    expect(lastSpec(t.scene.loft_preview).start_offsets).toEqual([0, 0])
    t.tool.onKey(key('Enter'))
    expect(lastSpec(t.scene.loft).start_offsets).toEqual([0, 0])
  })

  it('a click on a grip without a drag leaves the offsets alone', () => {
    const t = setup()
    t.tool.onPointerDown(null, rayAt(1))
    t.tool.onPointerDown(null, rayAt(2))
    const at = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
    t.tool.onPointerDown(null, at(0, 0))
    t.tool.onPointerUp()
    expect(lastSpec(t.scene.loft_preview).start_offsets).toEqual([0, 0])
    expect(t.onToast).not.toHaveBeenCalled()
  })
})
