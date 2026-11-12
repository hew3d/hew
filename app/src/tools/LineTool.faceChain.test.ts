/**
 * LineTool — face chains: every click after the first commits its segment as
 * its own draft on the face (`draft_begin_face` .. one `draft_add_segment`
 * .. `draft_end`, one undo step), exactly like plane mode — the kernel
 * lowers it into face topology (or the object's sketch) and nothing waits
 * to "reach an edge"; a chain closing on its start ends itself; a ground
 * chain's "projected" verdict does not outlive the chain, and Shift held
 * through the end of a chain never pins a plane by autorepeat.
 */
import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import { setPrecision, DEFAULT_PRECISION } from '../settings/units'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { draftMock, type DraftMockOpts } from './draftMock'

/** A ray straight down through world (x, y) — hits the z=1 top face. */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makePick(object: bigint, face: bigint) {
  return { object: () => object, face: () => face, depth: () => 1, instance: () => undefined, free: vi.fn() }
}

/** Object 7's top face 3: the unit square at z = 1. */
function makeScene(mockOpts: DraftMockOpts = {}) {
  const dm = draftMock(mockOpts)
  let gen = 1n
  // One undo step per ended draft: the history generation moves on `draft_end`.
  const draft_end = vi.fn((d: bigint) => {
    const r = dm.members.draft_end(d)
    gen += 1n
    return r
  })
  const scene = {
    history_generation: vi.fn(() => gen),
    ...dm.members,
    draft_end,
    scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    pick_face: vi.fn(() => makePick(7n, 3n)),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    node_parent: vi.fn(() => undefined),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    face_boundary: vi.fn(() => new Float32Array([0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1])),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])),
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return Object.assign(scene as unknown as WasmScene & typeof scene, { log: dm.log })
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const tool = new LineTool(scene, preview, vi.fn(), onToast, onFaceImprint, vi.fn())
  return { tool, preview, onToast, onFaceImprint }
}

const onEdge = (x: number, y: number): Snap => makeSnap({ x, y, z: 1, kind: 'on-edge', object: 7n, element: 11n, elementKind: 'edge' })
/** A hard pick keeps its exact point; a free `on-face` click commits the DISPLAYED length (displayedValue.ts). */
const hardOnFace = (x: number, y: number): Snap => makeSnap({ x, y, z: 1, kind: 'endpoint', object: 7n, element: 3n, elementKind: 'face' })
const onFace = (x: number, y: number): Snap => makeSnap({ x, y, z: 1, kind: 'on-face', object: 7n, element: 3n, elementKind: 'face' })

/** The chain as a flat xyz path, rebuilt from the committed segments (each
 *  click is one draft with one segment): the first start, then every end. */
const pathOf = (scene: ReturnType<typeof makeScene>) => {
  const segs = scene.log.segments
  return segs.length === 0 ? [] : [...segs[0].slice(0, 3), ...segs.flatMap((sg) => sg.slice(3, 6))]
}
/** Every draft was opened on object 7's face 3. */
const expectFaceDrafts = (scene: ReturnType<typeof makeScene>, n: number) => {
  expect(scene.draft_begin_face).toHaveBeenCalledTimes(n)
  for (const c of (scene.draft_begin_face as unknown as ReturnType<typeof vi.fn>).mock.calls) {
    expect(c[0]).toBe(7n)
    expect(c[1]).toBe(3n)
  }
  expect(scene.draft_begin_plane).not.toHaveBeenCalled()
}

