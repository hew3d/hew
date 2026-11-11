/**
 * The drawing-plane state (view-die design §3.4): each of the five draw
 * tools (Line/Rectangle/Circle/Polygon/Arc) exposes `drawPlaneState()`, the
 * value the view die's pips and the cursor cue read — the plane the next
 * click (or the gesture under way) uses, its pip, its hold, and what the
 * cursor is over. The shared bookkeeping lives in `drawPlane.ts`'s
 * `DrawPlaneTracker`, unit-tested there; the cue table in
 * `DrawPlaneCueLayer.ts`'s `planeCueFor`. This file checks each tool WIRES
 * its own gesture, lock, pin and hover into them, and that the cue each
 * state yields is the one §4 asks for.
 *
 * One shared parameterized suite drives all five tools through a common
 * `DrawToolUnderTest` surface, mirroring `idlePlaneLock.test.ts`.
 */
import { afterEach, describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import { RectangleTool } from './RectangleTool'
import { CircleTool } from './CircleTool'
import { PolygonTool } from './PolygonTool'
import { ArcTool } from './ArcTool'
import { axisDrawPlane, groundDrawPlane, type DrawPlane, type DrawPlaneState } from './drawPlane'
import { planeCueFor } from '../viewport/DrawPlaneCueLayer'
import { cursorPlaneOf } from '../viewport/drawPlanePips'
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
  /** `face_boundary`'s answer for every face (the lean outline). */
  faceLoop?: number[]
  /** `instance_pose`'s answer for every instance (row-major 3×4). */
  pose?: number[]
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
    face_boundary: vi.fn(() => new Float32Array(opts.faceLoop ?? [])),
    // Every face flat: four corners, one normal.
    object_mesh: vi.fn(() => ({ normals: () => new Float32Array([...normal, ...normal, ...normal, ...normal]), free: vi.fn() })),
    face_mesh_range: vi.fn(() => new Uint32Array([0, 4])),
    instance_pose: vi.fn(() => (opts.pose === undefined ? undefined : new Float64Array(opts.pose))),
    component_member_sketches: vi.fn(() => new BigUint64Array([])),
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
  drawPlaneState(): DrawPlaneState
  togglePlanePin(): void
  pinnablePlane(): DrawPlane | null
  planeKeyReleases(key: string): boolean
  customPipPlane(): DrawPlane | null
  pinCustomCandidate(): void
  customPipTtl(): number | null
  setFaceEligibility(pred: ((object: bigint, instance: bigint | undefined) => boolean) | null): void
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

/** A camera above and in front, looking down at about 45°: empty space
 *  infers the blue plane. */
function highCamera(): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
  cam.position.set(0, -6, 6)
  cam.up.set(0, 0, 1)
  cam.lookAt(0, 0, 0)
  cam.updateMatrixWorld(true)
  return cam
}

