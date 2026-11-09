/**
 * Drawing-plane cue (sketches on any plane, Phase 4 — the sketch-planes design
 * §6 bullet 1): each of the five draw tools (Line/Rectangle/Circle/Polygon/
 * Arc) exposes `activeDrawPlaneCue()`, the pure computation the Viewport
 * queries (duck-typed, like `snapConstraint`) to draw a grid patch on the
 * active drawing plane. A cue shows when anchored on a non-ground plane
 * (face or plane mode), idle with an active arrow-key lock and a tracked
 * hover point, under a Shift pin, or idle over a non-ground plane the next
 * click would land on — the inferred plane, a hovered sketch, or a hovered
 * face (view-plane design §5); everything else is null (the world grid
 * already covers ground). The actual grid math lives in `drawPlane.ts`'s `drawPlaneCue`,
 * unit-tested there; this file checks each tool WIRES its own state into
 * that helper correctly.
 *
 * One shared parameterized suite drives all four tools through a common
 * `DrawToolUnderTest` surface, mirroring `idlePlaneLock.test.ts`.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import { RectangleTool } from './RectangleTool'
import { CircleTool } from './CircleTool'
import { PolygonTool } from './PolygonTool'
import { ArcTool } from './ArcTool'
import { axisDrawPlane, groundDrawPlane, PATCH_OVER_FACES, type DrawPlane } from './drawPlane'
import { getDrawingAxes } from './drawingAxes'
import type { EditContext, Snap, SnapConstraint } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { followsViewPlane } from '../viewport/heldPointerReplay'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** The sketch handle `makeWasmScene`'s `sketchFill` answers the fill pick with. */
const FILL_SKETCH = 11n

/** A non-identity frame (tool-parity §4): red/green swapped-and-flipped
 *  relative to world, blue unchanged — the same fixture MoveTool.test.ts /
 *  RotateTool.test.ts use for their own moved-frame arrow-lock regressions. */
const ROTATED_FRAME_FLAT = [0, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1]