describe('LineTool — a face chain commits each click as its own draft', () => {
  it('a bisecting cut continued to a third boundary point is one draft per click, the whole chain committed', () => {
    const scene = makeScene()
    const { tool, onToast, onFaceImprint } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5)) // left edge
    tool.onPointerDown(onEdge(1, 0.5), rayThrough(1, 0.5)) // right edge
    expectFaceDrafts(scene, 1) // each click after the first commits NOW
    tool.onPointerDown(onEdge(0.5, 0), rayThrough(0.5, 0)) // bottom edge
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expectFaceDrafts(scene, 2) // Escape commits nothing more
    expect(pathOf(scene)).toEqual([0, 0.5, 1, 1, 0.5, 1, 0.5, 0, 1])
    expect(scene.draft_end).toHaveBeenCalledTimes(2)
    expect(onToast).not.toHaveBeenCalled()
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  it('corner → top-edge midpoint → corner commits two drafts', () => {
    const scene = makeScene()
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'endpoint', object: 7n, element: 1n, elementKind: 'vertex' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0.5, y: 1, z: 1, kind: 'midpoint', object: 7n, element: 12n, elementKind: 'edge' }), rayThrough(0.5, 1))
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 1, kind: 'endpoint', object: 7n, element: 2n, elementKind: 'vertex' }), rayThrough(1, 0))
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expectFaceDrafts(scene, 2)
    expect(pathOf(scene)).toEqual([0, 0, 1, 0.5, 1, 1, 1, 0, 1])
    expect(onToast).not.toHaveBeenCalled()
  })

  it('an interior point between two boundary points is passed through unchanged', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(hardOnFace(0.5, 0.7), rayThrough(0.5, 0.7))
    tool.onPointerDown(onEdge(1, 0.5), rayThrough(1, 0.5))
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expectFaceDrafts(scene, 2)
    expect(pathOf(scene)).toEqual([0, 0.5, 1, 0.5, 0.7, 1, 1, 0.5, 1])
  })

  // FLIPPED. This pinned the OLD refusal: an open chain whose end dangles in
  // the face's interior was refused at chain end (`EndpointNotOnBoundary`,
  // by the old face path). A loose line on a face now simply commits.
  it('a loose line whose end dangles in the face interior commits (no refusal, no toast)', () => {
    const scene = makeScene()
    const { tool, onToast, onFaceImprint } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onFace(0.5, 0.5), rayThrough(0.5, 0.5)) // dangling end
    expectFaceDrafts(scene, 1)
    expect(pathOf(scene)).toEqual([0, 0.5, 1, 0.5, 0.5, 1])
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expectFaceDrafts(scene, 1)
    expect(onToast).not.toHaveBeenCalled()
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  // Kept intent of the old "refused chain toasts the kernel's typed refusal
  // once": a refusal from the kernel during a click's draft is toasted once
  // with its code, the draft is cancelled, and nothing is imprinted.
  it('a kernel refusal during a click\'s draft toasts its typed code once and commits nothing else', () => {
    const scene = makeScene({
      failSegmentAt: () => new Error('PointOffPlane: point 1 is off the draft plane'),
    })
    const { tool, onToast, onFaceImprint } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onFace(0.5, 0.5), rayThrough(0.5, 0.5))
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(scene.draft_cancel).toHaveBeenCalledTimes(1)
    expect(scene.draft_end).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('PointOffPlane')
    expect(onFaceImprint).not.toHaveBeenCalled()
  })

  // FLIPPED. This pinned the OLD refusal wording ("doesn't split the face",
  // from `LoopNotStrictlyInside` by the old face path): a chain from edge
  // to edge across a face. Face mode no longer plans a path/loop at chain
  // end; the segment just commits and there is no toast at all.
  it('an edge-to-edge chain just commits: no "doesn\'t split the face" toast', () => {
    const scene = makeScene()
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onEdge(1, 0.5), rayThrough(1, 0.5))
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expectFaceDrafts(scene, 1)
    expect(pathOf(scene)).toEqual([0, 0.5, 1, 1, 0.5, 1])
    expect(onToast).not.toHaveBeenCalled()
  })

  it('makes no sub-face probes of its own: the kernel locates every piece', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onEdge(1, 0.5), rayThrough(1, 0.5))
    tool.onPointerDown(onFace(0.95, 0.05), rayThrough(0.95, 0.05))
    tool.onPointerDown(onEdge(0.5, 0), rayThrough(0.5, 0))
    const picksBefore = (scene.pick_face as ReturnType<typeof vi.fn>).mock.calls.length
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect((scene.pick_face as ReturnType<typeof vi.fn>).mock.calls.length).toBe(picksBefore)
    expectFaceDrafts(scene, 3) // one draft per click after the first
  })

  it('a stale face handle (UnknownFace) falls back to a draft on the face\'s plane', () => {
    const scene = makeScene()
    ;(scene.draft_begin_face as ReturnType<typeof vi.fn>).mockImplementationOnce(() => {
      throw new Error('UnknownFace: no such face')
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onEdge(1, 0.5), rayThrough(1, 0.5))
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_plane.mock.calls[0].slice(0, 6)).toEqual([0, 0.5, 1, 0, 0, 1])
    expect(pathOf(scene)).toEqual([0, 0.5, 1, 1, 0.5, 1])
    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('LineTool — a face chain closing on its own start ends itself', () => {
  it('a closed chain that touches the face\'s edges commits one draft per click and ends on the start', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(hardOnFace(0.5, 0.2), rayThrough(0.5, 0.2))
    tool.onPointerDown(onEdge(1, 0.5), rayThrough(1, 0.5))
    tool.onPointerDown(hardOnFace(0.5, 0.8), rayThrough(0.5, 0.8))
    expect(tool.capturingInput()).toBe(true)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5)) // back to the start
    expect(tool.capturingInput()).toBe(false)
    expectFaceDrafts(scene, 4)
    // The loop plus its closing segment, committed segment by segment.
    expect(pathOf(scene)).toEqual([0, 0.5, 1, 0.5, 0.2, 1, 1, 0.5, 1, 0.5, 0.8, 1, 0, 0.5, 1])
  })

  it('clicking the start point again commits the closing segment and ends the chain', () => {
    const scene = makeScene()
    const { tool, onFaceImprint } = makeTool(scene)
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2))
    tool.onPointerDown(hardOnFace(0.8, 0.2), rayThrough(0.8, 0.2))
    tool.onPointerDown(hardOnFace(0.5, 0.8), rayThrough(0.5, 0.8))
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2))
    expect(tool.capturingInput()).toBe(false) // the loop ended the chain by itself
    expectFaceDrafts(scene, 3)
    expect(pathOf(scene)).toEqual([0.2, 0.2, 1, 0.8, 0.2, 1, 0.5, 0.8, 1, 0.2, 0.2, 1])
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })
})

