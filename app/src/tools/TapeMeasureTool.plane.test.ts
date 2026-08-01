/**
 * TapeMeasureTool — sketches on any plane (Phase 4, the sketch-planes design
 * §6 bullet 2): hover-adopting a non-ground sketch's plane, and the idle
 * arrow-key plane lock, both freeze `snapConstraint()`'s plane for the whole
 * gesture so a guide/measurement started on a tilted sketch stays in that
 * plane instead of resolving to the ground fallback and refusing.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { TapeMeasureTool } from './TapeMeasureTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** A fresh `Ray` object with `RAY`'s own values by default — the Viewport
 *  builds a new `Ray` per pointer event, and `_pickFaceFor`'s memo keys on
 *  reference IDENTITY, so reusing the shared `RAY` constant across multiple
 *  calls in one test would wrongly look like the SAME event to the memo.
 *  Tests exercising the face-anchor live-tracking fix use this instead of
 *  `RAY` for every pointer move that's meant to represent a distinct event. */
function freshRay(overrides: Partial<Ray> = {}): Ray {
  return { origin: [...RAY.origin], direction: [...RAY.direction], ...overrides }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'plane', ...overrides }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

/** A tilted sketch: the y=0 plane, normal -Y (a vertical "wall" sketch). */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, -1, 0])

/** A real `pick_face` result — mirrors `FacePickJs` (crates/wasm-api/src/
 *  lib.rs): `object`/`face`/`instance` handles, `depth()` (a world-meter
 *  ray-parameter distance — `_resolveFaceAnchor`'s adjacency gate reads it
 *  to reconstruct the hit point), and a `free()` disposer. `depth` defaults
 *  to 5 — with `RAY`'s `origin: [0,0,5], direction: [0,0,-1]`, that lands the
 *  reconstructed hit point exactly at the world origin, matching every
 *  existing test's `edgePoint: [0,0,0]` so the new adjacency gate passes
 *  without touching those tests. */
function makeFacePick(object: bigint, face: bigint, instance?: bigint, depth = 5) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    depth: () => depth,
    free: vi.fn(),
  }
}

function makeWasmScene(opts: {
  sketchPick?: bigint
  /** `pick_face`'s result (Fix 4's coverage gap — case 3 of
   *  `_resolveOffsetPlane` / `_resolveFaceAnchor`) — `undefined` (the
   *  default) means no face under the cursor, as every other test in this
   *  suite relies on. Read from `opts.facePick` on every call (not captured
   *  once), so a test can hold onto the `opts` object it passed in and
   *  mutate `opts.facePick` between pointer events to simulate the cursor
   *  moving onto/off/between faces mid-gesture (the live-tracking fix's own
   *  coverage). */
  facePick?: ReturnType<typeof makeFacePick>
  /** The picked face's RAW local-space normal, for `face_normal` — only
   *  consulted when `facePick` is set. */
  faceNormal?: [number, number, number]
} = {}) {
  const guidePoints: number[][] = []
  const planes = new Map<bigint, Float64Array>([[TILTED_SKETCH, TILTED_PLANE]])
  const scene = {
    // World-identity drawing axes (tool-parity §4) — this suite pins the
    // legacy world-axis fast paths; see drawPlane.test.ts for moved-frame
    // coverage of `axisDrawPlane` itself.
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 2, 0, 0])),
    sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 2, 0, 0])),
    add_guide_line: vi.fn(),
    add_guide_point: vi.fn((x: number, y: number, z: number) => { guidePoints.push([x, y, z]) }),
    pick_sketch: vi.fn(() => opts.sketchPick),
    sketch_plane: vi.fn((h: bigint) => planes.get(h)),
    // No face under the cursor by default (WP-4's offset-plane face-pick
    // fallback, case 3 of `_resolveOffsetPlane`) — most tests in this suite
    // don't exercise the face-pick path itself; see the Fix-4 and
    // face-anchor-release describe blocks below for real coverage of it.
    pick_face: vi.fn(() => opts.facePick),
    face_normal: vi.fn(() => new Float64Array(opts.faceNormal ?? [0, 0, 1])),
  }
  return { scene: scene as unknown as WasmScene, guidePoints }
}

/** Read the `parallel` stage's private fields (WP-4) for whitebox assertions
 *  on `offsetLock`/`offsetPlane` that have no public accessor — mirrors
 *  `RotateTool.test.ts`'s `(tool as unknown as { stage }).stage` pattern. */
function parallelStage(tool: TapeMeasureTool): {
  kind: 'parallel'
  edgePoint: [number, number, number]
  edgeDir: [number, number, number]
  origin: [number, number, number]
  offsetPlane: { origin: [number, number, number]; normal: [number, number, number]; u: [number, number, number] } | null
} {
  const stage = (tool as unknown as { stage: { kind: string } }).stage
  if (stage.kind !== 'parallel') throw new Error(`expected parallel stage, got ${stage.kind}`)
  return stage as ReturnType<typeof parallelStage>
}

function offsetLockOf(tool: TapeMeasureTool): 0 | 1 | 2 | null {
  return (tool as unknown as { offsetLock: 0 | 1 | 2 | null }).offsetLock
}

/** Read `_shiftAxisLock` (WP-7 item 2) — no public accessor. */
function shiftAxisLockOf(tool: TapeMeasureTool): boolean {
  return (tool as unknown as { _shiftAxisLock: boolean })._shiftAxisLock
}

/** Read `_shiftLatchPending` (WP-7 item 2) — no public accessor. */
function shiftLatchPendingOf(tool: TapeMeasureTool): boolean {
  return (tool as unknown as { _shiftLatchPending: boolean })._shiftLatchPending
}

/** Read the `measure` stage's own mid-gesture axis lock (WP-5) — no public
 *  accessor, mirrors `offsetLockOf` above. */
function lockAxisOf(tool: TapeMeasureTool): 0 | 1 | 2 | null {
  return (tool as unknown as { lockAxis: 0 | 1 | 2 | null }).lockAxis
}

/** Read the `measure` stage's own fields for whitebox assertions (WP-5) —
 *  mirrors `parallelStage` above. */
function measureStage(tool: TapeMeasureTool): {
  kind: 'measure'
  p0: [number, number, number]
  p0OnGeometry: boolean
  p1: [number, number, number]
  onGeometry: boolean
} {
  const stage = (tool as unknown as { stage: { kind: string } }).stage
  if (stage.kind !== 'measure') throw new Error(`expected measure stage, got ${stage.kind}`)
  return stage as ReturnType<typeof measureStage>
}

function makeTool(scene: WasmScene) {
  const onGuideCreated = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const onRescaleArmed = vi.fn()
  const onRescaleApplied = vi.fn()
  const tool = new TapeMeasureTool(
    scene, new THREE.Group(), onGuideCreated, onToast, onMeasurement, onRescaleArmed, onRescaleApplied,
  )
  return { tool, onGuideCreated, onToast, onMeasurement, onRescaleArmed, onRescaleApplied }
}

/** Feed a plain digit/decimal string then Enter into the tool's VCB —
 *  mirrors TapeMeasureTool.test.ts's own `typeAndEnter`. */
function typeAndEnter(tool: TapeMeasureTool, buf: string): void {
  for (const ch of buf) tool.onKey(makeKeyEvent(ch))
  tool.onKey(makeKeyEvent('Enter'))
}

