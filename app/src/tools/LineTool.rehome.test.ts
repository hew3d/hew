/**
 * Line re-homing (tool-parity playtest2 §2b) — Kurt's finding 1's harder
 * half: "I need the ability to axis-lock at any point right up until
 * clicking the second point... I recognize that this complicates drawing
 * Lines since a Line could arbitrarily go into any 3D space this way."
 *
 * `LineTool` already has a mid-chain direction lock (`lockAxis`, armed by
 * arrow keys while `capturingInput()`), but the chain's drawing PLANE is
 * frozen at the first click and every later point is computed against that
 * SAME frozen plane. A lock aimed off that plane therefore used to resolve
 * a point the kernel's `Sketch::add_segment` genuinely refuses —
 * `SketchError::PointOffPlane` (`crates/kernel/src/sketch.rs`, exercised by
 * `crates/kernel/tests/sketch_specs.rs`, e.g. line 156 — pre-existing,
 * established kernel behavior this suite does not re-derive or touch).
 *
 * The fix re-homes instead of refusing: finalize the chain's sketch as it
 * stands and begin a NEW one, through the anchor, on a plane that actually
 * contains the locked direction — continuing the same visible polyline
 * across two kernel sketches. `_rehomeChain`'s plane CHOICE
 * (`rehomePlaneNormal`, lineInput.ts) is covered by its own pure unit tests;
 * this file drives the integration: does `LineTool` detect the off-plane
 * point and re-home BEFORE committing, instead of ever handing the kernel a
 * point it would refuse.
 *
 * The mock kernel's `sketch_add_segment` enforces the SAME plane-membership
 * tolerance (`kernel::tol::PLANE_DIST` = 1e-9) the real kernel does, so the
 * red-check below reproduces the actual pre-fix symptom — a thrown
 * `PointOffPlane`, surfaced as a toast — not just an absence of re-homing.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(p: readonly [number, number, number]): Snap {
  return { x: p[0], y: p[1], z: p[2], kind: 'plane' }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

const PLANE_DIST = 1e-9

type SegmentCall = { sketch: bigint; a: [number, number, number]; b: [number, number, number] }
type PlaneCall = [number, number, number, number, number, number]

/**
 * A mock kernel scene whose `sketch_add_segment` enforces plane membership
 * exactly like the real `Sketch::add_segment_inner`
 * (`crates/kernel/src/sketch.rs:2078-2084`) — throwing an Error whose
 * message starts with `PointOffPlane` (the toast path's convention,
 * `kernelErrors.ts`) for either endpoint farther than `PLANE_DIST` from the
 * target sketch's plane. `enforcePlanarity: false` reproduces the OLD
 * (pre-fix) unconditional-accept mock behavior other LineTool test files
 * use, for the one test that needs to observe what would happen WITHOUT any
 * kernel-side check at all (none of the fixtures here need that, but the
 * flag documents the difference for a future reader).
 */