describe('LineTool — face segments are committed as they are placed', () => {
  // The placed segment is committed geometry (it renders from the scene),
  // not a preview line, and face-mode transient segments are no longer
  // published: the preview holds only the live rubber band.
  it('after two face clicks the segment is committed and the preview holds just the rubber band', () => {
    const scene = makeScene()
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onFace(0.5, 0.5), rayThrough(0.5, 0.5))
    expectFaceDrafts(scene, 1)
    expect(scene.add_transient_segment).not.toHaveBeenCalled()
    expect(preview.children.length).toBe(0) // no placed-segment preview any more
    tool.onPointerMove(onFace(0.7, 0.3), rayThrough(0.7, 0.3))
    expect(preview.children.length).toBeGreaterThanOrEqual(1) // the rubber band
  })
})

describe('LineTool — a ground chain\'s "projected" verdict ends with the chain', () => {
  it('snapProjected() is false again once the chain ends, and while idle', () => {
    const scene = makeScene()
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockImplementation(() => undefined) // bare ground
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), rayThrough(0, 0))
    // An edge point (not a named point): a named point would keep its height.
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 1, kind: 'on-edge' }), rayThrough(1, 0))
    expect(tool.snapProjected()).toBe(true)
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.snapProjected()).toBe(false)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), rayThrough(2, 0))
    expect(tool.snapProjected()).toBe(false)
  })
})

describe('LineTool — Shift held through the end of a chain never pins a plane by autorepeat', () => {
  it('a keydown repeat arriving after the chain ended does not toggle the pin; a fresh press does', () => {
    const scene = makeScene()
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockImplementation(() => undefined) // ground chain
    const { tool } = makeTool(scene)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), rayThrough(0, 0))
    tool.onPointerMove(makeSnap({ x: 1, y: 0.01, z: 0, kind: 'ground' }), rayThrough(1, 0.01))
    tool.setShiftHeld(true) // mid-chain: the axis lock
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind: 'ground' }), rayThrough(1, 0))
    tool.onKey({ key: 'Escape' } as KeyboardEvent) // chain ends with Shift still down
    tool.setShiftHeld(true) // keydown autorepeat reaches the now-idle tool
    tool.setShiftHeld(true)
    expect(tool.statusHint()).not.toMatch(/Pinned/)
    tool.setShiftHeld(false)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), rayThrough(2, 0))
    tool.setShiftHeld(true) // a genuine new press
    expect(tool.statusHint()).toMatch(/Pinned/)
  })
})