describe('TapeMeasureTool — hover-adopt a non-ground sketch plane', () => {
  it('idle snapConstraint returns the hovered sketch plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    const constraint = tool.snapConstraint(RAY)
    expect(constraint).toEqual({ constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] } })
  })

  it('idle snapConstraint is null with no sketch under the cursor', () => {
    const { scene } = makeWasmScene({ sketchPick: undefined })
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('measuring between two points on the hovered sketch stays constrained to its plane through the second click', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    // First click over the tilted sketch (kind: 'plane' — a Phase 1 fallback
    // snap landing on the constraint plane still counts as "on the sketch",
    // not on-geometry, per snapOnGeometry's kind check).
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2, kind: 'plane' }), RAY)
    expect(tool.capturingInput()).toBe(true)

    // Mid-gesture: snapConstraint now returns the FROZEN plane regardless of
    // what's currently under the cursor, plus (WP-5) `anchor: p0` and
    // `offPlanePoints: true` — the plane no longer filters, it projects.
    expect(tool.snapConstraint(RAY)).toEqual({
      anchor: [1, 0, 2],
      constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] },
      offPlanePoints: true,
    })

    tool.onPointerMove(makeSnap({ x: 3, y: 0, z: 5, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5, kind: 'plane' }), RAY) // second click commits

    // Both endpoints landed via the 'plane' fallback (not on real geometry)
    // — SketchUp/snapOnGeometry semantics: a guide POINT is dropped at the
    // second (measure-mode) endpoint, per _commitMeasure's onGeometry check.
    expect(scene.add_guide_point).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false) // gesture ended
  })

  it('the frozen plane clears once the gesture ends — the NEXT gesture re-resolves it', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5, kind: 'plane' }), RAY) // commit, back to idle

    // Idle again: snapConstraint re-derives from the CURRENT hover, not a
    // stale frozen plane.
    expect(tool.snapConstraint(RAY)).toEqual({ constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] } })
  })
})

describe('TapeMeasureTool — idle arrow-key plane lock (design §6 bullet 2)', () => {
  it('an arrow key locks the plane while idle, named in statusHint', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    expect(tool.statusHint()).not.toContain('Locked')

    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.statusHint()).toContain('Locked to the red plane')
  })

  it('pressing the same arrow again clears the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.statusHint()).not.toContain('Locked')
  })

  it('idle snapConstraint is FREE (unconstrained) while locked — the plane derives from the click', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.snapConstraint(RAY)).toBeNull()
    expect(scene.pick_sketch).not.toHaveBeenCalled() // lock beats sketch-hover adoption
  })

  it('the first click freezes the locked axis plane through the clicked point', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3 }), RAY)

    // Plain (non-edge) snap → measure stage: `anchor`/`offPlanePoints` (WP-5)
    // ride along with the frozen plane.
    expect(tool.snapConstraint(RAY)).toEqual({
      anchor: [2, 1, 3],
      constraintPlane: { point: [2, 1, 3], normal: [1, 0, 0] },
      offPlanePoints: true,
    })
  })

  it('Escape while idle-locked clears the lock first; a second Escape does not throw', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.statusHint()).toContain('Locked')
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.statusHint()).not.toContain('Locked')
    expect(() => tool.onKey(makeKeyEvent('Escape'))).not.toThrow()
  })

  it('Escape aborting an anchored-but-uncommitted gesture preserves the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3 }), RAY) // anchor only
    expect(tool.capturingInput()).toBe(true)
    tool.onKey(makeKeyEvent('Escape')) // abort the gesture, keep the aim
    expect(tool.capturingInput()).toBe(false)
    expect(tool.statusHint()).toContain('Locked to the red plane')
  })

  it('cancel() clears the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.cancel()
    expect(tool.statusHint()).not.toContain('Locked')
  })

  it('the lock survives a completed gesture', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 4, z: 6 }), RAY) // commit (measure, drops a guide point)

    expect(tool.statusHint()).toContain('Locked to the red plane')
  })
})