function makeWasmScene() {
  const planes = new Map<bigint, [number, number, number, number, number, number]>()
  const segmentCalls: SegmentCall[] = []
  const planeCalls: PlaneCall[] = []
  let sketchCounter = 90n

  const distanceToPlane = (p: readonly [number, number, number], plane: readonly number[]): number => {
    const [ox, oy, oz, nx, ny, nz] = plane
    return nx * (p[0] - ox) + ny * (p[1] - oy) + nz * (p[2] - oz)
  }

  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      planes.set(sketchCounter, [0, 0, 0, 0, 0, 1])
      return sketchCounter
    }),
    begin_sketch_on_plane: vi.fn(
      (px: number, py: number, pz: number, nx: number, ny: number, nz: number) => {
        sketchCounter += 1n
        planeCalls.push([px, py, pz, nx, ny, nz])
        planes.set(sketchCounter, [px, py, pz, nx, ny, nz])
        return sketchCounter
      },
    ),
    pick_face: vi.fn(() => undefined), // never an eligible face in these fixtures
    pick_sketch: vi.fn(() => undefined), // never a hoverable sketch either
    sketch_plane: vi.fn((h: bigint) => {
      const p = planes.get(h)
      return p === undefined ? undefined : new Float64Array(p)
    }),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_add_segment: vi.fn(
      (sketch: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
        const plane = planes.get(sketch)
        if (plane === undefined) throw new Error('UnknownSketch: no such sketch')
        const da = distanceToPlane([ax, ay, az], plane)
        const db = distanceToPlane([bx, by, bz], plane)
        if (Math.abs(da) > PLANE_DIST) throw new Error('PointOffPlane: point 0 is off the sketch plane')
        if (Math.abs(db) > PLANE_DIST) throw new Error('PointOffPlane: point 1 is off the sketch plane')
        segmentCalls.push({ sketch, a: [ax, ay, az], b: [bx, by, bz] })
        return {
          new_edges: () => new BigUint64Array([]),
          regions_created: () => new BigUint64Array([]),
          regions_removed: () => new BigUint64Array([]),
          free: vi.fn(),
        }
      },
    ),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return {
    scene: scene as unknown as WasmScene,
    segmentCalls,
    planeCalls,
    beginGestureCalls: scene.sketch_begin_gesture as unknown as ReturnType<typeof vi.fn>,
    endGestureCalls: scene.sketch_end_gesture as unknown as ReturnType<typeof vi.fn>,
  }
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, onCommit, onToast, onFaceImprint, onMeasurement }
}

/**
 * Drive a chain anchored to a NON-ground vertical plane (normal = world +X,
 * matching Kurt's "a vertical face" repro): idle-lock red/X before the
 * first click, place two points that stay ON that plane (x = 2 throughout,
 * moving along Z for the first segment), arm a MID-CHAIN lock to the SAME
 * axis (red/X) — a direction that leaves the x=2 plane precisely because
 * it runs ALONG the plane's own normal (the "wall facing north, lock to
 * north" case) — then click a third point at exactly what the inference
 * engine's lock-fallback branch would resolve: on the line anchor2 + t·X,
 * off the x=2 plane.
 */
function driveOffPlaneChain(tool: LineTool) {
  tool.onKey(makeKeyEvent('ArrowRight')) // idle plane lock: red/X, before any click
  tool.onPointerDown(makeSnap([2, 3, 4]), RAY) // first click — anchors on the x=2 plane
  tool.onPointerDown(makeSnap([2, 3, 9]), RAY) // second click — stays on x=2 (moves along Z)
  tool.onKey(makeKeyEvent('ArrowRight')) // mid-chain direction lock: red/X (the plane's OWN normal)
  tool.onPointerDown(makeSnap([7, 3, 9]), RAY) // third click — the LOCKED point, off x=2 (x moved)
}