describe('LineTool — review fixes on the face-chain round', () => {
  it('a snapped edge point far from the origin reaches the kernel exactly as placed', () => {
    const scene = makeScene()
    const X = 200
    ;(scene.face_boundary as ReturnType<typeof vi.fn>).mockImplementation(() =>
      new Float32Array([X, 0, 1, X + 1, 0, 1, X + 1, 1, 1, X, 1, 1]))
    const { tool, onToast } = makeTool(scene)
    const edgePt = (x: number, y: number): Snap => makeSnap({ x, y, z: 1, kind: 'on-edge', object: 7n, element: 11n, elementKind: 'edge' })
    tool.onPointerDown(edgePt(X + 1.23e-5, 0.5), rayThrough(X, 0.5))
    tool.onPointerDown(edgePt(X + 1 - 1.23e-5, 0.5), rayThrough(X + 1, 0.5))
    tool.onPointerDown(edgePt(X + 0.5, 1.23e-5), rayThrough(X + 0.5, 0))
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expectFaceDrafts(scene, 2)
    expect(pathOf(scene)).toEqual([X + 1.23e-5, 0.5, 1, X + 1 - 1.23e-5, 0.5, 1, X + 0.5, 1.23e-5, 1])
    expect(onToast).not.toHaveBeenCalled()
  })

  it('the dblclick following a single click that closed a loop is consumed, not handed to the host', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2))
    tool.onPointerDown(onFace(0.8, 0.2), rayThrough(0.8, 0.2))
    tool.onPointerDown(onFace(0.5, 0.8), rayThrough(0.5, 0.8))
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2)) // closes; the chain is already gone
    expect(tool.capturingInput()).toBe(false)
    expect(tool.onDoubleClick(onFace(0.2, 0.2), rayThrough(0.2, 0.2))).toBe(true) // the same gesture's dblclick
    expect(tool.onDoubleClick(onFace(0.2, 0.2), rayThrough(0.2, 0.2))).toBe(false) // a later one is the host's
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2))
    tool.onPointerDown(onFace(0.8, 0.2), rayThrough(0.8, 0.2))
    tool.onPointerDown(onFace(0.5, 0.8), rayThrough(0.5, 0.8))
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2))
    tool.onPointerMove(onFace(0.3, 0.3), rayThrough(0.3, 0.3)) // the pointer moved on: no dblclick is owed
    expect(tool.onDoubleClick(onFace(0.3, 0.3), rayThrough(0.3, 0.3))).toBe(false)
  })

  it('a hover replayed at the closing press\'s own ray (the Viewport\'s replay after the commit) keeps the dblclick owed', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2))
    tool.onPointerDown(onFace(0.8, 0.2), rayThrough(0.8, 0.2))
    tool.onPointerDown(onFace(0.5, 0.8), rayThrough(0.5, 0.8))
    tool.onPointerDown(onFace(0.2, 0.2), rayThrough(0.2, 0.2)) // closes the loop
    tool.onPointerMove(onFace(0.2, 0.2), rayThrough(0.2, 0.2)) // replayed where it rests
    expect(tool.onDoubleClick(onFace(0.2, 0.2), rayThrough(0.2, 0.2))).toBe(true)
  })

  it('retyping a face segment\'s length undoes and re-commits it at the typed length', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onFace(0.5, 0.5), rayThrough(0.5, 0.5))
    expectFaceDrafts(scene, 1)
    tool.onKey({ key: '0' } as KeyboardEvent)
    tool.onKey({ key: '.' } as KeyboardEvent)
    tool.onKey({ key: '3' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    // Face mode retypes like plane mode: one undo, then the segment is
    // re-committed at 0.3 m along the same direction (the old chain-in-
    // preview redraw no longer exists — nothing is uncommitted).
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expectFaceDrafts(scene, 2)
    expect(scene.log.segments[1]).toEqual([0, 0.5, 1, 0.3, 0.5, 1])
  })
})