describe('TapeMeasureTool — parallel-guide vs. frozen plane, the edge wins (Blocker 2)', () => {
  it('edge IN the hover-adopted tilted-sketch plane: constraint kept, committed origin stays on-plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    // The default edge_endpoints mock is [0,0,0, 2,0,0] — direction [1,0,0],
    // which IS perpendicular to TILTED_SKETCH's normal [0,-1,0], and the
    // picked point (y=0) sits ON that plane too — the hover-adopted case.
    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    expect(tool.capturingInput()).toBe(true)

    // Constraint survives — the edge agrees with the frozen plane. WP-4:
    // `parallel`-stage snapConstraint now reports the constraint plane
    // through `edgePoint` (not the sketch's own origin) plus an `anchor`,
    // per `_resolveOffsetPlane`'s new contract — the NORMAL is still
    // exactly the frozen plane's, unchanged (see the "reduces to today's
    // plane-freeze behavior" coverage in the offset-plane describe block).
    // `offPlanePoints: true` rides along too (the report-2 fix) — every
    // plane-yielding case now discloses off-plane reachability, never a
    // hard filter.
    expect(tool.snapConstraint(RAY)).toEqual({
      anchor: [1, 0, 0],
      constraintPlane: { point: [1, 0, 0], normal: [0, -1, 0] },
      offPlanePoints: true,
    })

    // Drag the cursor to another point that itself stays on the plane
    // (y=0) — the resulting guide origin must too.
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3, kind: 'plane' }), RAY) // commit

    const wasm = scene as unknown as { add_guide_line: ReturnType<typeof vi.fn> }
    expect(wasm.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = wasm.add_guide_line.mock.calls[0] as number[]
    expect(oy).toBe(0) // origin stayed on the y=0 plane
    expect([ox, oz]).toEqual([1, 3])
    expect([dx, dy, dz]).toEqual([1, 0, 0]) // parallel to the source edge
  })

  it('idle-lock plane + edge NOT in that plane: the edge wins, gesture is unconstrained (legacy behavior)', () => {
    const { scene } = makeWasmScene() // no sketch under the cursor
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle-locks the red/X plane (normal [1,0,0])

    // The default edge_endpoints mock is [0,0,0, 2,0,0] — direction [1,0,0],
    // which is NOT perpendicular to the locked plane's normal [1,0,0]
    // (dot = 1) — the edge disagrees with the idle lock.
    tool.onPointerDown(
      makeSnap({ x: 2, y: 1, z: 3, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    expect(tool.capturingInput()).toBe(true)

    // The edge wins: the frozen plane is dropped for this gesture — no
    // constraintPlane at all (legacy unconstrained parallel-guide behavior).
    expect(tool.snapConstraint(RAY)).toBeNull()

    // Legacy offset behavior still works normally: perpComponent of the
    // cursor relative to the edge direction, unconstrained by any plane.
    tool.onPointerMove(makeSnap({ x: 2, y: 5, z: 3, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 5, z: 3, kind: 'plane' }), RAY) // commit

    const wasm = scene as unknown as { add_guide_line: ReturnType<typeof vi.fn> }
    expect(wasm.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = wasm.add_guide_line.mock.calls[0] as number[]
    // edgePoint (2,1,3) + perp((cursor-edgePoint), edgeDir=[1,0,0]) = (2,1,3) + (0,4,0) = (2,5,3)
    expect([ox, oy, oz]).toEqual([2, 5, 3])
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })
})

const dot3 = (a: [number, number, number], b: [number, number, number]): number =>
  a[0] * b[0] + a[1] * b[1] + a[2] * b[2]

describe('TapeMeasureTool — offset plane derived from a frozen plane lock (WP-4)', () => {
  it('an idle plane lock on a tilted edge that used to be DROPPED now derives a valid offset plane', () => {
    const { scene } = makeWasmScene() // no sketch hovered — the idle arrow lock supplies `_gesturePlane`
    const { tool } = makeTool(scene)
    ;(scene as unknown as { edge_endpoints: ReturnType<typeof vi.fn> }).edge_endpoints
      .mockReturnValue(new Float64Array([0, 0, 0, 1, 1, 0])) // tilted edge, dir [1,1,0]/√2

    tool.onKey(makeKeyEvent('ArrowRight')) // idle-locks the red/X plane (normal [1,0,0])
    tool.onPointerDown(
      makeSnap({ x: 2, y: 3, z: 4, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    const stage = parallelStage(tool)
    expect(stage.offsetPlane).not.toBeNull()
    const { normal, u } = stage.offsetPlane!
    const d = stage.edgeDir

    expect(dot3(normal, d)).toBeCloseTo(0, 9)
    expect(dot3(normal, u)).toBeCloseTo(0, 9)
    expect(Math.hypot(normal[0], normal[1], normal[2])).toBeCloseTo(1, 9)
    expect(Math.hypot(u[0], u[1], u[2])).toBeCloseTo(1, 9)

    // The OLD behavior would have dropped the constraint entirely (the edge
    // doesn't lie in the locked red plane) — the new one still constrains.
    expect(tool.snapConstraint(RAY)).not.toBeNull()
  })

  it('reduces to exactly today\'s plane-freeze behavior when the edge already lies in the locked plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH }) // normal [0,-1,0]
    const { tool } = makeTool(scene)
    // Default edge_endpoints: [0,0,0,2,0,0] → dir [1,0,0], already ⊥ [0,-1,0].
    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    const stage = parallelStage(tool)
    expect(stage.offsetPlane).not.toBeNull()
    // The normal is preserved UNCHANGED from the frozen plane's own normal —
    // `offsetPlaneNormal`'s documented identity when already perpendicular.
    expect(stage.offsetPlane!.normal).toEqual([0, -1, 0])
  })
})

// Fix 4 + coverage gap: `_resolveOffsetPlane` case 3 (a face picked under
// the gesture's first-click ray) had ZERO test coverage anywhere — every
// other test in this file/suite stubs `pick_face` to return `undefined`.
// Case 3 also used to store the RAW picked-face normal unprojected, unlike
// cases 1/2, which both explicitly project onto the plane perpendicular to
// the edge — leaving `offsetPlane.normal` off by up to `AXIS_ALONG_EDGE_SIN`
// (~3°) from the module doc's claimed "every case's normal is ⊥ d"
// invariant.
describe('TapeMeasureTool — face-pick offset-plane normal is exactly perpendicular to the edge (Fix 4)', () => {
  it('case 3 engages when no arrow lock and no frozen idle plane apply, and yields an EXACTLY-perpendicular normal', () => {
    const RAW_FACE_NORMAL: [number, number, number] = [0.02998649, 0, 0.99955043] // ~1.7° off ⊥ to [1,0,0]
    const { scene: sceneWithFace } = makeWasmScene({
      facePick: makeFacePick(9n, 4n),
      faceNormal: RAW_FACE_NORMAL,
    })
    const { tool } = makeTool(sceneWithFace)

    // Default edge: dir [1,0,0] through the origin (edge_endpoints' default).
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    expect(sceneWithFace.pick_face).toHaveBeenCalledTimes(1) // case 3 actually ran
    const stage = parallelStage(tool)
    expect(stage.offsetPlane).not.toBeNull() // case 3 engaged (no lock, no frozen plane available)
    const { normal, u } = stage.offsetPlane!
    const d = stage.edgeDir

    // (b) the resolved `normal` is now EXACTLY perpendicular to `d` — not
    // the raw, ~1.7°-off face normal `face_normal` reported.
    expect(dot3(normal, d)).toBeCloseTo(0, 9)
    expect(normal).not.toEqual(RAW_FACE_NORMAL)
    expect(Math.hypot(normal[0], normal[1], normal[2])).toBeCloseTo(1, 9)

    // (c) `u` is exactly perpendicular to `d` regardless (true both before
    // and after Fix 4 — `u` is always derived via a cross product with `d`).
    expect(dot3(u, d)).toBeCloseTo(0, 9)
    expect(Math.hypot(u[0], u[1], u[2])).toBeCloseTo(1, 9)
  })

  it('a face normal beyond the along-edge tolerance is rejected — falls through to the free 2-D offset (no plane)', () => {
    const { scene } = makeWasmScene({
      facePick: makeFacePick(9n, 4n),
      faceNormal: [1, 0, 0], // exactly along the default edge direction [1,0,0] — not viable
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    expect(scene.pick_face).toHaveBeenCalledTimes(1)
    expect(parallelStage(tool).offsetPlane).toBeNull() // rejected — no viable plane source at all
  })
})

/** Read the `parallel` stage's `faceAnchor` field — no public accessor. */
function faceAnchorOf(tool: TapeMeasureTool): unknown {
  return (tool as unknown as { stage: { faceAnchor: unknown } }).stage.faceAnchor
}

// The face-anchor LIVE-tracking fix (Kurt's report 1): the OLD `_resolveOffsetPlane`
// case 3 re-picked a face against a `firstClickRay` FIXED for the whole
// gesture, so once a face was found there it never changed again no matter
// where the cursor wandered afterward — dragging toward a second object
// showed every point as "on plane" with nothing on the second object ever
// reachable. The fix resolves the anchor ONCE at the first click but
// re-tests it against the LIVE cursor ray on every move/lock-change/
// snapConstraint call, engaging only while the ray still lands on that SAME
// face and releasing the instant it doesn't — never re-adopting a
// different face.
describe('TapeMeasureTool — face-anchor live-tracking (release/re-engage, Fix: report 1)', () => {
  it('case 3 engages on the first click\'s face, then RELEASES the moment a later move picks a DIFFERENT face', () => {
    const opts: { facePick?: ReturnType<typeof makeFacePick>; faceNormal?: [number, number, number] } = {
      facePick: makeFacePick(9n, 4n),
      faceNormal: [0, 0, 1],
    }
    const { scene } = makeWasmScene(opts)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      freshRay(),
    )
    expect(parallelStage(tool).offsetPlane).not.toBeNull() // engaged

    // A later move whose ray picks a DIFFERENT face (face 5n, not 4n).
    opts.facePick = makeFacePick(9n, 5n)
    tool.onPointerMove(makeSnap({ x: 0, y: 5, z: 0, kind: 'plane' }), freshRay())

    expect(parallelStage(tool).offsetPlane).toBeNull() // released — falls back to free 2-D offset
    // Free perpComponent of (0,5,0) off edgeDir [1,0,0], from edgePoint [0,0,0]:
    expect(parallelStage(tool).origin).toEqual([0, 5, 0])
  })

  it('case 3 releases when a later move\'s ray picks no face at all (pick_face → undefined)', () => {
    const opts: { facePick?: ReturnType<typeof makeFacePick>; faceNormal?: [number, number, number] } = {
      facePick: makeFacePick(9n, 4n),
      faceNormal: [0, 0, 1],
    }
    const { scene } = makeWasmScene(opts)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      freshRay(),
    )
    expect(parallelStage(tool).offsetPlane).not.toBeNull()

    opts.facePick = undefined // cursor drifted off any face entirely
    tool.onPointerMove(makeSnap({ x: 0, y: 5, z: 0, kind: 'plane' }), freshRay())

    expect(parallelStage(tool).offsetPlane).toBeNull()
    expect(scene.pick_face).toHaveBeenCalledTimes(2) // one per distinct ray — memo works, no extra calls
  })

  it('case 3 RE-ENGAGES if the cursor returns to the originally anchored face — release is not one-way/sticky', () => {
    const opts: { facePick?: ReturnType<typeof makeFacePick>; faceNormal?: [number, number, number] } = {
      facePick: makeFacePick(9n, 4n),
      faceNormal: [0, 0, 1],
    }
    const { scene } = makeWasmScene(opts)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      freshRay(),
    )
    const engagedPlane = parallelStage(tool).offsetPlane
    expect(engagedPlane).not.toBeNull()

    opts.facePick = undefined // move off the face — released
    tool.onPointerMove(makeSnap({ x: 0, y: 5, z: 0, kind: 'plane' }), freshRay())
    expect(parallelStage(tool).offsetPlane).toBeNull()

    opts.facePick = makeFacePick(9n, 4n) // move back onto the SAME anchored face
    tool.onPointerMove(makeSnap({ x: 0, y: 7, z: 0, kind: 'plane' }), freshRay())
    expect(parallelStage(tool).offsetPlane).toEqual(engagedPlane) // re-engaged, same plane
  })

  it('the SAME object/face but a DIFFERENT instance handle does not match the anchor — plane not engaged', () => {
    const opts: { facePick?: ReturnType<typeof makeFacePick>; faceNormal?: [number, number, number] } = {
      facePick: makeFacePick(9n, 4n), // instance undefined → null
      faceNormal: [0, 0, 1],
    }
    const { scene } = makeWasmScene(opts)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      freshRay(),
    )
    expect(parallelStage(tool).offsetPlane).not.toBeNull()

    opts.facePick = makeFacePick(9n, 4n, 77n) // same object/face, different instance
    tool.onPointerMove(makeSnap({ x: 0, y: 5, z: 0, kind: 'plane' }), freshRay())

    expect(parallelStage(tool).offsetPlane).toBeNull() // instance mismatch — not the same anchor
  })

  it('adjacency gate: a face parallel to the edge but whose hit point is far from it never becomes engageable for the whole gesture', () => {
    const opts: { facePick?: ReturnType<typeof makeFacePick>; faceNormal?: [number, number, number] } = {
      // faceNormal [0,0,1] IS perpendicular to edgeDir [1,0,0] (passes the
      // parallelism gate) but depth 15 puts the reconstructed hit point at
      // (0,0,5-15) = (0,0,-10) — far from edgePoint (0,0,0) along that same
      // normal, so the NEW adjacency gate rejects it (the real gap the
      // design review found: a face merely parallel to the edge, but not
      // actually passing through/near it).
      facePick: makeFacePick(9n, 4n, undefined, 15),
      faceNormal: [0, 0, 1],
    }
    const { scene } = makeWasmScene(opts)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      freshRay(),
    )
    expect(faceAnchorOf(tool)).toBeNull() // rejected at anchor-resolution time
    expect(parallelStage(tool).offsetPlane).toBeNull()

    // Even a later move whose ray picks the EXACT SAME face never engages it —
    // the anchor itself was never established, so case 3 has nothing to
    // re-test against for the rest of the gesture.
    tool.onPointerMove(makeSnap({ x: 0, y: 5, z: 0, kind: 'plane' }), freshRay())
    expect(parallelStage(tool).offsetPlane).toBeNull()
  })

  it('continuity: at a point exactly in the anchored face\'s plane, origin is numerically identical whether case 3 is engaged or released (no pop at the release boundary)', () => {
    const opts: { facePick?: ReturnType<typeof makeFacePick>; faceNormal?: [number, number, number] } = {
      facePick: makeFacePick(9n, 4n),
      faceNormal: [0, 0, 1], // already exactly ⊥ edgeDir [1,0,0] — u works out to [0,1,0]
    }
    const { scene } = makeWasmScene(opts)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      freshRay(),
    )
    expect(parallelStage(tool).offsetPlane).not.toBeNull() // engaged

    // Cursor at (0,7,0) — z=0, exactly on the anchored plane (through the
    // origin, normal [0,0,1]) — so the plane-projected offset and the
    // free-offset perpComponent must agree exactly.
    tool.onPointerMove(makeSnap({ x: 0, y: 7, z: 0, kind: 'plane' }), freshRay())
    const engagedOrigin = parallelStage(tool).origin

    opts.facePick = undefined // release
    tool.onPointerMove(makeSnap({ x: 0, y: 7, z: 0, kind: 'plane' }), freshRay())
    expect(parallelStage(tool).offsetPlane).toBeNull() // confirm actually released
    const releasedOrigin = parallelStage(tool).origin

    expect(releasedOrigin[0]).toBeCloseTo(engagedOrigin[0], 9)
    expect(releasedOrigin[1]).toBeCloseTo(engagedOrigin[1], 9)
    expect(releasedOrigin[2]).toBeCloseTo(engagedOrigin[2], 9)
    expect(engagedOrigin).toEqual([0, 7, 0])
  })
})

