/**
 * Reference lines on the drawing plane (wireframe-faces-and-solids design §3,
 * stage 2): the shared module, then real tool gestures on the draft mock with
 * `enforcePlane`.
 *
 * Kurt's case: a ground Rectangle ABCD, a vertical Rectangle on the plane
 * through its edge AB (`←` lock), anchored at A. B is its own foot, so the
 * vertical through B is offered, and the cursor above B snaps to it: the new
 * Rectangle is exactly as wide as AB.
 */
import { describe, it, expect, vi, afterEach } from 'vitest'
import * as THREE from 'three'
import { RectangleTool } from './RectangleTool'
import { LineTool } from './LineTool'
import { CircleTool } from './CircleTool'
import { PolygonTool } from './PolygonTool'
import { ArcTool } from './ArcTool'
import { draftMock } from './draftMock'
import {
  FROM_POINT_FALLBACK_TOLERANCE_M,
  MAX_OWNER_POINTS,
  REFERENCE_DWELL_MS,
  REFERENCE_RING_SIZE,
  ReferenceLines,
  footOnPlane,
  ownerPoints,
  planeDirections,
  referenceLines,
  resolveReferenceSnap,
  type ReferencePlane,
} from './referenceLines'
import { WORLD_DRAWING_AXES } from './drawingAxes'
import { CueLayer } from '../viewport/CueLayer'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

type V3 = [number, number, number]

const HOME: V3 = [3.5, -3, 2.5]
const noop = (): void => { /* no-op */ }

function cameraAt(eye: V3): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
  cam.position.set(...eye)
  cam.up.set(0, 0, 1)
  cam.lookAt(0, 0, 0)
  cam.updateMatrixWorld(true)
  return cam
}

function rayThrough(p: V3, eye: V3 = HOME): Ray {
  const d: V3 = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]]
  const len = Math.hypot(...d)
  return { origin: [...eye], direction: [d[0] / len, d[1] / len, d[2] / len] }
}

function snap(p: V3, kind = 'endpoint', extra: Partial<Snap> = {}): Snap {
  return { x: p[0], y: p[1], z: p[2], kind, ...extra }
}

function key(k: string): KeyboardEvent {
  return { key: k, repeat: false, preventDefault: noop } as unknown as KeyboardEvent
}

// A ground Rectangle with awkward, real coordinates. Its edge AB runs along x.
const A: V3 = [0.3, 0.2, 0]
const B: V3 = [0.9, 0.2, 0]
const C: V3 = [0.9, 0.8, 0]
const D: V3 = [0.3, 0.8, 0]
const GROUND_SKETCH = 7n

function makeScene(opts: { ownerPoints?: (() => Float64Array) | null } = {}) {
  const dm = draftMock({ enforcePlane: true })
  const owner = opts.ownerPoints === undefined ? () => new Float64Array([...A, ...B, ...C, ...D]) : opts.ownerPoints
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => undefined),
    face_plane: vi.fn(() => undefined),
    face_normal: vi.fn(() => undefined),
    ...dm.members,
    pick_sketch_region: vi.fn(() => undefined),
    pick_sketch_region_in_instance: vi.fn(() => undefined),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
    ...(owner === null ? {} : { snap_owner_points: vi.fn(owner) }),
  }
  return { scene: scene as unknown as WasmScene, raw: scene, log: dm.log }
}

function rectTool(scene: WasmScene) {
  const tool = new RectangleTool(scene, new THREE.Group(), noop, noop, noop)
  tool.updateCamera(cameraAt(HOME))
  return tool
}

type Clicker = { snapConstraint(r: Ray): unknown; onPointerMove(s: Snap | null, r: Ray): void; onPointerDown(s: Snap | null, r: Ray): void }
function click(tool: Clicker, s: Snap): void {
  const ray = rayThrough([s.x, s.y, s.z])
  tool.snapConstraint(ray)
  tool.onPointerMove(s, ray)
  tool.onPointerDown(s, ray)
}
function hover(tool: Clicker, s: Snap): void {
  const ray = rayThrough([s.x, s.y, s.z])
  tool.snapConstraint(ray)
  tool.onPointerMove(s, ray)
}