describe('LineTool — face chain points are the displayed length', () => {
  // This pins the Exact digits (0.500432 m); the default m rounding is coarser.
  beforeAll(() => setPrecision('exact', 'm'))
  afterAll(() => setPrecision(DEFAULT_PRECISION.m, 'm'))
  it('a free on-face click is quantized to the shown length; a hard click keeps its exact point', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onFace(0.5004321, 0.5), rayThrough(0.5004321, 0.5)) // shows 0.500432 m
    tool.onPointerDown(hardOnFace(0.5004321, 0.80077), rayThrough(0.5004321, 0.80077))
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    const path = pathOf(scene)
    expect(path.slice(0, 6)).toEqual([0, 0.5, 1, 0.500432, 0.5, 1])
    expect(path.slice(6)).toEqual([0.5004321, 0.80077, 1])
  })
})

// #51b: where edges of two separate Objects pass through each other, the
// engine offers the crossing as an `intersection` snap carrying both edges
// (covered by the inference specs). It reaches the Line tool through the
// ordinary snap pipeline, so a line starts from it exactly.
describe('LineTool — starts from an edge-crossing snap (#51b)', () => {
  // The reported two-box case: obj_9's top-back edge crosses obj_7's
  // front-left vertical edge at CROSSING, on obj_7's front face (y = 0.0714).
  const CROSSING: [number, number, number] = [0.5001, 0.0714, 0.1826]
  const Y = 0.0714
  /** Object 7's front face 3, which the pick ray to the crossing lands on. */
  function frontFaceScene() {
    const scene = makeScene()
    scene.face_normal = vi.fn(() => new Float64Array([0, -1, 0]))
    scene.face_plane = vi.fn(() => new Float64Array([0, Y, 0, 0, -1, 0]))
    scene.face_boundary = vi.fn(() => new Float32Array([
      0.5001, Y, 0, 1.2, Y, 0, 1.2, Y, 0.5, 0.5001, Y, 0.5,
    ]))
    return scene
  }
  const rayFromFront = (p: readonly [number, number, number]): Ray => ({
    origin: [p[0], p[1] - 5, p[2]], direction: [0, 1, 0],
  })

  for (const [label, object, element] of [
    ["obj_9's edge first", 9n, 21n],
    ["obj_7's edge first", 7n, 11n],
  ] as const) {
    it(`the first segment starts exactly at the crossing (${label})`, () => {
      const scene = frontFaceScene()
      const { tool, onToast } = makeTool(scene)
      const crossing = makeSnap({
        x: CROSSING[0], y: CROSSING[1], z: CROSSING[2],
        kind: 'intersection', object, element, elementKind: 'edge',
      })
      tool.onPointerDown(crossing, rayFromFront(CROSSING))
      expect(tool.capturingInput()).toBe(true)

      // Across obj_7's front face to a point on its right edge.
      const end: [number, number, number] = [1.2, Y, 0.3]
      tool.onPointerDown(
        makeSnap({ x: end[0], y: end[1], z: end[2], kind: 'on-edge', object: 7n, element: 12n, elementKind: 'edge' }),
        rayFromFront(end),
      )
      expect(onToast).not.toHaveBeenCalled()
      expect(scene.log.segments).toHaveLength(1)
      expect(scene.log.segments[0].slice(0, 3)).toEqual(CROSSING)
      expect(scene.log.segments[0].slice(3, 6)).toEqual(end)
      // Drawn on obj_7's front face, the face under the cursor.
      expectFaceDrafts(scene, 1)
    })
  }
})