// offPlanePoints (Fix: report 2) — the parallel stage's snapConstraint()
// never set this in any of its three plane-yielding cases, unlike `measure`
// stage, so a resolved plane was always a hard FILTER with no projection
// escape hatch: an off-plane point (e.g. a vertex atop a different box) was
// simply unreachable once ANY plane/axis was locked. Every case now sets it,
// and NEVER combines it with a `lockAxis` field (regression guard against
// reopening the axis/plane-collapse class of bug from the earlier review).
describe('TapeMeasureTool — offPlanePoints in every parallel-stage plane-yielding case (Fix: report 2)', () => {
  it('case 1 (arrow-locked axis): offPlanePoints true, lockAxis absent', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y lock

    const constraint = tool.snapConstraint(RAY)
    expect(constraint?.offPlanePoints).toBe(true)
    expect(constraint?.constraintPlane).toEqual({ point: [0, 0, 0], normal: [0, 0, 1] })
    expect(constraint).not.toHaveProperty('lockAxis')
  })

  it('case 2 (frozen idle-plane-lock-derived plane): offPlanePoints true, lockAxis absent', () => {
    const { scene } = makeWasmScene() // no sketch hovered — the idle arrow lock supplies `_gesturePlane`
    const { tool } = makeTool(scene)
    ;(scene as unknown as { edge_endpoints: ReturnType<typeof vi.fn> }).edge_endpoints
      .mockReturnValue(new Float64Array([0, 0, 0, 1, 1, 0])) // tilted edge, dir [1,1,0]/√2

    tool.onKey(makeKeyEvent('ArrowRight')) // idle-locks the red/X plane
    tool.onPointerDown(
      makeSnap({ x: 2, y: 3, z: 4, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    const constraint = tool.snapConstraint(RAY)
    expect(constraint?.offPlanePoints).toBe(true)
    expect(constraint?.constraintPlane).not.toBeUndefined()
    expect(constraint).not.toHaveProperty('lockAxis')
  })

  it('case 3 (face anchor): offPlanePoints true, lockAxis absent', () => {
    const { scene } = makeWasmScene({
      facePick: makeFacePick(9n, 4n),
      faceNormal: [0.02998649, 0, 0.99955043], // ~1.7° off ⊥ to [1,0,0], same fixture as the Fix-4 tests
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      freshRay(),
    )

    const constraint = tool.snapConstraint(freshRay())
    expect(constraint?.offPlanePoints).toBe(true)
    expect(constraint?.constraintPlane).not.toBeUndefined()
    expect(constraint).not.toHaveProperty('lockAxis')
  })
})

// End-to-end coverage of Kurt's report 2 itself: once ANY plane/axis is
// locked in parallel mode, an off-plane point used to be simply unreachable
// (shown as "on plane" with no way to actually use it). Now it contributes
// its scalar projection along the offset direction instead of being
// filtered out, and the chip discloses the projection via snapProjected().
describe('TapeMeasureTool — off-plane point reachable via projection in parallel mode (Fix: report 2, end-to-end)', () => {
  it('an axis-locked offset still reaches an elevated off-plane point, at its correct scalar projection, with snapProjected() true', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    // Ground-level edge: edgePoint [0,0,0], edgeDir [1,0,0] (edge_endpoints' default).
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onKey(makeKeyEvent('ArrowLeft')) // arrow-lock to green/Y — offset plane is z=0, u=[0,1,0]
    expect(offsetLockOf(tool)).toBe(1)

    // A point NOT on the resulting plane (elevated in Z) — e.g. a vertex atop a different box.
    tool.onPointerMove(makeSnap({ x: 2, y: 3, z: 5, kind: 'endpoint' }), RAY)

    // Correct scalar projection along u=[0,1,0]: dot((2,3,5),(0,1,0)) = 3 — not zero, not filtered out.
    expect(parallelStage(tool).origin).toEqual([0, 3, 0])
    expect(tool.snapProjected()).toBe(true)
  })
})

describe('TapeMeasureTool — snapProjected() false cases in parallel mode', () => {
  it('false when the snap point lies exactly on the resolved plane', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onKey(makeKeyEvent('ArrowLeft')) // plane z=0, u=[0,1,0]

    tool.onPointerMove(makeSnap({ x: 2, y: 3, z: 0, kind: 'endpoint' }), RAY) // z=0 — exactly on-plane
    expect(tool.snapProjected()).toBe(false)
  })

  it('false when no plane is resolved at all (free-offset mode)', () => {
    const { scene } = makeWasmScene() // no lock, no gesture plane, no face pick
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    expect(parallelStage(tool).offsetPlane).toBeNull()

    tool.onPointerMove(makeSnap({ x: 2, y: 3, z: 5, kind: 'endpoint' }), RAY)
    expect(tool.snapProjected()).toBe(false)
  })
})

describe('TapeMeasureTool — mid-gesture offset-axis lock, arrow keys (WP-4)', () => {
  it('a viable arrow key locks the offset to that axis; origin then moves in a straight line along it', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    // Default edge: dir [1,0,0] (red/X) through the origin.
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y lock
    expect(offsetLockOf(tool)).toBe(1)

    tool.onPointerMove(makeSnap({ x: 5, y: 3, z: 9, kind: 'plane' }), RAY)
    expect(parallelStage(tool).origin).toEqual([0, 3, 0]) // only the cursor's Y component matters

    tool.onPointerMove(makeSnap({ x: -8, y: 7, z: 2, kind: 'plane' }), RAY)
    expect(parallelStage(tool).origin).toEqual([0, 7, 0]) // still a straight line along green/Y
  })

  it('pressing a NON-viable arrow (axis runs along the source edge) is a no-op on offsetLock but updates the status hint', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    ;(scene as unknown as { edge_endpoints: ReturnType<typeof vi.fn> }).edge_endpoints
      .mockReturnValue(new Float64Array([0, 0, 0, 0, 2, 0])) // edge along green/Y

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    const before = tool.statusHint()

    tool.onKey(makeKeyEvent('ArrowLeft')) // green — runs along this edge

    expect(offsetLockOf(tool)).toBeNull() // no-op on the lock itself
    expect(tool.statusHint()).not.toBe(before)
    expect(tool.statusHint()).toContain('Green runs along this edge')
    expect(tool.statusHint()).toContain('lock red or blue instead')
  })

  it('pressing the same locked axis again releases the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    tool.onKey(makeKeyEvent('ArrowLeft'))
    expect(offsetLockOf(tool)).toBe(1)
    tool.onKey(makeKeyEvent('ArrowLeft'))
    expect(offsetLockOf(tool)).toBeNull()
  })

  it('pressing ArrowDown releases the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )

    tool.onKey(makeKeyEvent('ArrowLeft'))
    expect(offsetLockOf(tool)).toBe(1)
    tool.onKey(makeKeyEvent('ArrowDown'))
    expect(offsetLockOf(tool)).toBeNull()
  })
})

