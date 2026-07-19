/**
 * Drawing-plane cue (sketches on any plane, Phase 4 — the sketch-planes design
 * §6 bullet 1): each of the four draw tools (Line/Rectangle/Circle/Arc)
 * exposes `activeDrawPlaneCue()`, the pure computation the Viewport queries
 * (duck-typed, like `snapConstraint`) to draw a grid patch on the active
 * drawing plane. Two cases produce a cue — anchored on a non-ground plane
 * (face or plane mode), or idle with an active arrow-key lock and a tracked
 * hover point — everything else is null (the world grid already covers
 * ground). The actual grid math lives in `drawPlane.ts`'s `drawPlaneCue`,
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
import { ArcTool } from './ArcTool'
import { axisDrawPlane, groundDrawPlane, type DrawPlane } from './drawPlane'
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

function makePick(object: bigint, face: bigint) {
  return { object: () => object, face: () => face, instance: () => undefined, free: vi.fn() }
}

function makeWasmScene(opts: {
  /** `pick_face` result — set to return a pick for a face-mode fixture. */
  pick?: () => ReturnType<typeof makePick> | undefined
  /** The picked face's normal (also mirrored into `face_plane`'s offset). Default +Z (ground-plane face). */
  faceNormal?: readonly [number, number, number]
} = {}): WasmScene {
  let sketchCounter = 90n
  const normal = opts.faceNormal ?? [0, 0, 1]
  const scene = {
    begin_ground_sketch: vi.fn(() => { sketchCounter += 1n; return sketchCounter }),
    begin_sketch_on_plane: vi.fn(() => { sketchCounter += 1n; return sketchCounter }),
    pick_face: vi.fn(() => opts.pick?.()),
    pick_sketch: vi.fn(() => undefined), // no committed sketches in these fixtures
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
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
 *  concrete tool (Line/Rectangle/Circle/Arc) implements it identically. */
interface DrawToolUnderTest {
  onPointerDown(snap: Snap | null, ray: Ray): void
  onPointerMove(snap: Snap | null, ray: Ray): void
  onKey(ev: KeyboardEvent): void
  cancel(): void
  activeDrawPlaneCue(): { plane: DrawPlane; through: [number, number, number] } | null
}

const DRIVERS: { name: string; make(scene: WasmScene): DrawToolUnderTest }[] = [
  { name: 'Line', make: (scene) => new LineTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Rectangle', make: (scene) => new RectangleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Circle', make: (scene) => new CircleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
  { name: 'Arc', make: (scene) => new ArcTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn()) as unknown as DrawToolUnderTest },
]

describe.each(DRIVERS)('$name — activeDrawPlaneCue', ({ make }) => {
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
})

describe('activeDrawPlaneCue — sanity against groundDrawPlane()', () => {
  it('groundDrawPlane() itself is recognized as ground (sanity check for the fixtures above)', () => {
    expect(groundDrawPlane().ground).toBe(true)
  })
})