describe('LineTool — a locked first segment that leaves the face (playtest step 5, the ridge post)', () => {
  // A ↑ lock from a point on the box's top face (here the midpoint of its
  // back edge), when no face holding the lock is in view — the back face
  // turns away from the camera — so the chain cannot move onto a neighbour
  // face. The segment cannot lie on the top face either: the kernel refused
  // it as off the face (`PointOffPlane`), and the typed post never appeared.
  // A bare face anchor is a point like any other: the segment re-homes the
  // chain off the face exactly as a plane chain's locked segment does.
  const backEdgeMid = makeSnap({ x: 0.5, y: 1, z: 1, kind: 'midpoint', object: 7n, element: 12n, elementKind: 'edge' })
  const upFromMid = makeSnap({ x: 0.5, y: 1, z: 1.25, kind: 'on-axis' })
  const fromFrontAbove = (p: readonly [number, number, number]): Ray => {
    const eye: [number, number, number] = [3, -2.5, 3]
    const d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]]
    const len = Math.hypot(d[0], d[1], d[2])
    return { origin: eye, direction: [d[0] / len, d[1] / len, d[2] / len] }
  }
  /** The kernel's face draft refuses a point off the face's plane z = 1. */
  function strictScene() {
    const scene = makeScene({ enforcePlane: true })
    let faceDraftOpen = false
    const beginFace = scene.draft_begin_face as unknown as (...a: unknown[]) => bigint
    scene.draft_begin_face = vi.fn((...a: unknown[]) => {
      const d = beginFace(...a)
      faceDraftOpen = true
      return d
    }) as unknown as typeof scene.draft_begin_face
    const add = scene.draft_add_segment as unknown as (...a: unknown[]) => unknown
    scene.draft_add_segment = vi.fn((d: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
      if (faceDraftOpen && (Math.abs(az - 1) > 1e-9 || Math.abs(bz - 1) > 1e-9)) throw new Error('PointOffPlane: point is off the draft plane')
      return add(d, ax, ay, az, bx, by, bz)
    }) as unknown as typeof scene.draft_add_segment
    const end = scene.draft_end as unknown as (d: bigint) => string
    scene.draft_end = vi.fn((d: bigint) => {
      faceDraftOpen = false
      return end(d)
    }) as unknown as typeof scene.draft_end
    return scene
  }

  for (const how of ['typed', 'clicked'] as const) {
    it(`a ${how} ↑ segment from the back edge's midpoint draws the post off the face, with no toast`, () => {
      const scene = strictScene()
      const { tool, onToast } = makeTool(scene)
      tool.onPointerMove(backEdgeMid, fromFrontAbove([0.5, 1, 1]))
      tool.onPointerDown(backEdgeMid, fromFrontAbove([0.5, 1, 1]))
      tool.onKey({ key: 'ArrowUp' } as KeyboardEvent)
      tool.onPointerMove(upFromMid, fromFrontAbove([0.5, 1, 1.25]))
      if (how === 'typed') {
        for (const k of '0.5') tool.onKey({ key: k } as KeyboardEvent)
        tool.onKey({ key: 'Enter' } as KeyboardEvent)
      } else {
        tool.onPointerDown(upFromMid, fromFrontAbove([0.5, 1, 1.25]))
      }
      expect(onToast).not.toHaveBeenCalled()
      const top = how === 'typed' ? 1.5 : 1.25
      expect(scene.log.segments).toEqual([[0.5, 1, 1, 0.5, 1, top]])
      expect(scene.draft_begin_face).not.toHaveBeenCalled()
      expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
      // The chain goes on from the post's top.
      expect(tool.capturingInput()).toBe(true)
    })
  }

  it('the arrow lock of that first segment releases once it commits (R19, _leaveFaceForPlane)', () => {
    const scene = strictScene()
    const { tool } = makeTool(scene)
    tool.onPointerMove(backEdgeMid, fromFrontAbove([0.5, 1, 1]))
    tool.onPointerDown(backEdgeMid, fromFrontAbove([0.5, 1, 1]))
    tool.onKey({ key: 'ArrowUp' } as KeyboardEvent)
    expect((tool as unknown as { lockAxis: unknown }).lockAxis).toBe(2)
    tool.onPointerMove(upFromMid, fromFrontAbove([0.5, 1, 1.25]))
    tool.onPointerDown(upFromMid, fromFrontAbove([0.5, 1, 1.25]))
    expect(scene.log.segments).toHaveLength(1)
    expect((tool as unknown as { lockAxis: unknown }).lockAxis).toBeNull()
  })

  it('a refused plane commit keeps the face cursor, so retyping the length without moving retries', () => {
    const scene = strictScene()
    // The first plane draft's segment is refused once (a transient kernel refusal).
    let planeDraftOpen = false
    let refused = false
    const beginPlane = scene.draft_begin_plane as unknown as (...a: unknown[]) => bigint
    scene.draft_begin_plane = vi.fn((...a: unknown[]) => {
      const d = beginPlane(...a)
      planeDraftOpen = true
      return d
    }) as unknown as typeof scene.draft_begin_plane
    const add = scene.draft_add_segment as unknown as (...a: unknown[]) => unknown
    scene.draft_add_segment = vi.fn((...a: unknown[]) => {
      if (planeDraftOpen && !refused) {
        refused = true
        throw new Error('PointOffPlane: injected one-shot refusal')
      }
      return add(...a)
    }) as unknown as typeof scene.draft_add_segment
    const { tool, onToast } = makeTool(scene)
    tool.onPointerMove(backEdgeMid, fromFrontAbove([0.5, 1, 1]))
    tool.onPointerDown(backEdgeMid, fromFrontAbove([0.5, 1, 1]))
    tool.onKey({ key: 'ArrowUp' } as KeyboardEvent)
    tool.onPointerMove(upFromMid, fromFrontAbove([0.5, 1, 1.25]))
    const typeHalf = () => {
      for (const k of '0.5') tool.onKey({ key: k } as KeyboardEvent)
      tool.onKey({ key: 'Enter' } as KeyboardEvent)
    }
    typeHalf()
    expect(refused).toBe(true)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(String(onToast.mock.calls[0][0])).toMatch(/drawing surface/)
    expect(scene.log.segments).toEqual([])
    // Same pointer position: the typed length is still in the box, so Enter
    // alone retries; it must reach the scene, not ask for a cursor move.
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(scene.log.segments).toEqual([[0.5, 1, 1, 0.5, 1, 1.5]])
    for (const c of onToast.mock.calls) expect(String(c[0])).not.toContain('Move the cursor to point the line')
  })

  it('a later locked segment of a chain already on the face still stays on the face', () => {
    const scene = strictScene()
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onPointerDown(onFace(0.5, 0.5), rayThrough(0.5, 0.5))
    expectFaceDrafts(scene, 1)
    tool.onKey({ key: 'ArrowUp' } as KeyboardEvent)
    tool.onPointerMove(makeSnap({ x: 0.5, y: 0.5, z: 1.3, kind: 'on-axis' }), fromFrontAbove([0.5, 0.5, 1.3]))
    for (const k of '0.3') tool.onKey({ key: k } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    // Refused, and said so: face chains never re-home once they have a segment.
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(scene.log.segments).toHaveLength(1)
  })
})

