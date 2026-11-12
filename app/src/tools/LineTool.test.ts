import { describe, it, expect, vi, beforeAll, afterAll, beforeEach, afterEach } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import { setLengthUnit, getLengthUnit, setPrecision, DEFAULT_PRECISION } from '../settings/units'
import type { Scene as WasmScene } from '../wasm/loader'
import { setPickRenderMode } from './pickPolicy'
import type { Ray } from '../viewport/math'
import { draftMock } from './draftMock'

/** A ray straight down (−Z) through world (x, y) — hits a z=1 top face at (x, y, 1). */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    depth: () => 1,
    instance: () => instance,
    free: vi.fn(),
  }
}

/** Minimal WasmScene stub — only the members LineTool calls in these paths. */
function makeWasmScene(opts: {
  pick?: () => ReturnType<typeof makePick> | undefined
  /** node_parent(0, id) result per object (a grouped object's group id). */
  parents?: Map<bigint, bigint>
} = {}): WasmScene {
  return {
    history_generation: vi.fn(() => 1n),
    ...draftMock().members,
    pick_face: vi.fn(() => opts.pick?.()),
    pick_sketch: vi.fn(() => undefined), // no committed sketches in these fixtures
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])), // every minted sketch is on the ground plane
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
    // A top face at z=1, normal +Z.
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])), // translated +5 in x
    // Drawing axes at the world frame (origin + X/Y/Z). These fixtures
    // predate the movable axes, and draw-plane resolution now reads the
    // frame on every click, so the neutral world frame keeps them meaning
    // what they meant: this suite is about instance ROUTING, not about
    // where the axes sit.
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement }
}

// The context contract shared by every draw tool (see RectangleTool.test.ts):
// inside an entered object's editing context, drawing is scoped to that
// object — a click on another object's face OR on empty ground is ignored
// outright, never re-routed to a top-level ground sketch.
describe('LineTool — editing-context scoping', () => {
  it('in-context clicks on empty ground do NOT start a top-level ground sketch', () => {
    const scene = makeWasmScene() // pick_face misses — bare ground under the ray
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))

    expect(tool.capturingInput()).toBe(false) // no chain ever anchored
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).not.toHaveBeenCalled()
  })

  it('in-context clicks on a DIFFERENT object\'s face are ignored', () => {
    const scene = makeWasmScene({ pick: () => makePick(999n, 3n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0))

    expect(tool.capturingInput()).toBe(false)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
  })

  it('in-context clicks on the ENTERED object\'s face anchor a face chain', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))

    expect(tool.capturingInput()).toBe(true) // face chain anchored
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
  })
})

describe('LineTool — top-level plain-object policy (parity with RectangleTool)', () => {
  it('a plain object\'s face anchors a face chain directly (no edit context)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    expect(tool.capturingInput()).toBe(true)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
  })

  it('a GROUPED object\'s face is drawn on as free drawing on its world plane (not the ground)', () => {
    const scene = makeWasmScene({
      pick: () => makePick(7n, 3n),
      parents: new Map([[7n, 5n]]),
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1, kind: 'face' }), rayThrough(1, 1))

    expect((scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 6)).toEqual([0, 0, 1, 0, 0, 1])
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(1) // free segment on the plane
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
  })

  it.each([
    ['GROUPED', () => makeWasmScene({ pick: () => makePick(7n, 3n), parents: new Map([[7n, 5n]]) }), 0],
    ['instanced', () => makeWasmScene({ pick: () => makePick(7n, 3n, 12n) }), 5],
  ])('a first click ON the corner of a %s top draws free on its world plane, not the ground', (_n, mk, dx) => {
    const scene = mk()
    // The plain ray MISSES at the corner (strict polygon test); only the
    // nudged boundary probes hit the face.
    const plain = rayThrough(dx, 0).direction
    const hit = (scene.pick_face as ReturnType<typeof vi.fn>).getMockImplementation()!
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockImplementation((...a: number[]) =>
      a[3] === plain[0] && a[4] === plain[1] && a[5] === plain[2] ? undefined : (hit as (...x: number[]) => unknown)(...a))
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: dx, y: 0, z: 1, kind: 'endpoint', object: 7n, elementKind: 'vertex' }), rayThrough(dx, 0))
    tool.onPointerDown(makeSnap({ x: dx + 1, y: 1, z: 1, kind: 'face' }), rayThrough(dx + 1, 1))
    const a = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 6) as number[]
    expect(a[2]).toBeCloseTo(1)
    expect(a.slice(3)).toEqual([0, 0, 1])
    expect(scene.draft_begin_plane_in_instance).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(1)
  })

  it('instanced (component) geometry is drawn on as free drawing on its world plane', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, 12n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 5, y: 0, z: 1, kind: 'face' }), rayThrough(5, 0))
    tool.onPointerDown(makeSnap({ x: 6, y: 1, z: 1, kind: 'face' }), rayThrough(6, 1))

    const a = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 6) as number[]
    expect(a[2]).toBeCloseTo(1)
    expect(a.slice(3)).toEqual([0, 0, 1])
    expect(scene.draft_begin_plane_in_instance).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
  })
})