describe('TapeMeasureTool — typed offset commits along the resolved offset direction (WP-4)', () => {
  it('commits along the resolved u, not a perturbed (floating-point-drift) direction', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    ) // edgeDir [1,0,0], edgePoint [0,0,0]
    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y lock, u = [0,1,0]
    tool.onPointerMove(makeSnap({ x: 0, y: 5, z: 0, kind: 'plane' }), RAY)

    // Simulate accumulated floating-point drift: `origin` picks up a stray
    // x/z component that ISN'T a clean multiple of `u`, without touching
    // `offsetPlane.u` itself — the naive `rel/|rel|` direction (rel = origin
    // − edgePoint) would carry that drift into the commit; `sign(dot(rel,
    // u))·u` must not.
    const stage = parallelStage(tool)
    stage.origin = [1e-4, 5, -2e-4]

    tool.onKey(makeKeyEvent('5'))
    tool.onKey(makeKeyEvent('Enter'))

    const wasm = scene as unknown as { add_guide_line: ReturnType<typeof vi.fn> }
    expect(wasm.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = wasm.add_guide_line.mock.calls[0] as number[]
    expect([dx, dy, dz]).toEqual([1, 0, 0]) // the guide line's own direction is always edgeDir

    // The naive rel/|rel| direction (from the perturbed origin) would have
    // carried nonzero x/z components (~2e-5 scale) into a 5-unit commit;
    // sign(dot(rel,u))·u lands EXACTLY on the u axis instead.
    expect(ox).toBe(0)
    expect(oz).toBe(0)
    expect(oy).toBeCloseTo(5, 9)
  })

  // Fix 2: the `offsetPlane !== null` branch used to have no degenerate
  // guard — with zero offset (cursor never moved off the edge, e.g. an axis
  // lock set immediately after the first click), `dot(rel, u) < 0` defaults
  // to `false` for a zero `rel`, so `sign` came out `+1` and the typed
  // distance committed along an arbitrary side instead of no-op'ing like the
  // free-offset branch already did for the same input.
  it('a resolved offset plane/axis with zero offset (no mouse move before locking) no-ops instead of committing to an arbitrary side', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    ) // edgeDir [1,0,0], edgePoint [0,0,0]
    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y lock, set BEFORE any pointer move
    expect(offsetLockOf(tool)).toBe(1)
    expect(parallelStage(tool).offsetPlane).not.toBeNull() // a resolved offset plane, zero offset

    typeAndEnter(tool, '5')

    expect(tool.capturingInput()).toBe(false) // reset to idle
    const wasm = scene as unknown as { add_guide_line: ReturnType<typeof vi.fn> }
    expect(wasm.add_guide_line).not.toHaveBeenCalled() // no arbitrary-side commit
  })

  it('regression: the pre-existing free-offset degenerate guard (no resolved plane) still no-ops unchanged', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    ) // no axis lock, no frozen plane, no face pick — offsetPlane stays null
    expect(parallelStage(tool).offsetPlane).toBeNull()

    typeAndEnter(tool, '5')

    expect(tool.capturingInput()).toBe(false)
    const wasm = scene as unknown as { add_guide_line: ReturnType<typeof vi.fn> }
    expect(wasm.add_guide_line).not.toHaveBeenCalled()
  })
})

describe('TapeMeasureTool — mid-gesture axis lock in measure stage (WP-5)', () => {
  it('a viable arrow key locks the distance to that axis — every arrow is accepted, no rejection case', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y
    expect(lockAxisOf(tool)).toBe(1)
  })

  it('pressing the same locked axis again releases the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)

    tool.onKey(makeKeyEvent('ArrowLeft'))
    expect(lockAxisOf(tool)).toBe(1)
    tool.onKey(makeKeyEvent('ArrowLeft'))
    expect(lockAxisOf(tool)).toBeNull()
  })

  it('pressing ArrowDown releases the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)

    tool.onKey(makeKeyEvent('ArrowUp')) // blue/Z
    expect(lockAxisOf(tool)).toBe(2)
    tool.onKey(makeKeyEvent('ArrowDown'))
    expect(lockAxisOf(tool)).toBeNull()
  })

  it('a locked axis makes snapConstraint() return null — the lock stays entirely TS-side, never reaching the kernel', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'endpoint' }), RAY)

    tool.onKey(makeKeyEvent('ArrowRight')) // red/X
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('the status hint names the locked axis, mirroring the parallel-stage offset-lock hint', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.statusHint()).not.toContain('locked')

    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.statusHint()).toContain('Measurement locked to red')
  })

  it('the lock is scoped to one gesture — cleared once the gesture commits', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(lockAxisOf(tool)).toBe(0)

    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY) // second click commits
    expect(lockAxisOf(tool)).toBeNull()
  })
})

