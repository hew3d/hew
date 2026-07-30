/**
 * Mid-gesture plane lock (tool-parity playtest2 §2a) — Kurt's finding 1's
 * "smaller half": Rectangle/Circle/Polygon (a single anchored stage) and
 * Arc's `chord` stage (endpoint A placed, B not yet) commit NOTHING to the
 * kernel until the gesture's LAST click, so — unlike LineTool's chain
 * (§2b) — an arrow key pressed AFTER the anchor can simply re-lock the
 * plane THROUGH that anchor and let the rest of the gesture resolve against
 * it; the same arrow again reverts to whatever plane the anchor would have
 * resolved onto unlocked (`nextGestureLockPlane`, drawPlane.ts).
 *
 * One shared parameterized suite drives Rectangle/Circle/Polygon through a
 * common `DrawToolUnderTest` surface (mirrors idlePlaneLock.test.ts); Arc's
 * three-click chord/bulge shape gets its own describe block below.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RectangleTool } from './RectangleTool'
import { CircleTool } from './CircleTool'
import { PolygonTool } from './PolygonTool'
import { ArcTool } from './ArcTool'
import { makeSketchPlaneCache, type SketchPlaneCache } from './sketchGesture'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** A frame moved off world identity (mirrors drawPlane.test.ts's
 *  `resolveClickDrawTarget` moved-frame fixture): origin [5,0,0], red axis
 *  world +Y, green axis world -X, blue axis world +Z. Every test in this
 *  file runs against BOTH this frame and the world-identity default — the
 *  branch owns movable axes, so nothing here may pass only at world
 *  identity. */
const MOVED_FRAME_FLAT = [5, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1]
const WORLD_FRAME_FLAT = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]

/** A genuinely TILTED frame — unlike `MOVED_FRAME_FLAT` above (which only
 *  rotates about world Z, so its derived blue axis stays [0,0,1], vertical)
 *  — with a non-vertical blue axis: origin [0,0,0], red = world +X, green =
 *  world +Z, blue derived (x×y) = world −Y. `Scene::set_axes` only
 *  constrains the frame to be orthonormal/right-handed (blue = x×y); it has
 *  no verticality requirement, and AxesTool lets a user park red/green
 *  anywhere non-degenerate, so this is ordinary reachable state, not a
 *  contrived corner case (tool-parity DELTA review finding 1). */
const TILTED_FRAME_FLAT = [0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -1, 0]

function makeSnap(p: readonly [number, number, number]): Snap {
  return { x: p[0], y: p[1], z: p[2], kind: 'plane' }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

type SegmentCall = { sketch: bigint; a: [number, number, number]; b: [number, number, number] }
type PlaneCall = [number, number, number, number, number, number]

/** A hoverable non-ground sketch — the y=0 plane, normal -Y — used ONLY by
 *  the "idle lock beats sketch-hover" test below, to make that precedence
 *  check genuine (a fixture where `pick_sketch` always misses can't tell
 *  "beats hover" apart from "there was nothing to hover"). */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, -1, 0])