// Component-edit-parity.md phase A2: inside an INSTANCE editing context,
// draw tools route to the definition-owned wasm surface instead of the
// world one — the fix for the axis-lock symptom (see the idle-lock case
// below, the flagship repro) and for face-mode cuts refusing outright.
describe('LineTool — instance editing context (component-edit-parity.md phase A2)', () => {
  const INSTANCE = 42n
  const COMPONENT = 5n
  const INSTANCE_CTX = { kind: 'instance' as const, id: INSTANCE, component: COMPONENT }

  it('face mode routes to draft_begin_face_in_instance, never the world call', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, INSTANCE) })
    const { tool, onFaceImprint } = makeTool(scene)
    tool.setEditContext(INSTANCE_CTX)

    // `faceDrawEligible` isn't injected in this unit test, but the default
    // fallback policy (defaultFaceEligible) refuses any instanced pick —
    // inject the richer predicate directly, mirroring what the Viewport's
    // `faceDrawEligible` would report for a member of the entered instance.
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 1, kind: 'face' }), rayThrough(1, 0))
    // The second click commits its segment as its own draft; ending the chain
    // afterwards commits nothing further (face mode commits per click).
    tool.onDoubleClick(null, rayThrough(1, 0))

    expect(scene.draft_begin_face_in_instance).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_face_in_instance).toHaveBeenCalledWith(INSTANCE, 7n, 3n, expect.any(String))
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(1)
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  it('plane mode (idle-locked, THE original axis-lock symptom) opens draft_begin_plane_in_instance, never draft_begin_plane', () => {
    const scene = makeWasmScene({ pick: () => undefined }) // no face under the cursor
    const { tool } = makeTool(scene)
    tool.setEditContext(INSTANCE_CTX)

    // Arrow-key idle lock (Z/blue axis) — the exact repro named in the design.
    tool.onKey({ key: 'ArrowUp', repeat: false } as unknown as KeyboardEvent)
    tool.onPointerDown(makeSnap({ x: 6, y: 1, z: 0.5 }), rayThrough(6, 1))
    tool.onPointerDown(makeSnap({ x: 6, y: 2, z: 0.5 }), rayThrough(6, 2))

    expect(scene.draft_begin_plane_in_instance).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_plane_in_instance).toHaveBeenCalledWith(INSTANCE, 6, 1, 0.5, 0, 0, 1, expect.any(String))
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
    // Points go in as WORLD coordinates: the instance variant maps through
    // the pose itself (no tool-side toLocal any more).
    expect(scene.draft_add_segment).toHaveBeenCalledWith(
      expect.any(BigInt), 6, 1, 0.5, 6, 2, 0.5,
    )
  })

  it('plane mode on empty space (no idle lock) ALSO opens an instance draft, not a world one', () => {
    const scene = makeWasmScene({ pick: () => undefined })
    const { tool } = makeTool(scene)
    tool.setEditContext(INSTANCE_CTX)

    tool.onPointerDown(makeSnap({ x: 6, y: 1, z: 0 }), rayThrough(6, 1))
    tool.onPointerDown(makeSnap({ x: 7, y: 1, z: 0 }), rayThrough(7, 1))

    expect(scene.draft_begin_plane_in_instance).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
  })
})

/** Minimal fake `KeyboardEvent` — `onKey` only reads `.key`/`.repeat`/
 *  `.preventDefault`. Mirrors MoveTool.test.ts's/RotateTool.test.ts's own
 *  helper. */