describe('TapeMeasureTool — plane-lock implies projection in measure stage (WP-5)', () => {
  it('a frozen plane lock causes snapConstraint() to include offPlanePoints: true', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2, kind: 'plane' }), RAY)
    const constraint = tool.snapConstraint(RAY)
    expect(constraint?.offPlanePoints).toBe(true)
    expect(constraint?.constraintPlane).toEqual({ point: [0, 0, 0], normal: [0, -1, 0] })
  })

  it('an off-plane snap is projected onto the frozen plane — p1 reflects the projected point, snapProjected() is true', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY) // p0, exactly on the plane already
    expect(tool.snapProjected()).toBe(false)

    tool.onPointerMove(makeSnap({ x: 3, y: 4, z: 5, kind: 'endpoint' }), RAY) // off-plane real geometry

    expect(measureStage(tool).p1).toEqual([3, 0, 5]) // projected onto y=0, x/z preserved
    expect(tool.snapProjected()).toBe(true)
  })

  it('the SAME off-plane snap with no plane frozen leaves the point untouched — snapProjected() is false (angled-measurement behavior)', () => {
    const { scene } = makeWasmScene() // no sketch under the cursor — no frozen plane
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 3, y: 4, z: 5, kind: 'endpoint' }), RAY)

    expect(measureStage(tool).p1).toEqual([3, 4, 5]) // untouched
    expect(tool.snapProjected()).toBe(false)
  })

  it('regression: the rescale-arm guard reads the RAW snap\'s on-geometry-ness, unaffected by projection', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onRescaleArmed } = makeTool(scene)

    // p0: real geometry, exactly on the frozen plane already (no projection).
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    // p1: ALSO real geometry, but off the frozen plane — gets projected to
    // (3, 0, 5), a point that coincides with no actual geometry at all. The
    // arm guard must still treat it as resting on real geometry, because
    // that's what the RAW snap (kind: 'endpoint') actually was.
    tool.onPointerMove(makeSnap({ x: 3, y: 4, z: 5, kind: 'endpoint' }), RAY)
    expect(tool.snapProjected()).toBe(true)
    expect(measureStage(tool).onGeometry).toBe(true)

    typeAndEnter(tool, '10')

    // Armed exactly as it would be without any projection — using the
    // PROJECTED distance (p0 → (3, 0, 5) = √(3² + 5²) = √34) for the
    // "current distance", since that's genuinely where the measurement now
    // reads from/to; only the on-geometry VERDICT is unaffected.
    expect(onRescaleArmed).toHaveBeenCalledTimes(1)
    const info = onRescaleArmed.mock.calls[0][0]
    expect(info.currentDistance).toBeCloseTo(Math.sqrt(34))
  })
})

// Fix: a mid-gesture axis lock (`lockAxis`) collinear with a frozen
// `_gesturePlane`'s normal used to silently collapse the measured distance
// to zero — `snapConstraint()` combined BOTH `lockAxis` and
// `constraintPlane`/`offPlanePoints`, so the kernel handed back points
// purely along the locked axis, and `_measureTarget` then projected every
// one of them straight back onto the plane (whose normal IS that axis),
// landing exactly on `p0` regardless of where the cursor actually was. The
// later locked-axis-projection fix (`_measurePoint`) generalizes this: an
// active `lockAxis` now ALWAYS makes `snapConstraint()` return null
// (dropping `constraintPlane`/`offPlanePoints` along with everything else,
// collinear with the frozen plane or not — see the doc on `snapConstraint`),
// with the axial station computed entirely in TS from a free kernel snap.
describe('TapeMeasureTool — axis lock collinear with a frozen plane no longer zeroes the readout (Fix 1)', () => {
  it('locking the SAME axis as the frozen plane\'s normal makes snapConstraint() return null', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle-locks the red/X plane (normal [1,0,0])
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3, kind: 'endpoint' }), RAY) // measure stage, _gesturePlane frozen

    // Before locking an axis: plane-constrained, as usual.
    expect(tool.snapConstraint(RAY)).toEqual({
      anchor: [2, 1, 3],
      constraintPlane: { point: [2, 1, 3], normal: [1, 0, 0] },
      offPlanePoints: true,
    })

    tool.onKey(makeKeyEvent('ArrowRight')) // mid-gesture: lock the SAME axis (red/X)
    expect(lockAxisOf(tool)).toBe(0)

    // The lock now wins outright — snapConstraint() reaches the kernel with
    // nothing at all (no anchor, no constraintPlane, no offPlanePoints).
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('a dragged point produces a NON-ZERO distance while the collinear lock is held (the actual regression)', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle-locks the red/X plane (normal [1,0,0])
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3, kind: 'endpoint' }), RAY)
    tool.onKey(makeKeyEvent('ArrowRight')) // mid-gesture axis lock, same axis as the frozen plane

    // A free/unconstrained snap (since snapConstraint() sends nothing to the
    // kernel while locked) landing purely along red/X from p0 — same y/z,
    // only x moves. `_measurePoint` computes the same station here (the
    // free point already sits exactly on the locked axis), so p1 is
    // unaffected — but unlike the pre-fix kernel-side lock, it is never at
    // risk of being projected back onto the frozen plane and collapsed to
    // p0, no matter which axis is locked or how far the cursor moves.
    tool.onPointerMove(makeSnap({ x: 9, y: 1, z: 3, kind: 'endpoint' }), RAY)

    const stage = measureStage(tool)
    expect(stage.p1).toEqual([9, 1, 3]) // raw point, NOT projected back onto the plane
    expect(tool.snapProjected()).toBe(false)
    const dx = stage.p1[0] - stage.p0[0]
    const dy = stage.p1[1] - stage.p0[1]
    const dz = stage.p1[2] - stage.p0[2]
    const dist = Math.hypot(dx, dy, dz)
    expect(dist).toBeCloseTo(7, 9) // NOT zero
  })

  it('releasing the lock restores plane-constrained (projected) behavior on the next move', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle-locks the red/X plane
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3, kind: 'endpoint' }), RAY)
    tool.onKey(makeKeyEvent('ArrowRight')) // lock red/X
    tool.onPointerMove(makeSnap({ x: 9, y: 1, z: 3, kind: 'endpoint' }), RAY)
    expect(measureStage(tool).p1).toEqual([9, 1, 3])

    tool.onKey(makeKeyEvent('ArrowRight')) // same arrow again — releases the lock
    expect(lockAxisOf(tool)).toBeNull()

    expect(tool.snapConstraint(RAY)).toEqual({
      anchor: [2, 1, 3],
      constraintPlane: { point: [2, 1, 3], normal: [1, 0, 0] },
      offPlanePoints: true,
    })

    // An off-plane point now gets projected back onto the plane again, as
    // before any lock was ever set — the plane's normal is [1,0,0], so
    // projection resets X back to the plane's x=2, leaving y/z untouched.
    tool.onPointerMove(makeSnap({ x: 9, y: 5, z: 3, kind: 'endpoint' }), RAY)
    expect(measureStage(tool).p1).toEqual([2, 5, 3])
    expect(tool.snapProjected()).toBe(true)
  })
})