/** Start Kurt's vertical Rectangle at A on the plane y = 0.2 (`←`). */
function startVerticalAtA(tool: RectangleTool): void {
  tool.onKey(key('ArrowLeft'))
  click(tool, snap(A, 'endpoint', { sketch: GROUND_SKETCH, element: 1n, elementKind: 'sketch-edge' }))
}

describe('reference lines — the module', () => {
  const plane: ReferencePlane = { origin: [0, 0.2, 0], normal: [0, 1, 0], u: [1, 0, 0], v: [0, 0, 1] }

  it('the foot of a point on the plane is that point, bit for bit; off the plane it is the perpendicular', () => {
    const on = footOnPlane(B, plane)
    expect(on.off).toBe(false)
    expect(on.foot).toBe(B)
    const off = footOnPlane(C, plane)
    expect(off.off).toBe(true)
    expect(off.foot[0]).toBe(0.9)
    expect(off.foot[1]).toBeCloseTo(0.2, 15)
    expect(off.foot[2]).toBe(0)
  })

  it('the directions on an axis plane are the frame axes lying in it', () => {
    const [d1, d2] = planeDirections(plane, WORLD_DRAWING_AXES)
    expect([d1, d2]).toEqual([[1, 0, 0], [0, 0, 1]])
  })

  it('feet de-duplicate: off-plane corners over the same foot make one pair of lines', () => {
    // ABCD on the plane y = 0.2 through AB: C and D foot onto B and A. A is
    // the anchor (the engine offers its axis lines), so only B's foot remains.
    const lines = referenceLines([A, B, C, D], plane, planeDirections(plane, WORLD_DRAWING_AXES), A)
    expect(lines).toHaveLength(2)
    expect(lines.every((l) => l.foot[0] === B[0] && l.foot[1] === B[1] && l.foot[2] === B[2])).toBe(true)
    // Without a skipped anchor, A's pair appears too — still just two feet.
    expect(referenceLines([A, B, C, D], plane, planeDirections(plane, WORLD_DRAWING_AXES), null)).toHaveLength(4)
    // A point on the plane beats an off-plane point with the same foot.
    const merged = referenceLines([C, B], plane, [[1, 0, 0]], null)
    expect(merged).toHaveLength(1)
    expect(merged[0].source).toBeUndefined()
  })

  it('a point above the plane keeps its source for the tie line', () => {
    const lines = referenceLines([[0.5, 0.9, 0.4]], plane, [[1, 0, 0]], null)
    expect(lines[0].foot[0]).toBe(0.5)
    expect(lines[0].foot[1]).toBeCloseTo(0.2, 15)
    expect(lines[0].foot[2]).toBe(0.4)
    expect(lines[0].source).toEqual([0.5, 0.9, 0.4])
  })

  const fixedTolerance = (): number => 0.02
  const frame = (): typeof WORLD_DRAWING_AXES => WORLD_DRAWING_AXES

  it('the crossing of two reference lines snaps to the exact crossing', () => {
    // Ground plane; P1 offers x = 0.25 and y = 0.5, P2 offers x = 0.75 and y = 0.125.
    const ground: ReferencePlane = { origin: [0, 0, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0] }
    const hit = resolveReferenceSnap({
      snap: { x: 0.752, y: 0.498, z: 0, kind: 'ground' },
      cursor: [0.752, 0.498, 0], plane: ground, anchor: [0, 0, 0], lockDir: null, frame,
      points: [[0.25, 0.5, 0], [0.75, 0.125, 0]], tolerance: fixedTolerance,
    })
    expect(hit).not.toBeNull()
    expect(hit!.point).toEqual([0.75, 0.5, 0])
    expect(hit!.snap.kind).toBe('from-point')
    expect(hit!.snap.fromAlso).toBeDefined()
  })

  it('a reference line crosses an axis line from the anchor', () => {
    const ground: ReferencePlane = { origin: [0, 0, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0] }
    const hit = resolveReferenceSnap({
      snap: null, cursor: [0.503, 0.004, 0], plane: ground, anchor: [0, 0, 0], lockDir: null, frame,
      points: [[0.5, 0.75, 0]], tolerance: fixedTolerance,
    })
    expect(hit!.point).toEqual([0.5, 0, 0])
  })

  it('a real endpoint under the cursor beats a reference line', () => {
    const ground: ReferencePlane = { origin: [0, 0, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0] }
    const input = {
      cursor: [0.5, 0.004, 0] as V3, plane: ground, anchor: [0, 0, 0] as V3, lockDir: null, frame,
      points: [[0.5, 0.75, 0]] as V3[], tolerance: fixedTolerance,
    }
    expect(resolveReferenceSnap({ ...input, snap: { x: 0.5, y: 0.004, z: 0, kind: 'endpoint' } })).toBeNull()
    expect(resolveReferenceSnap({ ...input, snap: { x: 0.5, y: 0.004, z: 0, kind: 'on-edge' } })).toBeNull()
    // The weak kinds give way.
    for (const kind of ['plane', 'ground', 'on-face', 'on-axis']) {
      expect(resolveReferenceSnap({ ...input, snap: { x: 0.5, y: 0.004, z: 0, kind } })).not.toBeNull()
    }
  })

  it('out of tolerance, nothing is offered', () => {
    const ground: ReferencePlane = { origin: [0, 0, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0] }
    const hit = resolveReferenceSnap({
      snap: null, cursor: [0.6, 0.3, 0], plane: ground, anchor: [0, 0, 0], lockDir: null, frame,
      points: [[0.5, 0.75, 0]], tolerance: fixedTolerance,
    })
    expect(hit).toBeNull()
  })

  it('the fallback tolerance applies before any camera feed', () => {
    expect(new ReferenceLines().tolerance([1, 2, 3])).toBe(FROM_POINT_FALLBACK_TOLERANCE_M)
  })
})