function makeKeyEvent(key: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  return { key, repeat: opts.repeat ?? false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

// From-point closing inference (module doc — SketchUp's "draw three sides
// of a square, the fourth snaps shut"): a chain A(0,0,0) -> B(2,2,0), so the
// current segment starts at S=B with the only earlier vertex P=A. Every
// candidate test below hovers near (2,0,0) — where a segment locked toward
// -Y from B meets the line through A along red (X) — or near (0,1,0) —
// where the foot of a perpendicular from an UNLOCKED cursor lands on the
// line through A along green (Y). `makeWasmScene`'s default `axes()` is the
// world-identity frame, so red=X/green=Y/blue=Z throughout.
describe('LineTool — from-point closing inference', () => {
  function startTwoPointChain(tool: LineTool): void {
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // A
    // B — anchors S. A hard pick keeps B exact (a free click would round the 2.828.. m leg to the displayed 2.828 m).
    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 0, kind: 'endpoint' }), rayThrough(2, 2))
  }

  it('locked: candidate is where the locked segment meets the line through the earlier vertex', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    startTwoPointChain(tool)
    tool.onKey(makeKeyEvent('ArrowLeft')) // lock green (Y) — arrowToAxis: ArrowLeft -> 1

    // A soft/weak resolve near the true closing point (2, 0, 0) — 5 mm off,
    // well inside the 0.02 m fallback tolerance (no camera feed wired here).
    tool.onPointerMove(makeSnap({ x: 2, y: 0.005, z: 0, kind: 'on-axis', direction: [0, -1, 0] }), rayThrough(2, 0.005))

    expect(tool.lastSnap).not.toBeNull()
    expect(tool.lastSnap!.kind).toBe('from-point')
    expect(tool.lastSnap!.x).toBeCloseTo(2, 9)
    expect(tool.lastSnap!.y).toBeCloseTo(0, 9)
    expect(tool.lastSnap!.z).toBeCloseTo(0, 9)
    // direction is the CANDIDATE axis (red, through A) — not the lock
    // direction (green) the segment itself is traveling along.
    expect(tool.lastSnap!.direction).toEqual([1, 0, 0])

    // The click commits exactly at the candidate — never the raw 5 mm-off
    // point underneath it (a click must land where the chip/preview showed
    // it would).
    tool.onPointerDown(makeSnap({ x: 2, y: 0.005, z: 0, kind: 'on-axis', direction: [0, -1, 0] }), rayThrough(2, 0.005))
    const last = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.at(-1)!
    expect(last.slice(4, 7)).toEqual([2, 0, 0])
  })

  it('unlocked: candidate is the foot of the perpendicular from the cursor', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    startTwoPointChain(tool)
    // No lock. Cursor sits 1 cm off the line x=0 through A (green axis).
    tool.onPointerMove(makeSnap({ x: 0.01, y: 1, z: 0, kind: 'ground' }), rayThrough(0.01, 1))

    expect(tool.lastSnap).not.toBeNull()
    expect(tool.lastSnap!.kind).toBe('from-point')
    expect(tool.lastSnap!.x).toBeCloseTo(0, 9)
    expect(tool.lastSnap!.y).toBeCloseTo(1, 9)
    expect(tool.lastSnap!.z).toBeCloseTo(0, 9)
    expect(tool.lastSnap!.direction).toEqual([0, 1, 0])
  })

  it('precedence: a precise kernel snap (endpoint) always wins — never overridden', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    startTwoPointChain(tool)
    tool.onKey(makeKeyEvent('ArrowLeft')) // lock green

    // Same near-(2,0,0) position as the locked test above, but this time
    // the kernel resolved a PRECISE point (e.g. an existing endpoint) —
    // must pass through untouched.
    tool.onPointerMove(makeSnap({ x: 2, y: 0.005, z: 0, kind: 'endpoint' }), rayThrough(2, 0.005))

    expect(tool.lastSnap).not.toBeNull()
    expect(tool.lastSnap!.kind).toBe('endpoint')
    expect(tool.lastSnap!.y).toBeCloseTo(0.005, 9) // untouched — NOT snapped to (2,0,0)
  })

  it('out of tolerance: a weak snap kind far from any candidate line is left alone', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    startTwoPointChain(tool)
    tool.onKey(makeKeyEvent('ArrowLeft')) // lock green

    // Locked toward -Y, but 0.5 m off the (2,0,0) closing point — far
    // outside the 0.02 m fallback tolerance.
    tool.onPointerMove(makeSnap({ x: 2, y: 0.5, z: 0, kind: 'on-axis', direction: [0, -1, 0] }), rayThrough(2, 0.5))

    expect(tool.lastSnap).not.toBeNull()
    expect(tool.lastSnap!.kind).toBe('on-axis') // no from-point override
  })

  it('requires at least two committed points — a single anchored point never overrides', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // A only — no B yet
    tool.onKey(makeKeyEvent('ArrowLeft'))

    tool.onPointerMove(makeSnap({ x: 0, y: 2, z: 0, kind: 'on-axis', direction: [0, 1, 0] }), rayThrough(0, 2))

    expect(tool.lastSnap).not.toBeNull()
    expect(tool.lastSnap!.kind).toBe('on-axis')
  })
})