// Shift-held axis lock (tape-measure-rework WP-7 item 2) — mirrors
// MoveTool.setShiftHeld's toggle/release convention, with an added
// latch-pending retry for a genuinely degenerate direction at the moment
// Shift is pressed.
describe('TapeMeasureTool — Shift-held axis lock, parallel stage (WP-7 item 2)', () => {
  it('a Shift latch picks the nearest VIABLE axis and never the along-edge one', () => {
    const { scene } = makeWasmScene() // default edge dir [1,0,0] (red/X)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    // Pull mostly toward +Y, a little toward +Z — nearer to green than blue,
    // and the edge itself (red/X) is never a candidate at all.
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 1, kind: 'plane' }), RAY)

    tool.setShiftHeld(true)
    expect(offsetLockOf(tool)).toBe(1) // green/Y, not red/X (along-edge) or blue/Z
    expect(shiftAxisLockOf(tool)).toBe(true)
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })

  it('a Shift latch works at zero offset when offsetPlane came from a frozen-plane (not axis-lock) case, and release restores that plane exactly', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH }) // normal [0,-1,0]
    const { tool } = makeTool(scene)
    // Default edge_endpoints: dir [1,0,0], already ⊥ the tilted plane's
    // normal — the hover-adopted case, offsetPlane resolved via case 2
    // (`_gesturePlane`), NOT an axis lock. `_updateParallelOrigin` already
    // ran once at the first click, so `offsetPlane` is non-null even though
    // the cursor hasn't moved off the edge yet (zero offset).
    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    const before = parallelStage(tool).offsetPlane
    expect(before).not.toBeNull()
    expect(offsetLockOf(tool)).toBeNull() // not from an axis lock

    tool.setShiftHeld(true)
    expect(offsetLockOf(tool)).not.toBeNull() // latched despite zero offset
    expect(shiftAxisLockOf(tool)).toBe(true)
    // The latch now PRE-EMPTS the frozen-plane case (case 1 beats case 2 in
    // `_resolveOffsetPlane`) — `offsetLock` being non-null is exactly that.

    tool.setShiftHeld(false)
    expect(offsetLockOf(tool)).toBeNull()
    expect(shiftAxisLockOf(tool)).toBe(false)
    // The plane is restored EXACTLY — same origin/normal/u as before latching.
    expect(parallelStage(tool).offsetPlane).toEqual(before)
  })

  it('an arrow-set lock survives a Shift press+release untouched (Shift is a no-op once a lock already exists)', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y, arrow provenance
    expect(offsetLockOf(tool)).toBe(1)
    expect(shiftAxisLockOf(tool)).toBe(false)

    tool.setShiftHeld(true) // no-op: `cur !== null`
    expect(offsetLockOf(tool)).toBe(1)
    expect(shiftAxisLockOf(tool)).toBe(false) // still arrow provenance, not Shift's
    expect(shiftLatchPendingOf(tool)).toBe(false)

    tool.setShiftHeld(false) // release: `_shiftAxisLock` is false, so no-op
    expect(offsetLockOf(tool)).toBe(1) // the arrow lock survives
  })

  it('Shift-then-arrow converts provenance — an arrow always clears the Shift flags, even toggling off the SAME axis Shift had just latched', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'plane' }), RAY) // pure +Y pull
    tool.setShiftHeld(true)
    expect(offsetLockOf(tool)).toBe(1) // green/Y, latched by Shift
    expect(shiftAxisLockOf(tool)).toBe(true)

    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y again — the SAME axis index
    expect(shiftAxisLockOf(tool)).toBe(false) // provenance cleared regardless
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })

  it('Shift-then-a-DIFFERENT-arrow also converts provenance, landing on the arrow\'s own axis', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'plane' }), RAY) // latches green/Y
    tool.setShiftHeld(true)
    expect(offsetLockOf(tool)).toBe(1)

    tool.onKey(makeKeyEvent('ArrowUp')) // blue/Z — a different axis
    expect(offsetLockOf(tool)).toBe(2)
    expect(shiftAxisLockOf(tool)).toBe(false)
  })

  it('ArrowDown while Shift is held clears the lock without re-latching on the next pointer move', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'plane' }), RAY)
    tool.setShiftHeld(true)
    expect(offsetLockOf(tool)).toBe(1)

    tool.onKey(makeKeyEvent('ArrowDown'))
    expect(offsetLockOf(tool)).toBeNull()
    expect(shiftAxisLockOf(tool)).toBe(false)
    expect(shiftLatchPendingOf(tool)).toBe(false) // NOT pending

    tool.onPointerMove(makeSnap({ x: 0, y: 3.5, z: 0, kind: 'plane' }), RAY)
    expect(offsetLockOf(tool)).toBeNull() // did not re-latch
  })

  it('a degenerate first press (cursor still on the edge) sets pending, then latches on the first off-edge pointer move', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    expect(parallelStage(tool).offsetPlane).toBeNull() // no plane/face source either

    tool.setShiftHeld(true)
    expect(offsetLockOf(tool)).toBeNull()
    expect(shiftLatchPendingOf(tool)).toBe(true)

    tool.onPointerMove(makeSnap({ x: 0, y: 2, z: 0, kind: 'plane' }), RAY) // moves off the edge
    expect(offsetLockOf(tool)).toBe(1)
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })

  it('a rejected non-viable arrow leaves a live Shift lock completely untouched, and the rejection hint keeps precedence', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    ;(scene as unknown as { edge_endpoints: ReturnType<typeof vi.fn> }).edge_endpoints
      .mockReturnValue(new Float64Array([0, 0, 0, 0, 2, 0])) // edge along green/Y

    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'plane' }), RAY) // pulls toward +X
    tool.setShiftHeld(true)
    const lockedAxis = offsetLockOf(tool)
    expect(lockedAxis).not.toBeNull()
    expect(shiftAxisLockOf(tool)).toBe(true)

    tool.onKey(makeKeyEvent('ArrowLeft')) // green — runs along this edge → rejected

    expect(offsetLockOf(tool)).toBe(lockedAxis) // untouched
    expect(shiftAxisLockOf(tool)).toBe(true) // untouched
    expect(shiftLatchPendingOf(tool)).toBe(false)
    // `_offsetAxisRejectedHint` is checked FIRST in `statusHint()` — the
    // rejection message wins over the Shift-provenance wording, unchanged
    // from before this WP.
    expect(tool.statusHint()).toContain('Green runs along this edge')
    expect(tool.statusHint()).not.toContain('while Shift is held')
  })
})

describe('TapeMeasureTool — Shift-held axis lock, measure stage (WP-7 item 2)', () => {
  it('a Shift latch picks the dominant axis of p1-p0, making snapConstraint() return null', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 5, z: 2, kind: 'endpoint' }), RAY) // dominant axis: green/Y

    tool.setShiftHeld(true)
    expect(lockAxisOf(tool)).toBe(1)
    expect(shiftAxisLockOf(tool)).toBe(true)
    // A Shift-latched lock is indistinguishable from an arrow-set one here —
    // snapConstraint() returns null either way while `lockAxis` is held.
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('a degenerate p1 === p0 press sets pending, not an immediate latch', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY) // p1 === p0

    tool.setShiftHeld(true)
    expect(lockAxisOf(tool)).toBeNull()
    expect(shiftLatchPendingOf(tool)).toBe(true)

    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'endpoint' }), RAY)
    expect(lockAxisOf(tool)).toBe(1)
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })

  it('release clears a Shift-latched measure-stage lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'endpoint' }), RAY)
    tool.setShiftHeld(true)
    expect(lockAxisOf(tool)).toBe(1)

    tool.setShiftHeld(false)
    expect(lockAxisOf(tool)).toBeNull()
    expect(shiftAxisLockOf(tool)).toBe(false)
  })

  it('a latch does not mutate p1/onGeometry — it takes effect on the NEXT move via snapConstraint, like the arrow path', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 5, z: 0.3, kind: 'endpoint' }), RAY)
    const before = measureStage(tool)
    const beforeP0 = before.p0
    const beforeP1 = before.p1
    const beforeOnGeometry = before.onGeometry

    tool.setShiftHeld(true)
    expect(lockAxisOf(tool)).toBe(1)

    const after = measureStage(tool)
    expect(after.p0).toEqual(beforeP0)
    expect(after.p1).toEqual(beforeP1)
    expect(after.onGeometry).toBe(beforeOnGeometry)
  })

  it('the status hint names Shift provenance while held', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 5, z: 0, kind: 'endpoint' }), RAY)

    tool.setShiftHeld(true)
    expect(tool.statusHint()).toContain('Measurement locked to green')
    expect(tool.statusHint()).toContain('while Shift is held')
  })
})