describe('reference lines — the dwell ring', () => {
  const p = (i: number): Snap => snap([i, 0, 0.5])

  it('a point joins after 300 ms of resting, not before', () => {
    const refs = new ReferenceLines()
    refs.observeHover(p(1), 1000)
    refs.observeHover(p(1), 1000 + REFERENCE_DWELL_MS - 1)
    expect(refs.dwellPoints).toHaveLength(0)
    refs.observeHover(p(1), 1000 + REFERENCE_DWELL_MS)
    expect(refs.dwellPoints).toEqual([[1, 0, 0.5]])
  })

  it('leaving a point after resting on it counts, since a resting cursor sends no events', () => {
    const refs = new ReferenceLines()
    refs.observeHover(p(1), 0)
    refs.observeHover(null, 900)
    expect(refs.dwellPoints).toEqual([[1, 0, 0.5]])
  })

  it('leaving early does not count, and neither do kinds that are not named points', () => {
    const refs = new ReferenceLines()
    refs.observeHover(p(1), 0)
    refs.observeHover(p(2), 299)
    refs.observeHover(null, 598)
    expect(refs.dwellPoints).toHaveLength(0)
    refs.observeHover(snap([1, 1, 1], 'on-edge'), 1000)
    refs.observeHover(null, 2000)
    expect(refs.dwellPoints).toHaveLength(0)
  })

  it('the ring holds four: the oldest falls out', () => {
    const refs = new ReferenceLines()
    let t = 0
    for (let i = 1; i <= REFERENCE_RING_SIZE + 2; i++) {
      refs.observeHover(p(i), t)
      t += REFERENCE_DWELL_MS + 1
      refs.observeHover(p(i), t)
      t += 1
    }
    expect(REFERENCE_RING_SIZE).toBe(4)
    expect(refs.dwellPoints).toEqual([[3, 0, 0.5], [4, 0, 0.5], [5, 0, 0.5], [6, 0, 0.5]])
  })

  it('clearRing forgets the points and a dwell in progress', () => {
    const refs = new ReferenceLines()
    refs.observeHover(p(1), 0)
    refs.observeHover(p(1), 500)
    refs.observeHover(p(2), 600)
    refs.clearRing()
    refs.observeHover(null, 5000)
    expect(refs.dwellPoints).toHaveLength(0)
  })
})