// Playtest item 7: the first click on an edge shared by two faces adopts
// whichever face the pick hit; an arrow lock that leaves that face must
// re-adopt the neighbour holding both the anchor and the lock.
describe('LineTool — an axis lock that leaves the adopted face re-adopts the neighbouring face', () => {
  const WEST = new Float64Array([-1, 0, 0])
  const TOP = new Float64Array([0, 0, 1])
  // Planes through the anchor (0, 0.5, 0.5): the west face x = 0 and the
  // top face z = 0.5. `facePlane` may override a face's plane to move it
  // away from the anchor.
  const makeScene = (
    faceNormal: (face: bigint) => Float64Array,
    picks: bigint[],
    facePlane?: (face: bigint) => Float64Array,
  ) => {
    let call = 0
    const scene = makeWasmScene({ pick: () => makePick(7n, picks[Math.min(call++, picks.length - 1)]) })
    ;(scene as unknown as { face_normal: unknown }).face_normal = vi.fn((_o: bigint, f: bigint) => faceNormal(f))
    ;(scene as unknown as { face_plane: unknown }).face_plane = vi.fn((_o: bigint, f: bigint) => {
      if (facePlane) return facePlane(f)
      const n = faceNormal(f)
      return new Float64Array([0, 0.5, 0.5, n[0], n[1], n[2]])
    })
    return scene
  }
  const anchorOnWestFace = (scene: WasmScene) => {
    const { tool } = makeTool(scene)
    tool.updateDiskScale({ position: { x: -2.2, y: -2.6, z: 2.4 } } as unknown as THREE.Camera, () => 0.001)
    // The top/west edge midpoint of a unit box 0.5 high; the pick says "west face".
    tool.onPointerDown(makeSnap({ x: 0, y: 0.5, z: 0.5, kind: 'midpoint' }), rayThrough(0, 0.5))
    expect(tool.capturingInput()).toBe(true)
    return tool
  }
  const constraintNormal = (tool: LineTool) => tool.snapConstraint(rayThrough(0.5, 0.5))?.constraintPlane?.normal

  it('→ from a west-face anchor moves the chain onto the top face (the lock lies in it)', () => {
    const scene = makeScene((f) => (f === 3n ? WEST : TOP), [3n, 9n])
    const tool = anchorOnWestFace(scene)
    expect(constraintNormal(tool)).toBeUndefined() // a bare anchor is not held to a face yet
    tool.onKey({ key: 'ArrowRight' } as KeyboardEvent)
    expect(constraintNormal(tool)).toEqual([0, 0, 1])
    expect(scene.pick_face).toHaveBeenCalledTimes(2) // the anchor pick + the probe
  })

  it('↑ from a west-face anchor keeps the west face (blue lies in it)', () => {
    const scene = makeScene((f) => (f === 3n ? WEST : TOP), [3n, 9n])
    const tool = anchorOnWestFace(scene)
    tool.onKey({ key: 'ArrowUp' } as KeyboardEvent)
    expect(constraintNormal(tool)).toEqual([-1, 0, 0])
    expect(scene.pick_face).toHaveBeenCalledTimes(1) // no probe needed
  })

  it('a probe that lands on a face which cannot hold the lock leaves the chain alone', () => {
    const scene = makeScene(() => WEST, [3n, 9n])
    const tool = anchorOnWestFace(scene)
    tool.onKey({ key: 'ArrowRight' } as KeyboardEvent)
    expect(constraintNormal(tool)).toEqual([-1, 0, 0])
  })

  it('a compatible face whose plane does not pass through the anchor is not the neighbour', () => {
    // The probe ray hits a parallel floor 5 m below: right normal, wrong plane.
    const scene = makeScene(
      (f) => (f === 3n ? WEST : TOP),
      [3n, 9n],
      (f) => (f === 9n ? new Float64Array([0, 0, -5, 0, 0, 1]) : new Float64Array([0, 0.5, 0.5, -1, 0, 0])),
    )
    const tool = anchorOnWestFace(scene)
    tool.onKey({ key: 'ArrowRight' } as KeyboardEvent)
    expect(constraintNormal(tool)).toEqual([-1, 0, 0])
  })

  it('a chain with a committed segment never re-adopts', () => {
    const scene = makeScene((f) => (f === 3n ? WEST : TOP), [3n, 9n])
    const tool = anchorOnWestFace(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0.8, z: 0.5, kind: 'on-face' }), rayThrough(0, 0.8))
    tool.onKey({ key: 'ArrowRight' } as KeyboardEvent)
    expect(constraintNormal(tool)).toEqual([-1, 0, 0])
  })
})