describe('LineTool — an arrow lock on a face chain covers one segment (R19, _appendFacePoint)', () => {
  it('the lock releases once the locked face segment commits, and the next segment is free', () => {
    const scene = makeScene()
    const { tool } = makeTool(scene)
    const lockOf = () => (tool as unknown as { lockAxis: unknown }).lockAxis
    tool.onPointerDown(onEdge(0, 0.5), rayThrough(0, 0.5))
    tool.onKey({ key: 'ArrowRight' } as KeyboardEvent)
    expect(lockOf()).toBe(0)
    tool.onPointerMove(onFace(0.5, 0.5), rayThrough(0.5, 0.5))
    tool.onPointerDown(onFace(0.5, 0.5), rayThrough(0.5, 0.5))
    expectFaceDrafts(scene, 1)
    expect(scene.log.segments).toHaveLength(1)
    expect(lockOf()).toBeNull()
    // The next segment goes where it is clicked, off the old axis.
    tool.onPointerMove(hardOnFace(0.5, 0.9), rayThrough(0.5, 0.9))
    tool.onPointerDown(hardOnFace(0.5, 0.9), rayThrough(0.5, 0.9))
    expect(scene.log.segments).toHaveLength(2)
    expect(scene.log.segments[1].slice(3, 6)).toEqual([0.5, 0.9, 1])
  })
})