describe('reference lines — the start shape\'s corners (snap_owner_points)', () => {
  const owned = snap(A, 'endpoint', { sketch: 7n, element: 3n, elementKind: 'sketch-edge' })

  it('is absent from the pkg: no corners, no throw', () => {
    expect(ownerPoints({}, owned, A)).toEqual([])
  })

  it('a snap with no owner asks nothing', () => {
    const fn = vi.fn(() => new Float64Array([1, 2, 3]))
    expect(ownerPoints({ snap_owner_points: fn }, snap(A), A)).toEqual([])
    expect(fn).not.toHaveBeenCalled()
  })

  it('passes the owner through and returns the corners nearest the anchor first, at most 64', () => {
    const flat: number[] = []
    for (let i = 0; i < 100; i++) flat.push(100 - i, 0, 0)
    const fn = vi.fn(() => new Float64Array(flat))
    const pts = ownerPoints({ snap_owner_points: fn }, snap(A, 'endpoint', { object: 5n, element: 9n, elementKind: 'vertex' }), [0, 0, 0])
    expect(fn).toHaveBeenCalledWith(undefined, 5n, 9n, 'vertex', undefined)
    expect(pts).toHaveLength(MAX_OWNER_POINTS)
    expect(pts[0]).toEqual([1, 0, 0])
    expect(pts[63]).toEqual([64, 0, 0])
  })

  it('passes the placing instance as the fifth argument', () => {
    const fn = vi.fn(() => new Float64Array([1, 2, 3]))
    ownerPoints({ snap_owner_points: fn }, snap(A, 'endpoint', { object: 5n, instance: 11n, element: 9n, elementKind: 'vertex' }), A)
    expect(fn).toHaveBeenCalledWith(undefined, 5n, 9n, 'vertex', 11n)
  })

  it('a stale handle that makes the binding throw degrades to no corners', () => {
    const fn = vi.fn(() => { throw new Error('UnknownSketch') })
    expect(ownerPoints({ snap_owner_points: fn }, owned, A)).toEqual([])
  })
})

describe('reference lines — a plain sketch corner resolves its owner by pick (R15)', () => {
  /** A `pick_sketch_vertex` result at `at`, as the binding returns it. */
  function picked(at: V3, sketch = 7n, vertex = 4n) {
    return { sketch: () => sketch, vertex: () => vertex, x: () => at[0], y: () => at[1], z: () => at[2], free: vi.fn() }
  }
  const ray = rayThrough(A)

  it('an ownerless endpoint snap whose picked vertex is that point yields its island corners', () => {
    const hit = picked(A)
    const pick = vi.fn(() => hit)
    const fn = vi.fn(() => new Float64Array([...A, ...B, ...C, ...D]))
    const pts = ownerPoints({ pick_sketch_vertex: pick, snap_owner_points: fn }, snap(A, 'endpoint', { instance: 11n }), A, ray)
    expect(pick).toHaveBeenCalledWith(...ray.origin, ...ray.direction)
    expect(fn).toHaveBeenCalledWith(7n, undefined, 4n, 'sketch-vertex', 11n)
    expect(pts).toEqual([A, B, D, C])
    expect(hit.free).toHaveBeenCalled()
  })

  it('a picked vertex elsewhere (a guide point over empty space) yields none', () => {
    const pick = vi.fn(() => picked([A[0] + 0.01, A[1], A[2]]))
    const fn = vi.fn(() => new Float64Array([...A, ...B]))
    expect(ownerPoints({ pick_sketch_vertex: pick, snap_owner_points: fn }, snap(A), A, ray)).toEqual([])
    expect(fn).not.toHaveBeenCalled()
  })

  it('no ray, no pick binding, a throwing pick, or a non-point snap: none, no throw', () => {
    const fn = vi.fn(() => new Float64Array([...A, ...B]))
    const pick = vi.fn(() => picked(A))
    expect(ownerPoints({ pick_sketch_vertex: pick, snap_owner_points: fn }, snap(A), A)).toEqual([])
    expect(ownerPoints({ snap_owner_points: fn }, snap(A), A, ray)).toEqual([])
    const throws = vi.fn(() => { throw new Error('boom') })
    expect(ownerPoints({ pick_sketch_vertex: throws, snap_owner_points: fn }, snap(A), A, ray)).toEqual([])
    expect(ownerPoints({ pick_sketch_vertex: pick, snap_owner_points: fn }, snap(A, 'on-face'), A, ray)).toEqual([])
    expect(fn).not.toHaveBeenCalled()
  })

  it("Kurt's case from a plain corner: the vertical from B is offered with no dwell", () => {
    const { scene, raw } = makeScene()
    Object.assign(raw, { pick_sketch_vertex: vi.fn(() => picked(A)) })
    const tool = rectTool(scene)
    tool.onKey(key('ArrowLeft'))
    click(tool, snap(A)) // an ownerless endpoint, as the engine gives a plain corner
    expect(raw.snap_owner_points).toHaveBeenCalledWith(7n, undefined, 4n, 'sketch-vertex', undefined)
    hover(tool, snap([B[0] + 0.003, B[1], 0.7], 'plane'))
    const cue = tool.lastSnap
    expect(cue?.kind).toBe('from-point')
    expect(cue?.from).toEqual(B)
  })
})