// Playtest III: the second point decides which of a shared edge's faces the
// first click meant, with no lock involved.
describe('LineTool — the second point of a face chain decides the face', () => {
  const WEST = new Float64Array([-1, 0, 0])
  const TOP = new Float64Array([0, 0, 1])
  const makeScene = (picks: bigint[]) => {
    let call = 0
    const scene = makeWasmScene({ pick: () => makePick(7n, picks[Math.min(call++, picks.length - 1)]) })
    ;(scene as unknown as { face_normal: unknown }).face_normal = vi.fn((_o: bigint, f: bigint) => (f === 3n ? WEST : TOP))
    ;(scene as unknown as { face_plane: unknown }).face_plane = vi.fn((_o: bigint, f: bigint) =>
      f === 3n ? new Float64Array([0, 0.5, 0.5, -1, 0, 0]) : new Float64Array([0, 0.5, 0.5, 0, 0, 1]),
    )
    return scene
  }
  const constraint = (tool: LineTool) => tool.snapConstraint(rayThrough(0.5, 0.5))

  it('the first segment is not held to the adopted face; a second point on the neighbour moves the chain there', () => {
    const scene = makeScene([3n, 9n])
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0.5, z: 0.5, kind: 'midpoint' }), rayThrough(0, 0.5))
    expect(constraint(tool)?.constraintPlane).toBeUndefined() // free until the second point
    expect(constraint(tool)?.anchor).toEqual([0, 0.5, 0.5])
    // Hover a point on the TOP face (z = 0.5, x > 0): off the west plane.
    tool.onPointerMove(makeSnap({ x: 0.6, y: 0.5, z: 0.5, kind: 'on-face' }), rayThrough(0.6, 0.5))
    tool.onPointerDown(makeSnap({ x: 0.6, y: 0.5, z: 0.5, kind: 'on-face' }), rayThrough(0.6, 0.5))
    // From the second segment on, the chain is held to the top face.
    expect(constraint(tool)?.constraintPlane?.normal).toEqual([0, 0, 1])
  })

  it('a second point on neither face is projected onto the adopted face', () => {
    const scene = makeScene([3n, 9n])
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0.5, z: 0.5, kind: 'midpoint' }), rayThrough(0, 0.5))
    // A point on neither plane (x = 0.3, z = 0.2), seen along a ray that
    // crosses the west face: the click lands on the west face instead.
    const ray: Ray = { origin: [2, 0.8, 0.2], direction: [-1, 0, 0] }
    tool.onPointerDown(makeSnap({ x: 0.3, y: 0.8, z: 0.2, kind: 'endpoint' }), ray)
    expect(onToast).not.toHaveBeenCalled()
    expect(constraint(tool)?.constraintPlane?.normal).toEqual([-1, 0, 0])
  })
})