const DRIVERS: { name: string; make(scene: WasmScene): DrawToolUnderTest }[] = [
  { name: 'Line', make: (scene) => new LineTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Rectangle', make: (scene) => new RectangleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Circle', make: (scene) => new CircleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Polygon', make: (scene) => new PolygonTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Arc', make: (scene) => new ArcTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
]

/** The cue the Viewport draws for a tool right now. */
const cueOf = (tool: DrawToolUnderTest) => planeCueFor(tool.drawPlaneState())

describe.each(DRIVERS)('$name — drawPlaneState', ({ make }) => {
  it('follows the view, so the Viewport replays its hover after an idle key, Shift or the camera settling', () => {
    expect(followsViewPlane(make(makeWasmScene()))).toBe(true)
  })

  it('idle and unlocked with no camera or hover yet: the ground, inferred, no hover, no cue', () => {
    const tool = make(makeWasmScene())
    const state = tool.drawPlaneState()
    expect(state.plane).toEqual(groundDrawPlane())
    expect(state.pip).toBe('blue')
    expect(state.hold).toBe('inferred')
    expect(state.hover).toBeNull()
    expect(cueOf(tool)).toBeNull()
  })

  it('before any hover, the plane is the one the view faces: green from a low Front camera', () => {
    const tool = make(makeWasmScene())
    tool.updateCamera(frontCamera())
    expect(tool.drawPlaneState().pip).toBe('green')
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('idle-locked with no hover yet: the locked pip, no hover, no cue', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    const state = tool.drawPlaneState()
    expect(state.pip).toBe('red')
    expect(state.hold).toBe('locked')
    expect(state.hover).toBeNull()
    expect(cueOf(tool)).toBeNull()
  })

  it('the locked axis plane through the tracked hover point, once one lands; the held patch there', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    const state = tool.drawPlaneState()
    expect(state.plane).toEqual(axisDrawPlane(0, [2, 3, 4]))
    expect(state.hover?.point).toEqual([2, 3, 4])
    expect(cueOf(tool)).toEqual({ kind: 'patch', pip: 'red', plane: axisDrawPlane(0, [2, 3, 4]), through: [2, 3, 4], held: true })
  })

  it('switching the lock axis drops the stale hover (none again until the next move)', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    expect(cueOf(tool)).not.toBeNull()
    tool.onKey(makeKeyEvent('ArrowUp')) // switch to blue/Z
    expect(tool.drawPlaneState().pip).toBe('blue')
    expect(tool.drawPlaneState().hover).toBeNull()
    expect(cueOf(tool)).toBeNull()
  })

  it('an idle Escape releases the lock back to inferred', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('a plane-mode gesture anchored on a locked plane: the frozen plane through the anchor, still locked', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerDown(makeSnap([2, 1, 3]), RAY) // first click anchors through (2,1,3)
    const state = tool.drawPlaneState()
    expect(state.plane).toEqual(axisDrawPlane(0, [2, 1, 3]))
    expect(state.pip).toBe('red')
    expect(state.hold).toBe('locked')
  })

  it('a plain click on the ground anchors: the ground, held as anchored, the held patch at the cursor', () => {
    const tool = make(makeWasmScene())
    tool.onPointerDown(makeSnap([2, 1, 0]), RAY)
    tool.onPointerMove(makeSnap([0, 0, 0]), RAY)
    const state = tool.drawPlaneState()
    expect(state.plane.ground).toBe(true)
    expect(state.hold).toBe('anchored')
    // The patch sits at the cursor (RAY's foot on the ground), not at the anchor.
    expect(cueOf(tool)).toMatchObject({ kind: 'patch', pip: 'blue', through: [0, 0, 0], held: true })
  })

  it('cancel() returns an anchored, locked gesture to idle and unlocked', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap([2, 1, 3]), RAY)
    expect(tool.drawPlaneState().hold).toBe('locked')
    tool.cancel()
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('a FACE-mode gesture on a non-ground face: that face\'s plane, anchored', () => {
    // A vertical face: normal +X, through the origin.
    const scene = makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [1, 0, 0] })
    const tool = make(scene)
    tool.onPointerDown(makeSnap([0, 2, 3]), RAY)
    const state = tool.drawPlaneState()
    expect(state.plane.ground).toBe(false)
    expect(state.plane.normal).toEqual([1, 0, 0])
    expect(state.plane.origin).toEqual([0, 2, 3])
    expect(state.pip).toBe('red')
    expect(state.hold).toBe('anchored')
  })

  it('a FACE-mode gesture on a ground-plane face (normal +Z, through z=0): the ground, anchored', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 0, 1] })
    const tool = make(scene)
    tool.onPointerDown(makeSnap([1, 2, 0]), RAY)
    expect(tool.drawPlaneState().plane.ground).toBe(true)
    expect(tool.drawPlaneState().hold).toBe('anchored')
  })

  // ── Movable drawing axes (tool-parity §4) ──────────────────────────────
  it('under a moved frame, the locked plane equals the plane a click at the same lock/point would resolve onto, and lights red', () => {
    const scene = makeWasmScene({ frame: ROTATED_FRAME_FLAT })
    const tool = make(scene)
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY)
    const frame = getDrawingAxes(scene)
    const state = tool.drawPlaneState()
    expect(state.plane).toEqual(axisDrawPlane(0, [2, 3, 4], frame))
    // Axis 0 (red) of this moved frame is world [0,1,0], not world [1,0,0].
    expect(state.plane.normal).toEqual([0, 1, 0])
    expect(state.pip).toBe('red')
  })

  it('a Z lock through a z=0 hover under a moved frame is not the literal ground (world identity only)', () => {
    const scene = makeWasmScene({ frame: ROTATED_FRAME_FLAT })
    const tool = make(scene)
    tool.onKey(makeKeyEvent('ArrowUp')) // blue/Z lock
    tool.onPointerMove(makeSnap([1, 2, 0]), RAY)
    expect(tool.drawPlaneState().plane.ground).toBe(false)
    expect(tool.drawPlaneState().pip).toBe('blue')
  })

  it('a face turned 30° about blue lights the custom pip under a moved frame too', () => {
    // ROTATED_FRAME_FLAT's axes are world +Y, −X, +Z: a 30° face lies along
    // none of them.
    const r = (30 * Math.PI) / 180
    const scene = makeWasmScene({ frame: ROTATED_FRAME_FLAT, pick: () => makePick(7n, 2n), faceNormal: [Math.cos(r), Math.sin(r), 0] })
    const tool = make(scene)
    tool.onPointerMove(makeSnap([0, 0, 0.5]), FRONT_RAY)
    expect(tool.drawPlaneState().pip).toBe('custom')
  })

  // ── The idle hover plane (view-plane design §5) ─────────────────────────
  it('hovering a sketch fill on the red plane: that plane on the red pip, kind sketch, no cue, no "Facing" line', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const state = tool.drawPlaneState()
    expect(Math.abs(state.plane.normal[0])).toBeCloseTo(1, 12)
    expect(state.pip).toBe('red')
    expect(state.hover?.kind).toBe('sketch')
    expect(state.hover?.point[0]).toBeCloseTo(0, 12)
    expect(state.hover?.point[1]).toBeCloseTo(0.9, 12)
    expect(cueOf(tool)).toBeNull()
    expect(tool.statusHint()).not.toContain('Facing')
  })

  it('hovering a ground sketch from a low Front camera: the ground, no cue', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [0, 0, 1] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap([0.3, 1, 0]), { origin: [0.3, -5, 0.5], direction: [0, 6, -0.5] })
    expect(tool.drawPlaneState().plane.ground).toBe(true)
    expect(cueOf(tool)).toBeNull()
  })

  it('an arrow lock beats a hovered sketch: the lock\'s plane through the hover point', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onKey(makeKeyEvent('ArrowLeft')) // green/Y lock
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    expect(tool.drawPlaneState().plane).toEqual(axisDrawPlane(1, [...RED_FOOT]))
    expect(tool.drawPlaneState().pip).toBe('green')
  })

  it('Shift pins the very plane the state showed one move earlier', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const idle = tool.drawPlaneState()
    tool.setShiftHeld(true)
    const pinned = tool.drawPlaneState()
    expect(pinned.plane).toEqual(idle.plane)
    expect(pinned.hold).toBe('pinned')
    expect(cueOf(tool)).toMatchObject({ kind: 'patch', held: true })
  })

  it('a click over the hovered sketch anchors on the plane the idle state showed', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const idle = tool.drawPlaneState()
    tool.onPointerDown(makeSnap(RED_FOOT), RED_RAY)
    const anchored = tool.drawPlaneState()
    expect(anchored.plane.normal).toEqual(idle.plane.normal)
    expect(anchored.plane.origin[0]).toBeCloseTo(0, 12)
    expect(anchored.hold).toBe('anchored')
  })

  it('hovering a face whose normal is +Y: green, kind face, and no cue (a plain face shows none)', () => {
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 1, 0] }))
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    const state = tool.drawPlaneState()
    expect(Math.abs(state.plane.normal[1])).toBeCloseTo(1, 12)
    expect(state.pip).toBe('green')
    expect(state.hover?.kind).toBe('face')
    expect(cueOf(tool)).toBeNull()
    expect(tool.statusHint()).not.toContain('Facing')
  })

  it('hovering a face lying on the ground (normal +Z at z = 0): the ground, no cue', () => {
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 0, 1] }))
    tool.onPointerMove(makeSnap([0, 0, 0]), RAY)
    expect(tool.drawPlaneState().plane.ground).toBe(true)
    expect(cueOf(tool)).toBeNull()
  })

  it('empty space on the inferred green plane: the small patch at the ray\'s hit, inferred weight', () => {
    const tool = make(makeWasmScene())
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    expect(tool.drawPlaneState().hover?.kind).toBe('space')
    const cue = cueOf(tool)
    expect(cue).toMatchObject({ kind: 'patch', pip: 'green', held: false })
    if (cue?.kind === 'patch') {
      expect(cue.through[0]).toBeCloseTo(0.3, 12)
      expect(cue.through[1]).toBeCloseTo(0, 12)
      expect(cue.through[2]).toBeCloseTo(0.5, 12)
    }
  })

  it('leaning onto a face at its corner: the face\'s outline in its plane\'s color', () => {
    // Every ray strikes face 2 of object 7, a red-plane face through the
    // origin; the snap sits on that object's corner.
    const loop = [0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1]
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [1, 0, 0], faceLoop: loop }))
    const corner = { x: 0, y: 1, z: 1, kind: 'endpoint', object: 7n, elementKind: 'vertex' } as unknown as Snap
    tool.onPointerMove(corner, { origin: [5, 0.98, 0.98], direction: [-1, 0, 0] })
    const state = tool.drawPlaneState()
    expect(state.hover?.kind).toBe('lean')
    expect(state.hover?.faceLoop).toEqual([[0, 0, 0], [0, 1, 0], [0, 1, 1], [0, 0, 1]])
    expect(cueOf(tool)).toEqual({ kind: 'outline', pip: 'red', normal: [1, 0, 0], loop: [[0, 0, 0], [0, 1, 0], [0, 1, 1], [0, 0, 1]] })
  })

  // ── The pointer leaving the canvas ──────────────────────────────────────
  it('leaving the canvas drops the hover over a sketch but keeps its plane; the next hover brings it back', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    const before = tool.drawPlaneState()
    expect(before.hover).not.toBeNull()
    tool.onPointerLeave()
    expect(tool.drawPlaneState().hover).toBeNull()
    expect(tool.drawPlaneState().plane).toEqual(before.plane)
    expect(tool.drawPlaneState().pip).toBe('red')
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    expect(tool.drawPlaneState()).toEqual(before)
  })

  it('leaving the canvas drops the inferred plane\'s patch and its "Facing" line, but the pip stays lit', () => {
    const tool = make(makeWasmScene())
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    expect(cueOf(tool)).not.toBeNull()
    expect(tool.statusHint()).toContain('Facing')
    tool.onPointerLeave()
    expect(cueOf(tool)).toBeNull()
    expect(tool.statusHint()).not.toContain('Facing')
    expect(tool.drawPlaneState().pip).toBe('green')
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('the last hover plane survives the pointer leaving even when the camera then turns', () => {
    const tool = make(makeWasmScene())
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    tool.onPointerLeave()
    const top = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
    top.position.set(0, -1, 10)
    top.up.set(0, 0, 1)
    top.lookAt(0, 0, 0)
    top.updateMatrixWorld(true)
    tool.updateCamera(top)
    expect(tool.drawPlaneState().pip).toBe('green')
  })

  it('leaving the canvas keeps an arrow lock (its hover goes until the pointer returns)', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerMove(makeSnap([1, 2, 0]), RAY)
    const locked = tool.drawPlaneState()
    expect(locked.plane).toEqual(axisDrawPlane(0, [1, 2, 0]))
    tool.onPointerLeave()
    expect(tool.drawPlaneState().plane).toEqual(locked.plane)
    expect(tool.drawPlaneState().hold).toBe('locked')
    expect(tool.drawPlaneState().hover).toBeNull()
    expect(tool.statusHint()).toContain('Locked')
  })

  it('a lock made off the canvas (a pip click reaches onKey) shows at once, and its release returns the tracked plane', () => {
    const tool = make(makeWasmScene())
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    tool.onPointerLeave()
    tool.onKey(makeKeyEvent('ArrowUp'))
    expect(tool.drawPlaneState().pip).toBe('blue')
    expect(tool.drawPlaneState().hold).toBe('locked')
    tool.onKey(makeKeyEvent('ArrowUp'))
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('leaving the canvas keeps a Shift pin', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    tool.setShiftHeld(true)
    const pinned = tool.drawPlaneState()
    tool.onPointerLeave()
    expect(tool.drawPlaneState().plane).toEqual(pinned.plane)
    expect(tool.drawPlaneState().hold).toBe('pinned')
  })

  // ── The custom pip: pin and release (view-die design §3.3) ─────────────
  it('a custom plane under the cursor is pinnable; togglePlanePin pins it as Shift would, and releases it', () => {
    const r = (30 * Math.PI) / 180
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [Math.cos(r), Math.sin(r), 0] }))
    tool.onPointerMove(makeSnap([0, 0, 0.5]), FRONT_RAY)
    expect(tool.drawPlaneState().pip).toBe('custom')
    expect(tool.pinnablePlane()?.normal[0]).toBeCloseTo(Math.cos(r), 12)
    tool.onPointerLeave() // to the pip
    tool.togglePlanePin()
    expect(tool.drawPlaneState().hold).toBe('pinned')
    expect(tool.drawPlaneState().pip).toBe('custom')
    tool.togglePlanePin()
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('togglePlanePin ignores a held physical Shift (no autorepeat guard), and a pin displaces a lock', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    tool.setShiftHeld(true) // pinned, Shift still down
    tool.togglePlanePin() // released by the click
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('mid-gesture there is nothing to pin, and togglePlanePin does nothing', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap([2, 1, 3]), RAY)
    expect(tool.pinnablePlane()).toBeNull()
    tool.togglePlanePin()
    expect(tool.drawPlaneState().hold).toBe('locked')
  })

  // ── Pips as keys (view-die design §3.3) ────────────────────────────────
  it('under a Shift pin, another axis pip (its arrow) releases the pin and locks that plane', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    tool.setShiftHeld(true)
    tool.setShiftHeld(false)
    expect(tool.drawPlaneState().hold).toBe('pinned')
    tool.onPointerLeave() // to the pips
    tool.onKey(makeKeyEvent('ArrowUp')) // the blue pip
    const state = tool.drawPlaneState()
    expect(state.hold).toBe('locked')
    expect(state.pip).toBe('blue')
    // The pin is gone: nothing left to release, and a second blue click
    // releases the lock.
    expect(tool.pinnablePlane()).toBeNull()
    expect(tool.planeKeyReleases('ArrowUp')).toBe(true)
    tool.onKey(makeKeyEvent('ArrowUp'))
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  it('planeKeyReleases idle: only the arrow of the active lock', () => {
    const tool = make(makeWasmScene())
    expect(tool.planeKeyReleases('ArrowRight')).toBe(false)
    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.planeKeyReleases('ArrowRight')).toBe(true)
    expect(tool.planeKeyReleases('ArrowLeft')).toBe(false)
    expect(tool.planeKeyReleases('Escape')).toBe(false)
  })

  it('planeKeyReleases mid-gesture: the stage\'s own arrow meaning (Line\'s arrows lock the segment axis there)', () => {
    const tool = make(makeWasmScene())
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap([2, 1, 3]), RAY)
    expect(tool.drawPlaneState().hold).toBe('locked')
    const releases = tool.planeKeyReleases('ArrowRight')
    if (tool instanceof LineTool) {
      expect(releases).toBe(false)
    } else {
      expect(releases).toBe(true)
      // And the arrow does release it: the gesture reverts to its natural plane.
      tool.onKey(makeKeyEvent('ArrowRight'))
      expect(tool.drawPlaneState().hold).toBe('anchored')
    }
    expect(tool.planeKeyReleases('ArrowLeft')).toBe(false)
  })

  it('a lock set mid-gesture displaces a Shift pin: after Escape only the lock is left, and every consumer agrees', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    if (tool instanceof LineTool) return // Line's mid-chain arrows lock the segment axis, not the plane
    tool.updateCamera(frontCamera())
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    tool.setShiftHeld(true) // pin the red sketch plane
    tool.setShiftHeld(false)
    tool.onPointerDown(makeSnap(RED_FOOT), RED_RAY) // anchor on the pin
    expect(tool.drawPlaneState().hold).toBe('pinned')
    tool.onKey(makeKeyEvent('ArrowLeft')) // green lock through the anchor
    expect(tool.drawPlaneState().hold).toBe('locked')
    expect(tool.pinnablePlane()).toBeNull()
    tool.onKey(makeKeyEvent('Escape')) // abort the gesture; the aiming choice stays
    const state = tool.drawPlaneState()
    expect(state.hold).toBe('locked')
    expect(state.pip).toBe('green')
    expect(tool.statusHint()).toContain('Locked')
    // The idle lock's snap is free (the pin's would constrain it to red).
    expect(tool.snapConstraint(RED_RAY)).toBeNull()
    // A second Escape releases it: nothing left behind.
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.drawPlaneState().hold).toBe('inferred')
  })

  // ── Moved drawing axes on the ground (condition of view-die §3.4/§4) ────
  it('a moved frame whose blue plane IS the ground (shifted within it, turned about blue): no patch over empty space', () => {
    // Origin (2, 3, 0), turned 90° about blue.
    const scene = makeWasmScene({ frame: [2, 3, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1] })
    const tool = make(scene)
    tool.updateCamera(highCamera())
    tool.onPointerMove(makeSnap([0.5, 0.5, 0]), { origin: [0.5, -5.5, 6], direction: [0, 6, -6] })
    const state = tool.drawPlaneState()
    expect(state.pip).toBe('blue')
    expect(state.hover?.kind).toBe('ground')
    expect(cueOf(tool)).toBeNull()
  })

  it('a moved frame raised off the ground keeps the patch over empty space', () => {
    const scene = makeWasmScene({ frame: [0, 0, 0.75, 1, 0, 0, 0, 1, 0, 0, 0, 1] })
    const tool = make(scene)
    tool.updateCamera(highCamera())
    tool.onPointerMove(makeSnap([0.5, 0.5, 0.75]), { origin: [0.5, -5.5, 6], direction: [0, 6, -5.25] })
    expect(tool.drawPlaneState().hover?.kind).toBe('space')
    expect(cueOf(tool)).toMatchObject({ kind: 'patch', pip: 'blue', held: false })
  })

  // ── Entered sessions and instances (view-die design §4, E10) ────────────
  // A group or component session hands its members to the world tree as
  // ordinary top-level objects (ARCHITECTURE.md 2.11): the tools see them
  // at the top level, exactly like any world object.
  it('inside an entered session: a lock shows the held patch, and its hold carries the padlock', () => {
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [0, 1, 0] }))
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    tool.onKey(makeKeyEvent('ArrowLeft'))
    tool.onPointerMove(makeSnap([0.3, 0, 0.5]), FRONT_RAY)
    expect(tool.drawPlaneState().hold).toBe('locked')
    expect(cueOf(tool)).toMatchObject({ kind: 'patch', pip: 'green', held: true })
  })

  it('inside an entered session: a lean onto a member face outlines it', () => {
    const loop = [0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1]
    const tool = make(makeWasmScene({ pick: () => makePick(7n, 2n), faceNormal: [1, 0, 0], faceLoop: loop }))
    const corner = { x: 0, y: 1, z: 1, kind: 'endpoint', object: 7n, elementKind: 'vertex' } as unknown as Snap
    tool.onPointerMove(corner, { origin: [5, 0.98, 0.98], direction: [-1, 0, 0] })
    expect(cueOf(tool)).toMatchObject({ kind: 'outline', pip: 'red' })
  })

  it('inside an entered component instance: the leaned-to face\'s outline is posed into world space', () => {
    // The instance sits 10 m along red; its member face lies on its local x = 0.
    const loop = [0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1]
    const pose = [1, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 0]
    const pick = () => ({ ...makePick(7n, 2n), instance: () => 5n }) as unknown as ReturnType<typeof makePick>
    const tool = make(makeWasmScene({ pick, faceNormal: [1, 0, 0], faceLoop: loop, pose }))
    tool.setEditContext({ kind: 'instance', id: 5n, component: 3n })
    // The Viewport's injected policy there: the entered instance's faces.
    tool.setFaceEligibility((_object, instance) => instance === 5n)
    const corner = { x: 10, y: 1, z: 1, kind: 'endpoint', object: 7n, instance: 5n, elementKind: 'vertex' } as unknown as Snap
    tool.onPointerMove(corner, { origin: [15, 0.98, 0.98], direction: [-1, 0, 0] })
    const cue = cueOf(tool)
    expect(cue?.kind).toBe('outline')
    if (cue?.kind === 'outline') expect(cue.loop).toEqual([[10, 0, 0], [10, 1, 0], [10, 1, 1], [10, 0, 1]])
  })

  // ── The custom candidate: crossing the canvas to the violet pip (D4) ──
  describe('the custom pip\'s candidate', () => {
    let now = 10_000
    afterEach(() => vi.restoreAllMocks())
    const r = (30 * Math.PI) / 180
    const TURNED_N: [number, number, number] = [Math.cos(r), Math.sin(r), 0]
    /** A tool over a face turned 30° (custom), then — `overFace = false` —
     *  over the bare ground; the clock is the test's. */
    function crossing() {
      now = 10_000
      vi.spyOn(performance, 'now').mockImplementation(() => now)
      let overFace = true
      const scene = makeWasmScene({ pick: () => (overFace ? makePick(7n, 2n) : undefined), faceNormal: TURNED_N })
      const tool = make(scene)
      const hoverFace = () => { overFace = true; tool.onPointerMove(makeSnap([0, 0, 0.5]), FRONT_RAY) }
      const hoverGround = () => { overFace = false; tool.onPointerMove(makeSnap([2, 1, 0]), RAY) }
      return { tool, scene: scene as unknown as Record<string, ReturnType<typeof vi.fn>>, hoverFace, hoverGround }
    }

    it('D4: rotated face, across the ground, onto the pip: enabled, the filled pip honest, and the click pins the face', () => {
      const { tool, hoverFace, hoverGround } = crossing()
      hoverFace()
      now += 300
      hoverGround()
      hoverGround()
      now += 300
      tool.onPointerLeave()
      // The display is the real hover plane: blue over the ground.
      expect(tool.drawPlaneState().pip).toBe('blue')
      expect(tool.customPipPlane()?.normal).toEqual(TURNED_N)
      now += 5000 // dwelling on the pip: no timeout while off the canvas
      expect(tool.customPipPlane()).not.toBeNull()
      tool.pinCustomCandidate()
      expect(tool.drawPlaneState().hold).toBe('pinned')
      expect(tool.drawPlaneState().pip).toBe('custom')
      expect(tool.drawPlaneState().plane.normal).toEqual(TURNED_N)
      tool.pinCustomCandidate() // a second click releases
      expect(tool.drawPlaneState().hold).toBe('inferred')
      // A release spends the candidate (D5's "after release").
      expect(tool.customPipPlane()).toBeNull()
    })

    it('Shift is unchanged: it pins the tracked hover (the ground), not the candidate', () => {
      const { tool, hoverFace, hoverGround } = crossing()
      hoverFace()
      hoverGround()
      tool.setShiftHeld(true)
      expect(tool.drawPlaneState().pip).toBe('blue')
      expect(tool.drawPlaneState().hold).toBe('pinned')
    })

    it('D5: over the ground for more than 2 s, then onto the pip: disabled', () => {
      const { tool, hoverFace, hoverGround } = crossing()
      hoverFace()
      hoverGround()
      expect(tool.customPipTtl()).toBe(2000)
      now += 2001
      tool.onPointerLeave()
      expect(tool.customPipPlane()).toBeNull()
      expect(tool.customPipTtl()).toBeNull()
    })

    it('cleared by an arrow lock, a press, cancel(), a document change, and an axes move; not by a camera move', () => {
      const clears: [string, (t: DrawToolUnderTest, s: Record<string, ReturnType<typeof vi.fn>>) => void][] = [
        ['arrow lock', (t) => { t.onKey(makeKeyEvent('ArrowUp')); t.onKey(makeKeyEvent('ArrowUp')) }],
        ['press', (t) => {
          t.onPointerDown(makeSnap([2, 1, 0]), RAY)
          // Spent at the press itself, not only by the gesture's end.
          expect(t.customPipTtl()).toBeNull()
          t.onKey(makeKeyEvent('Escape'))
        }],
        ['cancel (Esc, tool switch)', (t) => t.cancel()],
        ['document change (edit, undo, redo)', (_t, s) => { s.history_generation.mockReturnValue(2n) }],
        ['drawing-axes move', (_t, s) => { s.axes.mockReturnValue(new Float64Array(ROTATED_FRAME_FLAT)) }],
      ]
      for (const [, clear] of clears) {
        const { tool, scene, hoverFace, hoverGround } = crossing()
        hoverFace()
        hoverGround()
        clear(tool, scene)
        tool.onPointerLeave()
        expect(tool.customPipPlane()).toBeNull()
      }
      const { tool, hoverFace, hoverGround } = crossing()
      hoverFace()
      hoverGround()
      tool.updateCamera(frontCamera())
      tool.onPointerLeave()
      expect(tool.customPipPlane()).not.toBeNull()
    })

    it('a pin made, or released, spends it', () => {
      const { tool, hoverFace, hoverGround } = crossing()
      hoverFace()
      tool.setShiftHeld(true) // pins the face
      tool.setShiftHeld(false)
      tool.setShiftHeld(true) // releases
      hoverGround()
      tool.onPointerLeave()
      expect(tool.customPipPlane()).toBeNull()
    })

    it('mid-gesture the custom pip is disabled', () => {
      const { tool, hoverFace } = crossing()
      hoverFace()
      tool.onKey(makeKeyEvent('ArrowRight'))
      tool.onPointerDown(makeSnap([2, 1, 3]), RAY)
      expect(tool.customPipPlane()).toBeNull()
      tool.pinCustomCandidate()
      expect(tool.drawPlaneState().hold).toBe('locked')
    })
  })

  // ── Inside an entered object ────────────────────────────────────────────
  it('inside an entered object, a sketch fill outside it puts no plane on the idle snap and is no hover', () => {
    const tool = make(makeWasmScene({ sketchFill: { origin: [0, 0, 0], normal: [1, 0, 0] } }))
    tool.updateCamera(frontCamera())
    // At the top level the same hover holds the snap to the sketch's plane…
    const top = tool.snapConstraint(RED_RAY)
    expect(top?.constraintPlane?.normal).toEqual([1, 0, 0])
    // …inside object 7 it does nothing: the click would do nothing either.
    tool.setEditContext({ kind: 'object', id: 7n })
    expect(tool.snapConstraint(RED_RAY)).toBeNull()
    tool.onPointerMove(makeSnap(RED_FOOT), RED_RAY)
    expect(tool.drawPlaneState().hover).toBeNull()
    expect(cueOf(tool)).toBeNull()
    // Nor does the snap dot's ring or the chip name a plane there.
    expect(cursorPlaneOf(tool.drawPlaneState())).toBeUndefined()
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

describe('drawPlaneState — sanity against groundDrawPlane()', () => {
  it('groundDrawPlane() itself is recognized as ground (sanity check for the fixtures above)', () => {
    expect(groundDrawPlane().ground).toBe(true)
  })
})