describe("Kurt's case — a vertical Rectangle above the far corner of a ground Rectangle", () => {
  afterEach(() => { vi.restoreAllMocks() })

  /** Hover a hair beside the vertical through B at height 0.7, then click. */
  function drawWidthAboveB(tool: RectangleTool): void {
    const near = snap([B[0] + 0.003, B[1], 0.7], 'plane')
    hover(tool, near)
    click(tool, near)
  }

  it('the vertical from B is offered: the cue is a from-point snap with B as its foot', () => {
    const { scene } = makeScene()
    const tool = rectTool(scene)
    startVerticalAtA(tool)
    hover(tool, snap([B[0] + 0.003, B[1], 0.7], 'plane'))
    const cue = tool.lastSnap
    expect(cue).not.toBeNull()
    expect(cue!.kind).toBe('from-point')
    expect(cue!.from).toEqual(B)
    expect(cue!.fromSource).toBeUndefined() // B is on the plane
    expect(cue!.direction).toEqual([0, 0, 1])
    expect([cue!.x, cue!.y]).toEqual([B[0], B[1]])
  })

  it("the committed Rectangle's width equals |AB| to the bit", () => {
    const { scene, log } = makeScene()
    const tool = rectTool(scene)
    startVerticalAtA(tool)
    drawWidthAboveB(tool)
    expect(log.segments).toHaveLength(4)
    const xs = log.segments.flatMap((s) => [s[0], s[3]])
    const width = Math.max(...xs) - Math.min(...xs)
    expect(width).toBe(B[0] - A[0])
    // The rectangle builds its far edge as anchor + extent, so it can differ
    // from B's x by one rounding step; the width itself is exact.
    expect(Math.max(...xs)).toBeCloseTo(B[0], 15)
    // And the Rectangle lies on the plane y = 0.2.
    expect(log.segments.every((s) => s[1] === A[1] && s[4] === A[1])).toBe(true)
  })

  it('works without the wasm read: the dwell ring alone offers the vertical from B', () => {
    const { scene } = makeScene({ ownerPoints: null })
    const tool = rectTool(scene)
    startVerticalAtA(tool)
    const now = vi.spyOn(performance, 'now')
    // Rest on B for 400 ms, then move above it.
    now.mockReturnValue(1000)
    hover(tool, snap(B, 'endpoint'))
    now.mockReturnValue(1400)
    hover(tool, snap(B, 'endpoint'))
    now.mockReturnValue(1500)
    hover(tool, snap([B[0] + 0.003, B[1], 0.7], 'plane'))
    expect(tool.lastSnap!.kind).toBe('from-point')
    expect(tool.lastSnap!.from).toEqual(B)
  })

  it('without owner corners or dwell nothing is offered', () => {
    const { scene } = makeScene({ ownerPoints: null })
    const tool = rectTool(scene)
    startVerticalAtA(tool)
    const raw = snap([B[0] + 0.003, B[1], 0.7], 'plane')
    hover(tool, raw)
    expect(tool.lastSnap).toEqual(raw)
  })

  it('a real endpoint under the cursor still wins', () => {
    const { scene } = makeScene()
    const tool = rectTool(scene)
    startVerticalAtA(tool)
    const top = snap([B[0] + 0.003, B[1], 0.7], 'endpoint')
    hover(tool, top)
    expect(tool.lastSnap!.kind).toBe('endpoint')
  })

  it('Esc from idle clears the ring; a tool change (activate) clears it too', () => {
    const { scene } = makeScene({ ownerPoints: null })
    const tool = rectTool(scene)
    const refs = (tool as unknown as { refs: ReferenceLines }).refs
    const now = vi.spyOn(performance, 'now')
    now.mockReturnValue(0)
    hover(tool, snap(B, 'endpoint'))
    now.mockReturnValue(500)
    hover(tool, snap(B, 'endpoint'))
    now.mockReturnValue(600)
    hover(tool, snap(C, 'endpoint'))
    expect(refs.dwellPoints).toEqual([B])
    tool.onKey(key('Escape'))
    expect(refs.dwellPoints).toHaveLength(0)

    now.mockReturnValue(1000)
    hover(tool, snap(B, 'endpoint'))
    now.mockReturnValue(1500)
    hover(tool, snap(B, 'endpoint'))
    now.mockReturnValue(1600)
    hover(tool, snap(C, 'endpoint'))
    expect(refs.dwellPoints).toEqual([B])
    tool.activate()
    expect(refs.dwellPoints).toHaveLength(0)
  })

  it('Esc mid-gesture keeps the ring', () => {
    const { scene } = makeScene({ ownerPoints: null })
    const tool = rectTool(scene)
    const refs = (tool as unknown as { refs: ReferenceLines }).refs
    const now = vi.spyOn(performance, 'now')
    now.mockReturnValue(0)
    hover(tool, snap(B, 'endpoint'))
    now.mockReturnValue(500)
    hover(tool, snap(C, 'endpoint'))
    expect(refs.dwellPoints).toEqual([B])
    startVerticalAtA(tool)
    tool.onKey(key('Escape'))
    expect(refs.dwellPoints).toEqual([B])
  })
})

