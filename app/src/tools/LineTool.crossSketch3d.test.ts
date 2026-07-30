/**
 * The 3d-line staircase defect — closing a re-homed chain back onto its own
 * origin (tool-parity 3d-line playtest): draw 1 m along green from the
 * origin, blue-lock 1 m up, red-lock 1 m across, then aim the fourth
 * segment back at (0,0,0). Three defects conspired in that one gesture:
 *
 *  1. The origin vertex would not snap — the engine's constraint-plane
 *     filter dropped it (it lies on the FIRST sketch's plane, not the
 *     current frozen y = 1 one). Fixed engine-side by
 *     `SnapQuery::off_plane_points` (covered by
 *     `crates/inference/tests/inference_specs.rs`, the staircase specs,
 *     which drive the REAL engine from the same oblique camera); LineTool
 *     opts in via `snapConstraint().offPlanePoints`.
 *  2. A phantom "Endpoint" was reported where no vertex exists — the tool
 *     published its own live rubber band as a transient snap candidate, so
 *     the engine kept snapping to the previous frame's cursor point
 *     (`_publishTransient`'s doc tells the story).
 *  3. The committed segment stayed glued to the frozen y = 1 plane (the
 *     plane through the red axis Kurt saw) instead of reaching the origin —
 *     re-homing was gated on an ACTIVE axis lock, so an unlocked off-plane
 *     point commit projected/refused instead of re-homing.
 *
 * This file drives the REAL four-segment tool gesture — the engine half
 * (that the snap RESOLVES to the true origin vertex from an oblique camera)
 * lives in the inference specs; here the same gesture proves the tool
 * HONOURS that snap: the fourth segment commits exactly to (0,0,0), on a
 * freshly re-homed plane that genuinely contains it (the mock kernel
 * enforces the real `PointOffPlane` planarity check), while the honest
 * "spans two planes, can't close a face" refusal still fires — the snap
 * works AND no face is claimed.
 *
 * Rays are genuine oblique-camera rays (eye at (5,−4,3.5), matching the
 * inference-spec fixture), not the near-overhead RAY constant the suite's
 * older fixtures over-relied on.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const PLANE_DIST = 1e-9

/** Oblique modelling camera — the same eye as the inference staircase specs. */
const EYE: [number, number, number] = [5, -4, 3.5]

/** A pick ray from the oblique eye through a world-space target. */
function rayAt(target: readonly [number, number, number]): Ray {
  const direction: [number, number, number] = [
    target[0] - EYE[0],
    target[1] - EYE[1],
    target[2] - EYE[2],
  ]
  return { origin: [...EYE], direction }
}

function makeSnap(p: readonly [number, number, number], kind = 'plane', extra: Partial<Snap> = {}): Snap {
  return { x: p[0], y: p[1], z: p[2], kind, ...extra }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

type SegmentCall = { sketch: bigint; a: [number, number, number]; b: [number, number, number] }
type PlaneCall = [number, number, number, number, number, number]

/**
 * A mock kernel scene whose `sketch_add_segment` enforces plane membership
 * exactly like the real `Sketch::add_segment` (`PointOffPlane` within
 * `kernel::tol::PLANE_DIST`) — same harness as LineTool.rehome.test.ts, so
 * an un-re-homed off-plane commit FAILS here the way the kernel would
 * refuse it, rather than being silently accepted.
 */
function makeWasmScene() {
  const planes = new Map<bigint, [number, number, number, number, number, number]>()
  const segmentCalls: SegmentCall[] = []
  const planeCalls: PlaneCall[] = []
  const transientCalls: number[][] = []
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
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
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
    add_transient_segment: vi.fn((...args: number[]) => {
      transientCalls.push(args)
    }),
  }
  return { scene: scene as unknown as WasmScene, segmentCalls, planeCalls, transientCalls }
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, onCommit, onToast }
}

/**
 * Kurt's staircase, click for click: green 1 m from the origin, blue-locked
 * 1 m up (re-homes onto x = 0), red-locked 1 m across (re-homes onto
 * y = 1), lock cleared. Leaves the chain anchored at (1,1,1) on the frozen
 * y = 1 plane — the exact state the fourth segment is aimed from.
 */