// Post-click segment retype (retypeWindow.ts): a length typed straight after
// the click resizes the segment just placed; once the pointer moves on, a
// typed length is the next segment (the chained-typing workflow).
describe('LineTool — retype the segment just placed', () => {
  function makeRetypeScene(opts: Parameters<typeof makeWasmScene>[0] = {}) {
    const base = makeWasmScene(opts) as unknown as Record<string, unknown>
    let gen = 1n
    const baseEnd = base.draft_end as (d: bigint) => string
    // The history generation moves when a draft ends (one undo step per commit).
    const scene = {
      ...base,
      history_generation: vi.fn(() => gen),
      draft_end: vi.fn((d: bigint) => { const r = baseEnd(d); gen += 1n; return r }),
      scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
      scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    }
    return scene as unknown as WasmScene
  }
  const key = (tool: LineTool, k: string) => tool.onKey({ key: k } as KeyboardEvent)
  const typeIn = (tool: LineTool, text: string) => { for (const ch of text) key(tool, ch); key(tool, 'Enter') }
  const segs = (scene: WasmScene) =>
    (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.map((c) => [c[1], c[2], c[4], c[5]] as number[])
  /** Click at world (x, y) on the ground, with the pointer parked there on screen. */
  function clickAt(tool: LineTool, x: number, y: number, px: number, py: number) {
    tool.onPointerMove(makeSnap({ x, y, z: 0 }), rayThrough(x, y))
    tool.onPointerScreenMove(px, py)
    tool.onPointerDown(makeSnap({ x, y, z: 0 }), rayThrough(x, y))
  }

  it('typing right after the second click resizes THAT segment (one undo, same direction) and the chain continues from the new end', () => {
    const scene = makeRetypeScene()
    const { tool, onCommit } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    clickAt(tool, 1, 0, 200, 100)
    expect(segs(scene)).toEqual([[0, 0, 1, 0]])
    expect(tool.statusHint()).toContain('resize the segment')
    typeIn(tool, '3')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(segs(scene).length).toBe(2)
    expect(segs(scene)[1]).toEqual([0, 0, 3, 0])
    expect(onCommit).toHaveBeenCalledTimes(2)
    // The chain now hangs off (3, 0): a click at (3, 2) draws from there.
    tool.onPointerScreenMove(240, 60)
    clickAt(tool, 3, 2, 300, 40)
    expect(segs(scene)[2]).toEqual([3, 0, 3, 2])
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
  })

  it('moving the pointer a few pixels closes the window: a typed length is then the NEXT segment along the cursor', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    clickAt(tool, 1, 0, 200, 100)
    // A resting-hand jitter keeps the window open…
    tool.onPointerScreenMove(202, 101)
    expect(tool.statusHint()).toContain('resize the segment')
    // …a deliberate move to aim the next segment closes it.
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))
    tool.onPointerScreenMove(200, 40)
    expect(tool.statusHint()).not.toContain('resize the segment')
    typeIn(tool, '2')
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(segs(scene)).toEqual([[0, 0, 1, 0], [1, 0, 1, 2]])
  })

  it('a typed (Enter) segment arms the window too, so a second value corrects it', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0))
    tool.onPointerScreenMove(200, 100)
    typeIn(tool, '2')
    expect(segs(scene)).toEqual([[0, 0, 2, 0]])
    typeIn(tool, '5')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(segs(scene)[1]).toEqual([0, 0, 5, 0])
  })

  it('a retyped middle segment restores the chain bookkeeping: the following click continues from the new end', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    clickAt(tool, 1, 0, 200, 100)
    tool.onPointerScreenMove(200, 40)
    clickAt(tool, 1, 1, 200, 30)
    expect(segs(scene)).toEqual([[0, 0, 1, 0], [1, 0, 1, 1]])
    typeIn(tool, '4')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(segs(scene)[2]).toEqual([1, 0, 1, 4])
    tool.onPointerScreenMove(300, 30)
    clickAt(tool, 5, 4, 400, 30)
    expect(segs(scene)[3]).toEqual([1, 4, 5, 4])
  })

  it('a negative typed length flips the segment to the other side of its start, like a typed new segment', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    clickAt(tool, 1, 0, 200, 100)
    typeIn(tool, '-2')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(segs(scene)[1]).toEqual([0, 0, -2, 0])
  })

  it('a zero typed length is refused with a toast that asks for a nonzero length, and the buffer clears', () => {
    const scene = makeRetypeScene()
    const { tool, onToast, onMeasurement } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    clickAt(tool, 1, 0, 200, 100)
    typeIn(tool, '0')
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledWith('A line needs a length. Type a length other than 0.')
    expect(onMeasurement).toHaveBeenLastCalledWith('')
    expect(segs(scene)).toEqual([[0, 0, 1, 0]])
  })

  it('an axis lock set after the click aims the NEXT segment: the window closes and the typed length draws along the lock', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    clickAt(tool, 1, 0, 200, 100)
    expect(tool.statusHint()).toContain('resize the segment')
    // Cursor parked (no screen move), arrow → lock Y for the next segment.
    tool.onPointerMove(makeSnap({ x: 1.2, y: 0.9, z: 0 }), rayThrough(1.2, 0.9))
    key(tool, 'ArrowLeft')
    expect(tool.statusHint()).not.toContain('resize the segment')
    typeIn(tool, '2')
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(segs(scene).length).toBe(2)
    expect(segs(scene)[1][0]).toBeCloseTo(1, 9)
    expect(segs(scene)[1][1]).toBeCloseTo(0, 9)
    expect(Math.hypot(segs(scene)[1][2] - 1, segs(scene)[1][3] - 0)).toBeCloseTo(2, 6)
  })

  it('Escape / ending the chain closes the window', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    clickAt(tool, 0, 0, 100, 100)
    clickAt(tool, 1, 0, 200, 100)
    key(tool, 'Escape')
    expect(tool.capturingInput()).toBe(false)
    expect(tool.statusHint()).not.toContain('resize')
  })
})