describe('a plain projected snap keeps "projected" and gains the tie line', () => {
  it('Rectangle on the ground: a corner in the air drops to its foot, with the air point as source', () => {
    const { scene } = makeScene({ ownerPoints: null })
    const tool = rectTool(scene)
    click(tool, snap([0, 0, 0], 'ground'))
    const air = snap([0.5, 0.5, 1], 'endpoint')
    hover(tool, air)
    expect(tool.snapProjected()).toBe(true)
    expect(tool.lastSnap).not.toBeNull()
    expect(tool.lastSnap!.kind).toBe('endpoint')
    expect([tool.lastSnap!.x, tool.lastSnap!.y, tool.lastSnap!.z]).toEqual([0.5, 0.5, 0])
    expect(tool.lastSnap!.fromSource).toEqual([0.5, 0.5, 1])
    expect(tool.lastSnap!.from).toBeUndefined()
  })
})

describe('Line goes through the shared module', () => {
  it('an earlier chain vertex off the plane lends its foot', () => {
    // A chain on the ground whose first vertex was lifted: its foot is on the ground.
    const { scene } = makeScene({ ownerPoints: null })
    const tool = new LineTool(scene, new THREE.Group(), noop, noop, noop)
    tool.updateCamera(cameraAt(HOME))
    click(tool, snap([0, 0, 0], 'ground'))
    click(tool, snap([2, 2, 0], 'endpoint'))
    tool.onKey(key('ArrowLeft'))
    tool.onPointerMove(snap([2, 0.005, 0], 'on-axis', { direction: [0, -1, 0] }), rayThrough([2, 0.005, 0]))
    expect(tool.lastSnap!.kind).toBe('from-point')
    expect(tool.lastSnap!.from).toEqual([0, 0, 0])
    expect(tool.lastSnap!.direction).toEqual([1, 0, 0])
  })
})

describe('CueLayer draws the reference line', () => {
  const base: Snap = { x: 0.9, y: 0.2, z: 0.7, kind: 'from-point', direction: [0, 0, 1], from: [0.9, 0.2, 0] }

  it('dashed foot-to-snap line and a foot dot', () => {
    const layer = new CueLayer()
    layer.update(base)
    const kinds = layer.group.children.map((c) => c.constructor.name)
    expect(kinds).toEqual(['LineSegments', 'Points'])
  })

  it('an off-plane source adds the thin tie line', () => {
    const layer = new CueLayer()
    layer.update({ ...base, fromSource: [0.9, 0.9, 0] })
    expect(layer.group.children).toHaveLength(3)
  })

  it('a crossing draws both lines', () => {
    const layer = new CueLayer()
    layer.update({ ...base, fromAlso: { from: [0.3, 0.2, 0.7], direction: [1, 0, 0] } })
    expect(layer.group.children).toHaveLength(4)
  })

  it('a plain projected snap draws only the tie from its source', () => {
    const layer = new CueLayer()
    layer.update({ x: 0.5, y: 0.5, z: 0, kind: 'endpoint', fromSource: [0.5, 0.5, 1] })
    expect(layer.group.children).toHaveLength(1)
    expect(layer.group.children[0]).toBeInstanceOf(THREE.LineSegments)
  })

  it('replaces the old ten-metre guide line for a from-point snap', () => {
    const layer = new CueLayer()
    layer.update({ ...base, direction: [0, 0, 1] })
    const line = layer.group.children[0] as THREE.LineSegments
    // The dashed foot-to-snap line spans 0.7 m, not the 10 m guide.
    line.geometry.computeBoundingBox()
    const box = line.geometry.boundingBox!
    expect(box.max.z - box.min.z).toBeLessThan(1)
  })
})