describe('TapeMeasureTool — Shift lock, both stages (WP-7 item 2)', () => {
  it('_resetToIdle() clears _shiftAxisLock/_shiftLatchPending — parallel stage, via cancel()', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'plane' }), RAY)
    tool.setShiftHeld(true)
    expect(shiftAxisLockOf(tool)).toBe(true)

    tool.cancel()
    expect(shiftAxisLockOf(tool)).toBe(false)
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })

  it('_resetToIdle() clears _shiftAxisLock/_shiftLatchPending — measure stage, via a real commit', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'endpoint' }), RAY)
    tool.setShiftHeld(true)
    expect(shiftAxisLockOf(tool)).toBe(true)

    tool.onPointerDown(makeSnap({ x: 5, y: 5, z: 5, kind: 'ground' }), RAY) // commit, back to idle
    expect(shiftAxisLockOf(tool)).toBe(false)
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })

  it('setShiftHeld is a COMPLETE no-op while idle — no field changes at all, not even pending', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.setShiftHeld(true)
    expect(shiftAxisLockOf(tool)).toBe(false)
    expect(shiftLatchPendingOf(tool)).toBe(false)
    expect(offsetLockOf(tool)).toBeNull()
    expect(lockAxisOf(tool)).toBeNull()

    tool.setShiftHeld(false)
    expect(shiftAxisLockOf(tool)).toBe(false)
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })

  it('setShiftHeld is a COMPLETE no-op while pendingRescale — no field changes at all, not even pending', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3') // arms pendingRescale — both ends rest on real geometry
    expect((tool as unknown as { stage: { kind: string } }).stage.kind).toBe('pendingRescale')

    tool.setShiftHeld(true)
    expect(shiftAxisLockOf(tool)).toBe(false)
    expect(shiftLatchPendingOf(tool)).toBe(false)

    tool.setShiftHeld(false)
    expect(shiftAxisLockOf(tool)).toBe(false)
    expect(shiftLatchPendingOf(tool)).toBe(false)
  })
})

// Locked-axis reading decoupled from the kernel's own snap magnet (the
// tape-measure-rework fix this file's own regression exists to cover): the
// kernel's `SnapLock::Axis` unconditionally force-projects its winning
// candidate's POSITION onto the locked axis line before returning it —
// correct only for real geometry that happens to run ALONG the locked axis,
// silently erasing all cursor motion for anything that runs ACROSS it.
// `snapConstraint()` now sends nothing lock-related to the kernel at all
// while `lockAxis` is held (see its own doc) — the kernel runs a fully
// free/unconstrained snap, and `_measurePoint` projects THAT onto the
// locked axis itself, in TS.
describe('TapeMeasureTool — locked-axis reading decoupled from the kernel snap (tape-measure-rework fix)', () => {
  it('a real snap OFF the locked axis is projected from its own free position, and snapConstraint() is null throughout', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'endpoint' }), RAY) // p0 = [1, 2, 3]
    tool.onKey(makeKeyEvent('ArrowRight')) // lock red/X
    expect(lockAxisOf(tool)).toBe(0)
    expect(tool.snapConstraint(RAY)).toBeNull()

    // A real vertex/edge snap with nonzero y/z relative to p0 — exactly the
    // "runs across the locked axis" case that used to get its position
    // erased by the kernel's own force-projection. Free position [4, 7, -2]:
    // component along red/X from p0 is dot([3, 5, -5], [1, 0, 0]) = 3, so
    // the correct projected point is p0 + [1,0,0]*3 = [4, 2, 3] — y/z stay
    // at p0's own y/z, NOT the free snap's y/z (7, -2).
    tool.onPointerMove(makeSnap({ x: 4, y: 7, z: -2, kind: 'endpoint' }), RAY)

    expect(measureStage(tool).p1).toEqual([4, 2, 3])
    expect(tool.snapProjected()).toBe(true) // the free position (4,7,-2) != the projected one
    expect(tool.snapConstraint(RAY)).toBeNull() // still nothing reaches the kernel while locked
  })

  it('a ground/plane fallback snap under a lock takes its station from the RAY, not from naively projecting the fallback point\'s own coordinates', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY) // p0 = [0, 0, 0]
    tool.onKey(makeKeyEvent('ArrowUp')) // lock blue/Z
    expect(lockAxisOf(tool)).toBe(2)

    // A ground/plane fallback snap's OWN coordinates are deliberately chosen
    // to project (naively) onto a totally different station (20) than the
    // ray-based one (5) computed below — this is what actually discriminates
    // between the two implementations: a naive `dot(snap.xyz - p0, axis)`
    // would read 20; the correct ray-based station reads 5.
    const groundSnap = makeSnap({ x: 0, y: 0, z: 20, kind: 'ground' })

    // A ray from [5, 0, 0] aimed at 45 degrees toward the Z axis. By hand:
    // b = dot(axis, rayDir) = 1/sqrt(2); denom = 1 - b^2 = 0.5;
    // w = p0 - rayOrigin = [-5, 0, 0]; d = dot(axis, w) = 0;
    // e = dot(rayDir, w) = (1/sqrt(2)) * 5 = 5/sqrt(2);
    // t = (b*e - d) / denom = (5/2) / 0.5 = 5.
    const s = 1 / Math.SQRT2
    const ray: Ray = { origin: [5, 0, 0], direction: [-s, 0, s] }

    tool.onPointerMove(groundSnap, ray)

    const p1 = measureStage(tool).p1
    expect(p1[0]).toBeCloseTo(0, 9)
    expect(p1[1]).toBeCloseTo(0, 9)
    expect(p1[2]).toBeCloseTo(5, 9) // the ray-based station — NOT 20 (the naive one)
  })

  it('onPointerDown\'s commit and onPointerMove\'s preview agree exactly on p1 for the same snap+ray', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY) // p0 = [0, 0, 0]
    tool.onKey(makeKeyEvent('ArrowUp')) // lock blue/Z

    // Same ray-based-station setup as the previous test (station = 5).
    const s = 1 / Math.SQRT2
    const ray: Ray = { origin: [5, 0, 0], direction: [-s, 0, s] }
    const snap = makeSnap({ x: 999, y: 999, z: 999, kind: 'ground' }) // coords unused by the ray-based branch

    tool.onPointerMove(snap, ray)
    const previewP1 = [...measureStage(tool).p1] as [number, number, number]

    tool.onPointerDown(snap, ray) // second click — commits (kind 'ground' is off-geometry, so a guide point drops)

    expect(guidePoints).toHaveLength(1)
    expect(guidePoints[0][0]).toBeCloseTo(previewP1[0], 9)
    expect(guidePoints[0][1]).toBeCloseTo(previewP1[1], 9)
    expect(guidePoints[0][2]).toBeCloseTo(previewP1[2], 9)
  })

  it('a degenerate typed commit under a held Y/Z axis lock defaults to +axis, not hardcoded [1,0,0]', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1, kind: 'endpoint' }), RAY) // p0 = [1, 1, 1], p1 === p0
    tool.onKey(makeKeyEvent('ArrowLeft')) // lock green/Y — cursor hasn't moved, still degenerate

    typeAndEnter(tool, '5')

    // add_guide_line(origin, direction) — direction must be +Y ([0,1,0]),
    // not the hardcoded [1,0,0] default.
    expect(scene.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = (scene.add_guide_line as unknown as { mock: { calls: number[][] } }).mock.calls[0]
    expect([ox, oy, oz]).toEqual([1, 1, 1])
    expect([dx, dy, dz]).toEqual([0, 1, 0])
  })

  it('regression: a degenerate typed commit with NO lock held still defaults to [1,0,0], unchanged', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1, kind: 'endpoint' }), RAY) // p0 = [1, 1, 1], p1 === p0, no lock

    typeAndEnter(tool, '5')

    expect(scene.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = (scene.add_guide_line as unknown as { mock: { calls: number[][] } }).mock.calls[0]
    expect([ox, oy, oz]).toEqual([1, 1, 1])
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })
})