function makeWasmScene(opts: { frame?: number[]; sketchPick?: bigint } = {}) {
  const planes = new Map<bigint, Float64Array>(
    opts.sketchPick !== undefined ? [[opts.sketchPick, TILTED_PLANE]] : [],
  )
  const segmentCalls: SegmentCall[] = []
  const planeCalls: PlaneCall[] = []
  let sketchCounter = 90n
  let nextRegionsCreated: bigint[] = []

  const scene = {
    axes: vi.fn(() => new Float64Array(opts.frame ?? WORLD_FRAME_FLAT)),
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      planes.set(sketchCounter, new Float64Array([0, 0, 0, 0, 0, 1]))
      return sketchCounter
    }),
    begin_sketch_on_plane: vi.fn(
      (px: number, py: number, pz: number, nx: number, ny: number, nz: number) => {
        sketchCounter += 1n
        planeCalls.push([px, py, pz, nx, ny, nz])
        planes.set(sketchCounter, new Float64Array([px, py, pz, nx, ny, nz]))
        return sketchCounter
      },
    ),
    pick_face: vi.fn(() => undefined), // never an eligible face in these fixtures
    pick_sketch: vi.fn(() => opts.sketchPick),
    sketch_plane: vi.fn((h: bigint) => planes.get(h)),
    face_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_begin_curve: vi.fn(),
    sketch_begin_curve_with: vi.fn(),
    sketch_begin_polygon_with: vi.fn(),
    sketch_end_curve: vi.fn(),
    sketch_add_segment: vi.fn(
      (sketch: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
        segmentCalls.push({ sketch, a: [ax, ay, az], b: [bx, by, bz] })
        const rc = nextRegionsCreated
        nextRegionsCreated = []
        return {
          new_edges: () => new BigUint64Array([]),
          regions_created: () => new BigUint64Array(rc),
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
    setNextRegionsCreated: (rc: bigint[]) => { nextRegionsCreated = rc },
  }
}

/** The subset of each draw tool's public surface these tests drive. */
interface DrawToolUnderTest {
  onPointerDown(snap: Snap | null, ray: Ray): void
  onPointerMove(snap: Snap | null, ray: Ray): void
  onKey(ev: KeyboardEvent): void
  capturingInput(): boolean
  cancel(): void
}

interface Driver {
  name: string
  make(scene: WasmScene, cache?: SketchPlaneCache): DrawToolUnderTest
  /** Click ONE anchor point — leaves the tool anchored (single-point stage,
   *  nothing committed yet). */
  anchor(tool: DrawToolUnderTest, a: readonly [number, number, number]): void
  /** Finish the gesture from its anchored stage with a second click at `b`,
   *  committing into the kernel. */
  finish(tool: DrawToolUnderTest, b: readonly [number, number, number]): void
}

const DRIVERS: Driver[] = [
  {
    name: 'Rectangle',
    make: (scene, cache) =>
      new RectangleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn(), cache) as unknown as DrawToolUnderTest,
    anchor: (tool, a) => tool.onPointerDown(makeSnap(a), RAY),
    finish: (tool, b) => tool.onPointerDown(makeSnap(b), RAY),
  },
  {
    name: 'Circle',
    make: (scene, cache) =>
      new CircleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn(), cache) as unknown as DrawToolUnderTest,
    anchor: (tool, a) => tool.onPointerDown(makeSnap(a), RAY),
    finish: (tool, b) => tool.onPointerDown(makeSnap(b), RAY),
  },
  {
    name: 'Polygon',
    make: (scene, cache) =>
      new PolygonTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn(), cache) as unknown as DrawToolUnderTest,
    anchor: (tool, a) => tool.onPointerDown(makeSnap(a), RAY),
    finish: (tool, b) => tool.onPointerDown(makeSnap(b), RAY),
  },
]