function driveStaircase(tool: LineTool) {
  // Segment 1 — green, on the ground.
  tool.onPointerDown(makeSnap([0, 0, 0], 'endpoint'), rayAt([0, 0, 0]))
  tool.onPointerMove(makeSnap([0, 1, 0], 'on-axis', { direction: [0, 1, 0] }), rayAt([0, 1, 0]))
  tool.onPointerDown(makeSnap([0, 1, 0], 'on-axis', { direction: [0, 1, 0] }), rayAt([0, 1, 0]))
  // Segment 2 — blue-locked, straight up (leaves the ground plane).
  tool.onKey(makeKeyEvent('ArrowUp'))
  tool.onPointerMove(makeSnap([0, 1, 1], 'on-axis', { direction: [0, 0, 1] }), rayAt([0, 1, 1]))
  tool.onPointerDown(makeSnap([0, 1, 1], 'on-axis', { direction: [0, 0, 1] }), rayAt([0, 1, 1]))
  // Segment 3 — red-locked, across (leaves the x = 0 plane).
  tool.onKey(makeKeyEvent('ArrowRight'))
  tool.onPointerMove(makeSnap([1, 1, 1], 'on-axis', { direction: [1, 0, 0] }), rayAt([1, 1, 1]))
  tool.onPointerDown(makeSnap([1, 1, 1], 'on-axis', { direction: [1, 0, 0] }), rayAt([1, 1, 1]))
  // Clear the lock (same arrow toggles it off) — the fourth segment is free.
  tool.onKey(makeKeyEvent('ArrowRight'))
}

describe('LineTool — the 3d-line staircase: closing back onto the chain origin across three sketches', () => {
  it('honours the cross-sketch origin Endpoint: the fourth segment commits exactly to (0,0,0) on a re-homed plane, with the honest no-face toast', () => {
    const { scene, segmentCalls, planeCalls } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)

    driveStaircase(tool)
    expect(segmentCalls).toHaveLength(3)
    expect(onToast).not.toHaveBeenCalled()

    // Anchored on the frozen y = 1 plane, the tool OPTS IN to off-plane
    // point candidates — the engine-side half of this fix
    // (`SnapQuery::off_plane_points`) is unreachable without this bit.
    const constraint = tool.snapConstraint(rayAt([0, 0, 0]))
    expect(constraint?.offPlanePoints).toBe(true)
    expect(constraint?.anchor).toEqual([1, 1, 1])
    expect(constraint?.constraintPlane?.normal).toEqual([0, 1, 0])

    // Fourth segment: the engine (proven by the inference staircase specs)
    // now resolves the TRUE origin vertex as an Endpoint from this same
    // oblique camera. The tool must honour it, unlocked.
    const originSnap = makeSnap([0, 0, 0], 'endpoint')
    tool.onPointerMove(originSnap, rayAt([0, 0, 0]))
    tool.onPointerDown(originSnap, rayAt([0, 0, 0]))

    // The segment REACHES the origin — exactly, not a projection onto the
    // frozen y = 1 plane (the mock kernel would have thrown PointOffPlane
    // for any un-re-homed off-plane commit, and the pre-fix stray vertex
    // (−1.165, 1, −0.832) would fail these exact-coordinate assertions).
    expect(segmentCalls).toHaveLength(4)
    expect(segmentCalls[3].a).toEqual([1, 1, 1])
    expect(segmentCalls[3].b).toEqual([0, 0, 0])

    // It re-homed: a NEW sketch on a NEW plane through the anchor that
    // genuinely contains both endpoints — not the y = 1 sketch of segment 3.
    expect(segmentCalls[3].sketch).not.toBe(segmentCalls[2].sketch)
    expect(planeCalls).toHaveLength(3) // x = 0, y = 1, then the closing plane
    const closing = planeCalls[2]
    expect(closing.slice(0, 3)).toEqual([1, 1, 1])
    const [px, py, pz, nx, ny, nz] = closing
    const dOrigin = nx * (0 - px) + ny * (0 - py) + nz * (0 - pz)
    expect(Math.abs(dOrigin)).toBeLessThanOrEqual(PLANE_DIST)

    // The honest refusal is intact and coexists with the commit: the origin
    // belongs to the chain's FIRST sketch, so no face can be claimed — the
    // toast says so, but the segment above still committed.
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast).toHaveBeenCalledWith(expect.stringContaining('spans two planes'))
  })

  it('never publishes the live rubber band as a snap candidate (the phantom-"Endpoint" feedback loop)', () => {
    const { scene, transientCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    driveStaircase(tool)

    // Hover the fourth segment around — plane mode publishes NO transient
    // segments at all (committed segments are already persistent snap
    // candidates via reconcile/register_sketch): the engine must never see
    // the tool's own anchor→cursor rubber band, whose far endpoint is the
    // previous frame's cursor and therefore always inside the next pick
    // cone — the phantom "Endpoint" that followed the cursor and reported
    // a vertex where nothing exists.
    tool.onPointerMove(makeSnap([-1.2, 1, -0.84], 'plane'), rayAt([0, 0, 0]))
    tool.onPointerMove(makeSnap([0, 0, 0], 'endpoint'), rayAt([0, 0, 0]))

    expect(transientCalls).toHaveLength(0)
  })
})