describe('LineTool — chain re-homing when a lock leaves the frozen plane (design §2b)', () => {
  it('re-homes onto a new sketch instead of throwing, and keeps the polyline visually unbroken', () => {
    const { scene, segmentCalls, planeCalls } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)

    expect(() => driveOffPlaneChain(tool)).not.toThrow()

    expect(onToast).not.toHaveBeenCalled()
    // Two distinct planes were minted: the original x=2 wall, then the
    // re-homed plane through the second anchor.
    expect(planeCalls).toHaveLength(2)
    expect(planeCalls[0]).toEqual([2, 3, 4, 1, 0, 0]) // the original x=2 plane (idle-locked)

    // Two segments committed, into TWO DIFFERENT sketches — the first
    // segment's sketch is untouched by the second (re-homed) commit.
    expect(segmentCalls).toHaveLength(2)
    expect(segmentCalls[0]).toEqual({ sketch: segmentCalls[0].sketch, a: [2, 3, 4], b: [2, 3, 9] })
    expect(segmentCalls[1].sketch).not.toBe(segmentCalls[0].sketch)
    expect(segmentCalls[1]).toEqual({ sketch: segmentCalls[1].sketch, a: [2, 3, 9], b: [7, 3, 9] })

    // The re-homed plane genuinely contains the segment just drawn (its own
    // commit succeeded against the mock's plane-membership check) AND the
    // previous segment's direction — spanned by Z (segment 1) and X
    // (segment 2, the lock direction), giving a plane with normal ±Y.
    expect(planeCalls[1]).toEqual([2, 3, 9, 0, 1, 0])
  })

  it('undo granularity: each commit — including the re-homing one — is exactly one begin/end gesture bracket, same as any other segment', () => {
    const { scene, beginGestureCalls, endGestureCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    driveOffPlaneChain(tool)

    // Two commits (segment 1 on the original plane, segment 2 re-homed) —
    // two gesture brackets, not three or four. The re-homing commit opens
    // exactly the SAME single bracket `runSketchGesture` opens for every
    // other commit (it happens to also mint a sketch inside that bracket,
    // on a plane-cache miss, exactly like any other first commit onto a
    // fresh plane) — so undoing the re-homed segment costs exactly one
    // Cmd+Z, identical to undoing any other single LineTool segment.
    expect(beginGestureCalls).toHaveBeenCalledTimes(2)
    expect(endGestureCalls).toHaveBeenCalledTimes(2)
  })

  it('the first (pre-lock) segment stays a plain on-plane commit — re-homing only engages when the lock actually leaves the plane', () => {
    const { scene, planeCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle lock: red/X
    tool.onPointerDown(makeSnap([2, 3, 4]), RAY)
    tool.onPointerDown(makeSnap([2, 3, 9]), RAY) // stays on x=2 — ordinary commit, no re-home

    expect(planeCalls).toHaveLength(1) // only the original plane was ever minted
  })

  it('an ordinary unlocked ON-plane click never re-homes — re-homing fires only when the committed point genuinely leaves the frozen plane', () => {
    const { scene, planeCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle lock: red/X
    tool.onPointerDown(makeSnap([2, 3, 4]), RAY)
    // No mid-chain lockAxis armed — an ordinary (unlocked) second click.
    tool.onPointerDown(makeSnap([2, 8, 4]), RAY)

    expect(planeCalls).toHaveLength(1)
  })
})

describe('LineTool — honest refusal when a re-homed chain closes onto an EARLIER sketch (playtest-2 review finding B)', () => {
  it('visually returning to the chain\'s start point, after a re-home moved onto a new sketch, surfaces a toast instead of silently failing to close', () => {
    const { scene, segmentCalls } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)

    // The exact "floor edge, arrow-lock vertical, across, back down to the
    // start" workflow the module doc calls out: driveOffPlaneChain lays
    // down segment 1 on the x=2 wall, then re-homes onto a NEW sketch for
    // segment 2 (the y=3 plane, spanned by Z then the X lock) — both
    // segments share y=3 throughout.
    driveOffPlaneChain(tool)
    // Fourth click: back down to the chain's very first anchor, [2,3,4] —
    // still on the CURRENT (re-homed) y=3 plane, so no further re-home is
    // needed, but [2,3,4] itself belongs to the FIRST sketch, not this one.
    tool.onPointerDown(makeSnap([2, 3, 4]), RAY)

    // The mock's regions_created() is always empty (no cross-sketch region
    // detection in the real kernel either — the two vertices are merely
    // coincident, not welded) — so this must NOT be silent.
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast).toHaveBeenCalledWith(expect.stringContaining('spans two planes'))
    // The point still commits (as its own, merely-coincident vertex) rather
    // than being dropped outright — the refusal is about the CLOSURE, not
    // the placement.
    expect(segmentCalls).toHaveLength(3)
    expect(segmentCalls[2]).toEqual({ sketch: segmentCalls[2].sketch, a: [7, 3, 9], b: [2, 3, 4] })
  })

  it('an ordinary NOT-YET-closed chain (no coincident earlier-sketch point) stays silent, as before', () => {
    const { scene } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)

    driveOffPlaneChain(tool) // two segments, still open, nothing coincides with an earlier sketch
    tool.onPointerDown(makeSnap([9, 3, 9]), RAY) // an ordinary next point, nowhere near the start

    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('LineTool — a mid-gesture undo invalidates the stale cross-sketch bookkeeping (tool-parity DELTA review finding 2)', () => {
  it('a coincidence with a PRE-UNDO vertex does not surface the "spans two planes" toast once onHistoryChanged has fired', () => {
    const { scene } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)

    // Same setup as the "closes onto an earlier sketch" repro just above:
    // two segments across two re-homed sketches (A, then B), leaving
    // `_chainVertices` holding [ {[2,3,4],A}, {[2,3,9],A}, {[7,3,9],B} ].
    driveOffPlaneChain(tool)

    // The Viewport's shared undo/redo choke point (`applyHistoryChange` in
    // Viewport.tsx) calls this on the active tool for EVERY undo/redo entry
    // point, whether or not the undone step actually touched THIS chain's
    // geometry — LineTool cannot tell from here, so it drops its own cached
    // description of already-committed geometry rather than risk it going
    // stale (see `onHistoryChanged`'s doc in LineTool.ts / types.ts).
    tool.onHistoryChanged()

    // Continue the SAME chain (still anchored at [7,3,9] on sketch B's
    // plane — onHistoryChanged does not touch planeStage) and close back to
    // the chain's very first point, [2,3,4]. Pre-fix, `_chainVertices` still
    // remembered that point as belonging to sketch A (a DIFFERENT sketch
    // from the current B) and wrongly fired the "spans two planes" toast —
    // exactly the assertion the sibling test two blocks up makes WITHOUT an
    // intervening onHistoryChanged call, where that same toast is in fact
    // correct. Here, the invalidation must have cleared that bookkeeping,
    // so the coincidence is silently treated as just another ordinary
    // (merely coincident) vertex — no wrongful refusal.
    tool.onPointerDown(makeSnap([2, 3, 4]), RAY)

    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('LineTool — chain re-homing against a MOVED drawing-axes frame', () => {
  /** origin [5,0,0], red = world +Y, green = world −X, blue = world +Z —
   *  mirrors drawPlane.test.ts's `resolveClickDrawTarget` moved-frame
   *  fixture. `_rehomeChain` itself never reads the frame (it operates on
   *  already-resolved world-space directions — the frame only matters for
   *  how the FIRST click's idle lock resolves its plane, §2a), so this
   *  proves the composition of a moved-frame first click with re-homing,
   *  not a frame-dependent code path inside re-homing itself. */
  function makeMovedFrameScene() {
    const fixture = makeWasmScene()
    ;(fixture.scene.axes as unknown as ReturnType<typeof vi.fn>).mockReturnValue(
      new Float64Array([5, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1]),
    )
    return fixture
  }

  it('re-homes correctly when the initial plane came from a moved-frame idle lock', () => {
    const { scene, segmentCalls, planeCalls } = makeMovedFrameScene()
    const { tool, onToast } = makeTool(scene)

    // Red (axis 0) of the moved frame is world +Y — locking to it, both at
    // the idle click and again mid-chain, is this fixture's version of
    // "wall facing north, lock to north": the plane's own normal.
    tool.onKey(makeKeyEvent('ArrowRight')) // idle lock: red = world +Y
    tool.onPointerDown(makeSnap([2, 3, 4]), RAY) // plane: normal world Y, through [2,3,4] (y=3)
    tool.onPointerDown(makeSnap([2, 3, 9]), RAY) // stays on-plane (y=3 unchanged, moves along Z)
    tool.onKey(makeKeyEvent('ArrowRight')) // mid-chain lock: red = world +Y again
    tool.onPointerDown(makeSnap([2, 7, 9]), RAY) // off the y=3 plane (y moved)

    expect(onToast).not.toHaveBeenCalled()
    expect(planeCalls).toHaveLength(2)
    expect(planeCalls[0]).toEqual([2, 3, 4, 0, 1, 0])
    expect(planeCalls[1]).toEqual([2, 3, 9, -1, 0, 0]) // spanned by Z (seg 1) and Y (seg 2, the lock)
    expect(segmentCalls).toHaveLength(2)
    expect(segmentCalls[1].sketch).not.toBe(segmentCalls[0].sketch)
    expect(segmentCalls[1]).toEqual({ sketch: segmentCalls[1].sketch, a: [2, 3, 9], b: [2, 7, 9] })
  })
})