describe.each(DRIVERS)('$name — mid-gesture plane lock (design §2a)', ({ name, make, anchor, finish }) => {
  it('an arrow key AFTER the anchor re-locks the plane through the anchor, at world identity', () => {
    const { scene, planeCalls } = makeWasmScene()
    const tool = make(scene)

    anchor(tool, [2, 3, 0]) // ground-plane anchor, nothing committed
    expect(tool.capturingInput()).toBe(true)

    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock, through the anchor
    finish(tool, [2, 9, 5]) // on the new plane: x fixed at the anchor's 2

    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(planeCalls).toHaveLength(1)
    // Plane passes through the ANCHOR (2,3,0), normal = world +X.
    expect(planeCalls[0]).toEqual([2, 3, 0, 1, 0, 0])
  })

  it('the SAME arrow again releases the lock and reverts to the natural (unlocked) plane', () => {
    const { scene, planeCalls, segmentCalls } = makeWasmScene()
    const tool = make(scene)

    anchor(tool, [2, 3, 0]) // no lock active yet — natural = ground
    tool.onKey(makeKeyEvent('ArrowRight')) // lock on
    tool.onKey(makeKeyEvent('ArrowRight')) // same arrow — lock off, revert to natural
    finish(tool, [7, 9, 0])

    expect(planeCalls).toHaveLength(0) // never mint an axis-plane sketch
    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(segmentCalls.every((s) => s.a[2] === 0 && s.b[2] === 0)).toBe(true) // ground fast path
  })

  it('re-lock passes through the ORIGINAL anchor, not a cursor that drifted after anchoring', () => {
    const { scene, planeCalls } = makeWasmScene()
    const tool = make(scene)

    anchor(tool, [2, 3, 0])
    tool.onPointerMove(makeSnap([40, 40, 0]), RAY) // cursor wanders far from the anchor
    tool.onKey(makeKeyEvent('ArrowRight'))
    finish(tool, [2, 9, 5])

    expect(planeCalls[0].slice(0, 3)).toEqual([2, 3, 0]) // origin is the ANCHOR, not [40,40,0]
  })

  it('a different arrow switches the lock axis (not toggle-off)', () => {
    const { scene, planeCalls } = makeWasmScene()
    const tool = make(scene)

    anchor(tool, [2, 3, 0])
    tool.onKey(makeKeyEvent('ArrowRight')) // red
    tool.onKey(makeKeyEvent('ArrowLeft')) // green — switches, does not release
    finish(tool, [9, 3, 5]) // on the new plane: y fixed at the anchor's 3

    expect(planeCalls).toHaveLength(1)
    expect(planeCalls[0]).toEqual([2, 3, 0, 0, 1, 0])
  })

  it('the mid-gesture lock reads the CURRENT (moved) drawing-axes frame, not literal world axes', () => {
    const { scene, planeCalls } = makeWasmScene({ frame: MOVED_FRAME_FLAT })
    const tool = make(scene)

    anchor(tool, [2, 3, 0])
    tool.onKey(makeKeyEvent('ArrowRight')) // axis 0 (red) of the MOVED frame = world +Y
    finish(tool, [9, 3, 5])

    expect(planeCalls).toHaveLength(1)
    expect(planeCalls[0]).toEqual([2, 3, 0, 0, 1, 0])
  })

  it('releasing the lock under a moved frame still reverts cleanly to the natural (ground) plane', () => {
    const { scene, planeCalls, segmentCalls } = makeWasmScene({ frame: MOVED_FRAME_FLAT })
    const tool = make(scene)

    anchor(tool, [2, 3, 0])
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onKey(makeKeyEvent('ArrowRight')) // release
    finish(tool, [7, 9, 0])

    expect(planeCalls).toHaveLength(0)
    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(segmentCalls.every((s) => s.a[2] === 0 && s.b[2] === 0)).toBe(true)
  })

  it('after re-locking, the gesture stays sticky in plane mode — no NEW face pick beyond the anchor click', () => {
    const { scene } = makeWasmScene()
    const tool = make(scene)

    anchor(tool, [2, 3, 0])
    const pickFaceCallsAtAnchor = (scene.pick_face as ReturnType<typeof vi.fn>).mock.calls.length

    tool.onKey(makeKeyEvent('ArrowRight'))
    finish(tool, [2, 9, 5])

    // Face pick is only ever consulted while idle deciding face-vs-plane
    // mode (once, at the anchor click); an anchored gesture never re-checks
    // it — `_currentMode`'s sticky-mid-gesture branch, unaffected by this
    // change — and the arrow-key handler itself never touches `pick_face`.
    expect((scene.pick_face as ReturnType<typeof vi.fn>).mock.calls.length).toBe(pickFaceCallsAtAnchor)
  })

  it('releasing the lock after an idle-locked, OFF-GROUND anchor reverts to a plane THROUGH THE ANCHOR — not the world ground plane, which would silently drop its elevation (playtest-2 review finding A)', () => {
    const { scene, segmentCalls } = makeWasmScene()
    const tool = make(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle lock ON (red/X) BEFORE any click
    anchor(tool, [2, 3, 5]) // first click resolves THROUGH the active lock: a vertical
                             // plane through (2,3,5) — an anchor genuinely off the
                             // ground (z=5), which is entirely normal while locked
    tool.onKey(makeKeyEvent('ArrowRight')) // release mid-gesture: same arrow toggles off
    finish(tool, [9, 9, 5]) // second click, still on the anchor's own plane (z=5)

    // The committed geometry must stay on the ANCHOR's plane (z=5). Reverting
    // the released lock to the world ground plane (z=0) silently drops the
    // anchor's elevation and commits the wrong shape with no refusal.
    expect(segmentCalls.length).toBeGreaterThan(0)
    expect(segmentCalls.every((s) => s.a[2] === 5 && s.b[2] === 5)).toBe(true)
  })

  it('releasing the lock after an idle-locked, OFF-GROUND anchor under a TILTED (non-vertical-blue) drawing-axes frame reverts to the LITERAL ground orientation through the anchor, not the frame\'s tilted blue axis (tool-parity DELTA review finding 1)', () => {
    const { scene, planeCalls, segmentCalls } = makeWasmScene({ frame: TILTED_FRAME_FLAT })
    const tool = make(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle lock ON (red/X of the tilted frame — still literal world +X here) BEFORE any click
    anchor(tool, [2, 4, 6]) // first click resolves through the active lock: a vertical
                             // plane through (2,4,6), normal = world +X — off the
                             // ground (z=6), entirely normal while locked
    tool.onKey(makeKeyEvent('ArrowRight')) // release mid-gesture: same arrow toggles off
    finish(tool, [9, 9, 6]) // second click, still on the anchor's own released plane

    // Anchor elevation must still be preserved — the SAME contract the
    // world-identity case just above checks; a tilted frame must not
    // regress it.
    expect(segmentCalls.length).toBeGreaterThan(0)
    expect(segmentCalls.every((s) => s.a[2] === 6 && s.b[2] === 6)).toBe(true)

    // The released plane's ORIENTATION must be the LITERAL ground normal
    // [0,0,1] — matching what an unlocked click would actually have
    // resolved to (`resolveIdleDrawTarget` is frame-agnostic: absent a
    // sketch hover it always falls back to the literal `groundDrawPlane()`,
    // never reading the drawing-axes frame at all) — NOT the tilted frame's
    // derived blue axis [0,-1,0]. Pre-fix, `groundNaturalTarget` followed
    // `frame`'s blue axis and committed [2,4,6, 0,-1,0]: a vertical wall
    // facing world Y that no unlocked click could ever produce.
    expect(planeCalls).toHaveLength(1)
    expect(planeCalls[0]).toEqual([2, 4, 6, 0, 0, 1])
  })

  it(`${name}: an idle-locked FIRST click still beats sketch-hover adoption (§5.2 precedence, unchanged by §2a)`, () => {
    // A genuinely hoverable sketch under the cursor — proves the lock wins
    // over a real competing adoption, not merely "there was nothing to
    // adopt anyway".
    const { scene, planeCalls } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const tool = make(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle lock, before any click
    anchor(tool, [2, 3, 4]) // first click resolves THROUGH the idle lock, not the hovered sketch
    finish(tool, [2, 9, 7]) // on the locked plane: x fixed at the anchor's 2

    expect(planeCalls).toHaveLength(1)
    expect(planeCalls[0]).toEqual([2, 3, 4, 1, 0, 0])
    expect(scene.sketch_begin_gesture).not.toHaveBeenCalledWith(TILTED_SKETCH)
  })
})

describe('Arc — mid-gesture plane lock (design §2a, chord stage only)', () => {
  function makeArc(scene: WasmScene) {
    return new ArcTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as {
      onPointerDown(snap: Snap | null, ray: Ray): void
      onKey(ev: KeyboardEvent): void
      capturingInput(): boolean
    }
  }

  it('an arrow key during the CHORD stage (only A placed) re-locks the plane through A', () => {
    const { scene, planeCalls } = makeWasmScene()
    const tool = makeArc(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY) // endpoint A — chord stage, nothing committed
    tool.onKey(makeKeyEvent('ArrowRight')) // red lock through A

    tool.onPointerDown(makeSnap([2, 6, 1]), RAY) // endpoint B (on the new red plane, x fixed at 2)
    tool.onPointerDown(makeSnap([2, 4.5, 2]), RAY) // bulge, off the chord line — commits

    expect(planeCalls).toHaveLength(1)
    expect(planeCalls[0]).toEqual([2, 3, 0, 1, 0, 0])
  })

  it('the same arrow again releases the chord-stage lock, reverting to the natural (ground) plane', () => {
    const { scene, planeCalls, segmentCalls } = makeWasmScene()
    const tool = makeArc(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onKey(makeKeyEvent('ArrowRight')) // release

    tool.onPointerDown(makeSnap([5, 3, 0]), RAY)
    tool.onPointerDown(makeSnap([3.5, 4.5, 0]), RAY) // bulge — commits on the ground

    expect(planeCalls).toHaveLength(0)
    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(segmentCalls.every((s) => s.a[2] === 0 && s.b[2] === 0)).toBe(true)
  })

  it('releasing an idle lock after an OFF-GROUND endpoint A, under a TILTED (non-vertical-blue) frame, reverts to the LITERAL ground orientation through A, not the frame\'s tilted blue axis (tool-parity DELTA review finding 1)', () => {
    const { scene, planeCalls, segmentCalls } = makeWasmScene({ frame: TILTED_FRAME_FLAT })
    const tool = makeArc(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle lock ON (red/X — still literal world +X in this fixture) BEFORE any click
    tool.onPointerDown(makeSnap([2, 4, 6]), RAY) // endpoint A resolves through the lock: vertical plane through (2,4,6), off the ground
    tool.onKey(makeKeyEvent('ArrowRight')) // release mid-gesture

    tool.onPointerDown(makeSnap([9, 9, 6]), RAY) // endpoint B, still on A's released plane
    tool.onPointerDown(makeSnap([5, 4, 6]), RAY) // bulge, off the chord line — commits

    // Anchor elevation preserved, same as the world-identity case above.
    expect(segmentCalls.length).toBeGreaterThan(0)
    expect(segmentCalls.every((s) => s.a[2] === 6 && s.b[2] === 6)).toBe(true)
    // Orientation must be the LITERAL ground normal [0,0,1] through A —
    // not the tilted frame's derived blue axis [0,-1,0].
    expect(planeCalls).toHaveLength(1)
    expect(planeCalls[0]).toEqual([2, 4, 6, 0, 0, 1])
  })

  it('an arrow key during the BULGE stage (A and B both placed) is NOT consumed as a re-lock — out of scope, see the module doc', () => {
    const { scene, planeCalls } = makeWasmScene()
    const tool = makeArc(scene)

    tool.onPointerDown(makeSnap([0, 0, 0]), RAY) // A
    tool.onPointerDown(makeSnap([4, 0, 0]), RAY) // B — chord fixed, now in bulge stage
    expect(() => tool.onKey(makeKeyEvent('ArrowRight'))).not.toThrow()

    tool.onPointerDown(makeSnap([2, 1, 0]), RAY) // bulge — commits on the ORIGINAL (ground) plane

    expect(planeCalls).toHaveLength(0) // no axis-plane sketch was minted
    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
  })
})