function makeSnap(p: readonly [number, number, number]): Snap {
  return { x: p[0], y: p[1], z: p[2], kind: 'plane' }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

function makePick(object: bigint, face: bigint) {
  return { object: () => object, face: () => face, depth: () => 1, instance: () => undefined, free: vi.fn() }
}

function makeWasmScene(opts: {
  /** `pick_face` result — set to return a pick for a face-mode fixture. */
  pick?: () => ReturnType<typeof makePick> | undefined
  /** The picked face's normal (also mirrored into `face_plane`'s offset). Default +Z (ground-plane face). */
  faceNormal?: readonly [number, number, number]
  /** The flat 12-float buffer `Scene::axes()` returns — defaults to world
   *  identity (this suite otherwise pins the legacy world-axis fast paths);
   *  a moved-frame test overrides it with `ROTATED_FRAME_FLAT`. */
  frame?: number[]
  /** A filled sketch region the fill pick (`pick_sketch_region` and its
   *  `_in_instance` sibling) answers for every ray: handle 11, on the plane
   *  through `origin` with `normal` (the `sketch_plane` buffer). */
  sketchFill?: { origin: readonly [number, number, number]; normal: readonly [number, number, number] }
} = {}): WasmScene {
  let sketchCounter = 90n
  const normal = opts.faceNormal ?? [0, 0, 1]
  const fill = opts.sketchFill
  const fillPick = () => fill === undefined
    ? undefined
    : { sketch: () => FILL_SKETCH, region: () => 1n, depth: () => 99, detached: () => false, free: vi.fn() }
  const scene = {
    // World-identity drawing axes (tool-parity §4) — this suite pins the
    // legacy world-axis fast paths; see drawPlane.test.ts for moved-frame
    // coverage of `axisDrawPlane` itself.
    axes: vi.fn(() => new Float64Array(opts.frame ?? [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    history_generation: vi.fn(() => 1n),
    begin_ground_sketch: vi.fn(() => { sketchCounter += 1n; return sketchCounter }),
    begin_sketch_on_plane: vi.fn(() => { sketchCounter += 1n; return sketchCounter }),
    pick_face: vi.fn(() => opts.pick?.()),
    pick_sketch: vi.fn(() => undefined), // no committed sketches in these fixtures
    pick_sketch_region: vi.fn(fillPick),
    pick_sketch_region_in_instance: vi.fn(fillPick),
    sketch_plane: vi.fn(() => new Float64Array(fill === undefined ? [0, 0, 0, 0, 0, 1] : [...fill.origin, ...fill.normal])),
    sketch_lines: vi.fn(() => new Float32Array()),
    sketch_is_detached: vi.fn(() => false),
    face_normal: vi.fn(() => new Float64Array(normal)),
    face_plane: vi.fn(() => new Float64Array([0, 0, 0, ...normal])),
    node_parent: vi.fn(() => undefined), // every picked object is top-level (eligible)
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_begin_curve: vi.fn(),
    sketch_begin_curve_with: vi.fn(),
    sketch_end_curve: vi.fn(),
    sketch_add_segment: vi.fn(() => ({
      new_edges: () => new BigUint64Array([]),
      regions_created: () => new BigUint64Array([]),
      regions_removed: () => new BigUint64Array([]),
      free: vi.fn(),
    })),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return scene as unknown as WasmScene
}

/** The subset of each draw tool's public surface these tests drive — every
 *  concrete tool (Line/Rectangle/Circle/Polygon/Arc) implements it identically. */
interface DrawToolUnderTest {
  onPointerDown(snap: Snap | null, ray: Ray): void
  onPointerMove(snap: Snap | null, ray: Ray): void
  onKey(ev: KeyboardEvent): void
  cancel(): void
  updateCamera(camera: THREE.Camera): void
  setShiftHeld(held: boolean): void
  statusHint(): string
  onPointerLeave(): void
  setEditContext(ctx: EditContext): void
  snapConstraint(ray: Ray): SnapConstraint | null
  activeDrawPlaneCue(): { plane: DrawPlane; through: [number, number, number] } | null
}

/** A camera on the Front side, low and level, looking along +Y: step 7
 *  infers the green plane over empty space. */
function frontCamera(): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
  cam.position.set(0, -10, 0.5)
  cam.up.set(0, 0, 1)
  cam.lookAt(0, 0, 0.5)
  cam.updateMatrixWorld(true)
  return cam
}

/** A ray from the Front side, off-centre, crossing the red plane x = 0 at
 *  (0, 0.9, 0.37) — inside a rectangle standing on that plane. */
const RED_RAY: Ray = { origin: [3, -3, 0.5], direction: [-3, 3.9, -0.13] }
const RED_FOOT = [0, 0.9, 0.37] as const
/** A level ray from the Front side, crossing y = 0 at (0.3, 0, 0.5). */
const FRONT_RAY: Ray = { origin: [0.3, -5, 0.5], direction: [0, 1, 0] }

const DRIVERS: { name: string; make(scene: WasmScene): DrawToolUnderTest }[] = [
  { name: 'Line', make: (scene) => new LineTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Rectangle', make: (scene) => new RectangleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Circle', make: (scene) => new CircleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Polygon', make: (scene) => new PolygonTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Arc', make: (scene) => new ArcTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
]

describe.each(DRIVERS)('$name — activeDrawPlaneCue', ({ make }) => {
  it('follows the view, so the Viewport replays its hover after an idle key, Shift or the camera settling', () => {
    expect(followsViewPlane(make(makeWasmScene()))).toBe(true)
  })

  it('null when idle and unlocked', () => {
    const tool = make(makeWasmScene())
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('null when idle-locked but no hover has landed yet', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('the locked axis plane through the tracked hover point, once one lands', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    expect(tool.activeDrawPlaneCue()).toEqual({ plane: axisDrawPlane(0, [2, 3, 4]), through: [2, 3, 4] })
  })

  it('switching the lock axis drops the stale hover (null again until the next move)', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    expect(tool.activeDrawPlaneCue()).not.toBeNull()
    tool.onKey(makeKeyEvent('ArrowUp')) // switch to blue/Z
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('null once the lock is cleared (idle Escape)', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('the frozen plane through the anchor, once a plane-mode gesture anchors on a locked (non-ground) plane', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerDown(makeSnap([2, 1, 3]), RAY) // first click anchors through (2,1,3)
    expect(tool.activeDrawPlaneCue()).toEqual({ plane: axisDrawPlane(0, [2, 1, 3]), through: [2, 1, 3] })
  })

  it('null once anchored on the GROUND plane (no lock, plain click)', () => {
    const tool = make(makeWasmScene())
    tool.onPointerDown(makeSnap([2, 1, 0]), RAY)
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('cancel() clears any anchored-plane cue back to idle-unlocked (null)', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap([2, 1, 3]), RAY)
    expect(tool.activeDrawPlaneCue()).not.toBeNull()
    tool.cancel()
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('the frozen plane through the anchor, once a FACE-mode gesture anchors on a non-ground face', () => {
    // A vertical face: normal +X, through the origin.
    const scene = makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [1, 0, 0] })
    const tool = make(scene)
    tool.onPointerDown(makeSnap([0, 2, 3]), RAY)
    const cue = tool.activeDrawPlaneCue()
    expect(cue).not.toBeNull()
    expect(cue!.plane.ground).toBe(false)
    expect(cue!.plane.normal).toEqual([1, 0, 0])
    expect(cue!.through).toEqual([0, 2, 3])
  })

  it('null once anchored on a GROUND-plane face (normal +Z, through z=0)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 0, 1] })
    const tool = make(scene)
    tool.onPointerDown(makeSnap([1, 2, 0]), RAY)
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  // ── Movable drawing axes (tool-parity §4) ──────────────────────────────
  it('under a moved frame, the idle-lock cue equals the plane a click at the same lock/point would resolve onto', () => {
    const scene = makeWasmScene({ frame: ROTATED_FRAME_FLAT })
    const tool = make(scene)
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    const frame = getDrawingAxes(scene)
    const cue = tool.activeDrawPlaneCue()
    expect(cue).toEqual({ plane: axisDrawPlane(0, [2, 3, 4], frame), through: [2, 3, 4] })
    // Axis 0 (red) of this moved frame is world [0,1,0], not world [1,0,0] —
    // the un-fixed bug (defaulting to world identity) would show a plane
    // through the raw world-X normal instead.
    expect(cue!.plane.normal).toEqual([0, 1, 0])
  })

  it('a Z lock through a z=0 hover under a moved frame does not suppress to null (the ground fast path is world-identity only)', () => {
    const scene = makeWasmScene({ frame: ROTATED_FRAME_FLAT })
    const tool = make(scene)
    tool.onKey(makeKeyEvent('ArrowUp')) // blue/Z lock
    tool.onPointerMove(makeSnap([1, 2, 0]), RAY)
    const cue = tool.activeDrawPlaneCue()
    expect(cue).not.toBeNull()
    expect(cue!.plane.ground).toBe(false)
  })

  // ── The idle hover plane (view-plane design §5) ─────────────────────────
  it('hovering a sketch fill on the red plane: a patch on that plane through the ray\'s foot, and no "Facing" line', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const cue = tool.activeDrawPlaneCue()
    expect(cue).not.toBeNull()
    expect(Math.abs(cue!.plane.normal[0])).toBeCloseTo(1, 12)
    expect(cue!.through[0]).toBeCloseTo(0, 12)
    expect(cue!.through[1]).toBeCloseTo(0.9, 12)
    expect(tool.statusHint()).not.toContain('Facing')
  })

  it('hovering a ground sketch from a low Front camera: no patch (the ground never shows one)', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [0, 0, 1] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap([0.3, 1, 0]), { origin: [0.3, -5, 0.5], direction: [0, 6, -0.5] })
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('an arrow lock beats a hovered sketch: the lock\'s plane through the hover point', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y lock
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    expect(tool.activeDrawPlaneCue()).toEqual({ plane: axisDrawPlane(1, [...RED_FOOT]), through: [...RED_FOOT] })
  })

  it('Shift pins the very plane the idle patch showed one move earlier', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const idle = tool.activeDrawPlaneCue()
    expect(idle).not.toBeNull()
    tool.setShiftHeld(true)
    const pinned = tool.activeDrawPlaneCue()
    expect(pinned?.plane).toEqual(idle!.plane)
  })

  it('a click over the hovered sketch anchors on the plane the idle patch showed', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const idle = tool.activeDrawPlaneCue()
    expect(idle).not.toBeNull()
    tool.onPointerDown(makeSnap(RED_FOOT), RED_RAY)
    const anchored = tool.activeDrawPlaneCue()
    expect(anchored?.plane).toEqual(idle!.plane)
    expect(anchored?.through).toEqual([...RED_FOOT])
  })

  it('hovering a face whose normal is +Y: a patch on its plane through the ray\'s foot at y = 0', () => {
    expect(PATCH_OVER_FACES).toBe(true)
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 1, 0] }))
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    const cue = tool.activeDrawPlaneCue()
    expect(cue).not.toBeNull()
    expect(Math.abs(cue!.plane.normal[1])).toBeCloseTo(1, 12)
    expect(cue!.through[1]).toBeCloseTo(0, 12)
    expect(tool.statusHint()).not.toContain('Facing')
  })

  it('hovering a face lying on the ground (normal +Z at z = 0): no patch', () => {
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 0, 1] }))
    tool.onPointerMove(makeSnap([0, 0, 0]), RAY)
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  // ── The pointer leaving the canvas ──────────────────────────────────────
  it('leaving the canvas drops the idle patch over a hovered sketch; the next hover brings it back', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const before = tool.activeDrawPlaneCue()
    expect(before).not.toBeNull()
    tool.onPointerLeave()
    expect(tool.activeDrawPlaneCue()).toBeNull()
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    expect(tool.activeDrawPlaneCue()).toEqual(before)
  })

  it('leaving the canvas drops the inferred plane\'s patch and its "Facing" line', () => {
    const tool = make(makeWasmScene())
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    expect(tool.activeDrawPlaneCue()).not.toBeNull()
    expect(tool.statusHint()).toContain('Facing')
    tool.onPointerLeave()
    expect(tool.activeDrawPlaneCue()).toBeNull()
    expect(tool.statusHint()).not.toContain('Facing')
  })

  it('leaving the canvas keeps an arrow lock and its patch', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerMove(makeSnap([1, 2, 0]), RAY)
    const locked = tool.activeDrawPlaneCue()
    expect(locked).toEqual({ plane: axisDrawPlane(0, [1, 2, 0]), through: [1, 2, 0] })
    tool.onPointerLeave()
    expect(tool.activeDrawPlaneCue()).toEqual(locked)
    expect(tool.statusHint()).toContain('Locked')
  })

  it('leaving the canvas keeps a Shift pin and its patch', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    tool.setShiftHeld(true)
    const pinned = tool.activeDrawPlaneCue()
    expect(pinned).not.toBeNull()
    tool.onPointerLeave()
    expect(tool.activeDrawPlaneCue()).toEqual(pinned)
  })

  // ── Inside an entered object ────────────────────────────────────────────
  it('inside an entered object, a sketch fill outside it puts no plane on the idle snap', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    // At the top level the same hover holds the snap to the sketch's plane…
    const top = tool.snapConstraint(RED_RAY)
    expect(top?.constraintPlane?.normal).toEqual([1, 0, 0])
    // …inside object 7 it does nothing: the click would do nothing either.
    tool.setEditContext({ kind: 'object', id: 7n })
    expect(tool.snapConstraint(RED_RAY)).toBeNull()
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('inside an entered object, the inferred plane over nothing puts no plane on the idle snap', () => {
    const tool = make(makeWasmScene())
    tool.updateCamera(frontCamera())
    expect(tool.snapConstraint(FRONT_RAY)?.fallbackPlane).toBeDefined()
    tool.setEditContext({ kind: 'object', id: 7n })
    expect(tool.snapConstraint(FRONT_RAY)).toBeNull()
  })

  it('inside an entered object, a face of that object still holds the snap to its plane', () => {
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 1, 0] }))
    tool.setEditContext({ kind: 'object', id: 7n })
    expect(tool.snapConstraint(FRONT_RAY)?.constraintPlane?.normal).toEqual([0, 1, 0])
  })
})

describe('activeDrawPlaneCue — sanity against groundDrawPlane()', () => {
  it('groundDrawPlane() itself is recognized as ground (sanity check for the fixtures above)', () => {
    expect(groundDrawPlane().ground).toBe(true)
  })
})