describe('idle dwell points work before the first click (design §3.2 source 3)', () => {
  afterEach(() => { vi.restoreAllMocks() })

  const CORNER: V3 = [0.5, 0.5, 1] // a solid's top corner
  const FOOT: V3 = [0.5, 0.5, 0]
  const NEAR_FOOT = snap([0.503, 0.5, 0], 'ground')

  /** Rest on the corner for a second, then move down to the ground near its foot. */
  function restThenMove(tool: Clicker): void {
    const now = vi.spyOn(performance, 'now')
    now.mockReturnValue(1000)
    hover(tool, snap(CORNER, 'endpoint'))
    now.mockReturnValue(2000)
    hover(tool, snap(CORNER, 'endpoint'))
    now.mockReturnValue(2100)
    hover(tool, NEAR_FOOT)
  }

  const makers: Array<[string, (s: WasmScene) => Clicker & { lastSnap: Snap | null }]> = [
    ['Rectangle', (s) => rectTool(s) as never],
    ['Line', (s) => { const t = new LineTool(s, new THREE.Group(), noop, noop, noop); t.updateCamera(cameraAt(HOME)); return t as never }],
    ['Circle', (s) => { const t = new CircleTool(s, new THREE.Group(), noop, noop, noop); t.updateCamera(cameraAt(HOME)); return t as never }],
    ['Polygon', (s) => { const t = new PolygonTool(s, new THREE.Group(), noop, noop, noop); t.updateCamera(cameraAt(HOME)); return t as never }],
    ['Arc', (s) => { const t = new ArcTool(s, new THREE.Group(), noop, noop, noop); t.updateCamera(cameraAt(HOME)); return t as never }],
  ]

  for (const [name, make] of makers) {
    it(`${name}: the cue is the From Point crossing through the corner's foot, with no click yet`, () => {
      const { scene } = makeScene({ ownerPoints: null })
      const tool = make(scene)
      restThenMove(tool)
      const cue = tool.lastSnap
      expect(cue).not.toBeNull()
      expect(cue!.kind).toBe('from-point')
      expect([cue!.x, cue!.y, cue!.z]).toEqual(FOOT)
      expect(cue!.fromSource).toEqual(CORNER)
    })

    it(`${name}: nothing rested on, nothing offered`, () => {
      const { scene } = makeScene({ ownerPoints: null })
      const tool = make(scene)
      hover(tool, NEAR_FOOT)
      expect(tool.lastSnap?.kind ?? null).not.toBe('from-point')
    })
  }

  it('Rectangle: the first click lands exactly on the crossing', () => {
    const { scene, log } = makeScene({ ownerPoints: null })
    const tool = rectTool(scene)
    restThenMove(tool)
    tool.onPointerDown(NEAR_FOOT, rayThrough([0.503, 0.5, 0]))
    click(tool, snap([1.5, 1.5, 0], 'endpoint'))
    const xs = log.segments.flatMap((s) => [s[0], s[3]])
    const ys = log.segments.flatMap((s) => [s[1], s[4]])
    expect(Math.min(...xs)).toBe(0.5)
    expect(Math.min(...ys)).toBe(0.5)
  })

  it('Line: the first click lands exactly on the crossing', () => {
    const { scene, log } = makeScene({ ownerPoints: null })
    const tool = new LineTool(scene, new THREE.Group(), noop, noop, noop)
    tool.updateCamera(cameraAt(HOME))
    restThenMove(tool)
    tool.onPointerDown(NEAR_FOOT, rayThrough([0.503, 0.5, 0]))
    click(tool, snap([1.5, 0.5, 0], 'endpoint'))
    expect(log.segments).toHaveLength(1)
    expect(log.segments[0].slice(0, 3)).toEqual(FOOT)
  })
})