describe('LineTool — a clicked length is exactly the displayed length', () => {
  // These pin the Exact digits (9.399 mm); the default mm rounding is coarser.
  beforeAll(() => setPrecision('exact', 'mm'))
  afterAll(() => setPrecision(DEFAULT_PRECISION.mm, 'mm'))
  const segs = (scene: WasmScene) =>
    (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.map((c) => [c[1], c[2], c[3], c[4], c[5], c[6]] as number[])
  const prev = getLengthUnit()
  const at = (x: number, y: number, kind = 'ground') => makeSnap({ x, y, z: 0, kind })

  it('free click commits the same segment as typing the shown length', () => {
    setLengthUnit('mm')
    try {
      const ex = 0.0079, ey = 0.0071 // length 10.6217 mm, shows "10.622 mm"
      const a = makeTool(makeWasmScene())
      a.tool.onPointerDown(at(0, 0), rayThrough(0, 0))
      a.tool.onPointerMove(at(ex, ey), rayThrough(ex, ey))
      a.tool.onPointerDown(at(ex, ey), rayThrough(ex, ey))
      expect(a.onMeasurement).toHaveBeenCalledWith('10.622 mm')
      const clicked = segs((a.tool as unknown as { wasmScene: WasmScene }).wasmScene)

      const b = makeTool(makeWasmScene())
      b.tool.onPointerDown(at(0, 0), rayThrough(0, 0))
      b.tool.onPointerMove(at(ex, ey), rayThrough(ex, ey))
      for (const ch of '10.622') b.tool.onKey({ key: ch } as KeyboardEvent)
      b.tool.onKey({ key: 'Enter' } as KeyboardEvent)
      const typed = segs((b.tool as unknown as { wasmScene: WasmScene }).wasmScene)

      expect(clicked.length).toBe(1)
      expect(clicked).toEqual(typed)
      const [, , , x1, y1] = clicked[0]
      expect(Math.hypot(x1, y1)).toBeCloseTo(10.622 * 0.001, 15)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('a hard-snapped (endpoint) click keeps the snapped point exactly', () => {
    setLengthUnit('mm')
    try {
      const scene = makeWasmScene()
      const { tool } = makeTool(scene)
      tool.onPointerDown(at(0, 0), rayThrough(0, 0))
      tool.onPointerMove(at(0.0079, 0.0071, 'endpoint'), rayThrough(0.0079, 0.0071))
      tool.onPointerDown(at(0.0079, 0.0071, 'endpoint'), rayThrough(0.0079, 0.0071))
      const [, , , x1, y1] = segs(scene)[0]
      expect(x1).toBe(0.0079)
      expect(y1).toBe(0.0071)
    } finally {
      setLengthUnit(prev)
    }
  })
})

describe('LineTool — X-ray: a hidden snap off the face chain drops onto it, disclosed', () => {
  const down = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  // Runs under the mode it is named for.
  beforeEach(() => setPickRenderMode('xray'))
  afterEach(() => setPickRenderMode('shaded'))
  const segs = (scene: WasmScene) =>
    (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.map((c) => c.slice(1, 7) as number[])

  it('a later segment lands on the hidden point dropped onto the face, with the "projected" cue', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    // The idle hover over the face opts in only in X-ray.
    setPickRenderMode('xray')
    try {
      expect(tool.snapConstraint(down(0, 0))?.seeThroughPoints).toBe(true)
    } finally {
      setPickRenderMode('shaded')
    }
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), down(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 1, kind: 'on-face' }), down(1, 0))
    expect(tool.snapConstraint(down(1, 0))?.seeThroughPoints).toBe(true)
    tool.onPointerMove(makeSnap({ x: 1, y: 0.4, z: 1, kind: 'on-face' }), down(1, 0.4))
    expect(tool.snapProjected()).toBe(false)
    const hidden = makeSnap({ x: 1, y: 0.5, z: 0.6, kind: 'midpoint' })
    tool.onPointerMove(hidden, down(1.2, 0.5))
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(hidden, down(1.2, 0.5))
    const last = segs(scene).at(-1)!
    expect(last.slice(3, 6).map((v) => Number(v.toFixed(12)))).toEqual([1, 0.5, 1])
  })

  it('the first segment discloses the ray hit that replaces an off-face snap', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), down(0, 0))
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 1, kind: 'on-face' }), down(1, 0.5))
    expect(tool.snapProjected()).toBe(false)
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0.6, kind: 'midpoint' }), down(1.2, 0.5))
    expect(tool.snapProjected()).toBe(true)
  })
})

describe('LineTool — a hidden point that is no part of the face, clicked first, drops onto it; "projected" never outlives a gesture', () => {
  const down = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  // Runs under the mode it is named for.
  beforeEach(() => setPickRenderMode('xray'))
  afterEach(() => setPickRenderMode('shaded'))

  it('the idle hover and the first click over a face drop a hidden crossing onto the face, disclosed', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    const crossing = makeSnap({ x: 1, y: 0.5, z: 0.6, kind: 'intersection' })
    tool.onPointerMove(crossing, down(1.2, 0.5))
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(crossing, down(1.2, 0.5))
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(makeSnap({ x: 1.6, y: 0.9, z: 1, kind: 'on-face' }), down(1.6, 0.9))
    const first = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls[0].slice(1, 7) as number[]
    expect(first.slice(0, 3).map((v) => Number(v.toFixed(12)))).toEqual([1, 0.5, 1])
    // The far end is the displayed value (rounded to the readout), on the face.
    expect(first[5]).toBeCloseTo(1, 12)
  })

  it('the idle hover over a face opts in to hidden points only in X-ray', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    try {
      for (const [mode, expected] of [['shaded', false], ['wireframe', false], ['xray', true]] as const) {
        setPickRenderMode(mode)
        expect(tool.snapConstraint(down(0.5, 0.5))?.seeThroughPoints ?? false, mode).toBe(expected)
      }
    } finally {
      setPickRenderMode('shaded')
    }
  })

  it('after a ground chain closes on an off-plane click, the idle hover is not "projected"', () => {
    const scene = {
      ...makeWasmScene(),
      ...draftMock({ regionsCreated: (n) => (n === 2 ? [1n] : []) }).members,
    } as unknown as WasmScene
    const { tool } = makeTool(scene)
    const ground = (x: number, y: number) => makeSnap({ x, y, z: 0, kind: 'endpoint' })
    tool.onPointerDown(ground(0, 0), down(0, 0))
    tool.onPointerDown(ground(1, 0), down(1, 0))
    tool.onPointerDown(ground(1, 1), down(1, 1))
    // The closing click snaps a point above the start: its height drops.
    // An edge point, not a named point (a named point would keep its height).
    const above = makeSnap({ x: 0, y: 0, z: 0.5, kind: 'on-edge' })
    tool.onPointerMove(above, down(0, 0))
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(above, down(0, 0))
    expect(tool.snapProjected(), 'the chain closed: idle').toBe(false)
    // The close itself clears the verdict, not only the idle gate in front
    // of it: nothing stale is left for the next chain's first read.
    expect((tool as unknown as { _snapProjected: boolean })._snapProjected).toBe(false)
    tool.onPointerMove(ground(2, 2), down(2, 2))
    expect(tool.snapProjected()).toBe(false)
  })
})
