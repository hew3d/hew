/**
 * The degenerate-segment gate must sit EXACTLY on the kernel's own line
 * (`SketchError::DegenerateSegment`: endpoints within `tol::POINT_MERGE`
 * = 1e-9 of each other), because two OTHER layers already do:
 *
 *  - the kernel itself refuses below `POINT_MERGE` and accepts above it;
 *  - the inference engine's lock-projection disqualification (the locked-
 *    click degeneracy fix) reads `tol::POINT_MERGE` directly, so a snap
 *    whose lock projection lands ABOVE `POINT_MERGE` is returned to the
 *    tool as a real, committable winner.
 *
 * The tool's gates were a legacy hardcoded 1e-8 — an order of magnitude
 * looser — which left a band [1e-9, 1e-8) where the engine hands back a
 * genuine point (e.g. a real vertex 5 nm along the lock from the anchor,
 * kept by `off_plane_points`, which has no distance gate for point kinds)
 * and the tool still refused it with the "same as the last point" toast:
 * the original metre-scale locked-click failure, narrowed but not closed.
 * A metre-scale sweep cannot see a nanometre band, so these cases probe
 * the band itself, on both sides of the true threshold.
 *
 * The mock kernel enforces BOTH real `Sketch::add_segment` refusals —
 * `PointOffPlane` (as in LineTool.crossSketch3d.test.ts) and
 * `DegenerateSegment` at the kernel's exact `POINT_MERGE` — so a gate that
 * under-refuses (looser than the kernel) fails here by throwing, and one
 * that over-refuses (the legacy 1e-8) fails the commit assertions.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import { segmentLength } from './lineInput'
import type { Ray } from '../viewport/math'

/** `kernel::tol::PLANE_DIST` / `kernel::tol::POINT_MERGE` (both 1e-9 m). */
const PLANE_DIST = 1e-9
const POINT_MERGE = 1e-9

/** Oblique modelling camera — the same eye as the inference staircase specs. */
const EYE: [number, number, number] = [5, -4, 3.5]

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

/**
 * A mock kernel scene enforcing the real `Sketch::add_segment` refusals:
 * `PointOffPlane` within `tol::PLANE_DIST` AND `DegenerateSegment` for
 * endpoints within `tol::POINT_MERGE` — the second is what makes an
 * under-refusing tool gate fail loudly here instead of being silently
 * accepted by a permissive mock.
 */
function makeWasmScene() {
  const planes = new Map<bigint, [number, number, number, number, number, number]>()
  const segmentCalls: SegmentCall[] = []
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
        if (Math.abs(distanceToPlane([ax, ay, az], plane)) > PLANE_DIST)
          throw new Error('PointOffPlane: point 0 is off the sketch plane')
        if (Math.abs(distanceToPlane([bx, by, bz], plane)) > PLANE_DIST)
          throw new Error('PointOffPlane: point 1 is off the sketch plane')
        if (Math.hypot(bx - ax, by - ay, bz - az) < POINT_MERGE)
          throw new Error('DegenerateSegment: segment endpoints coincide')
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
  return { scene: scene as unknown as WasmScene, segmentCalls }
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, onCommit, onToast, onFaceImprint }
}

/**
 * The locked-click defect's exact state: ground segment, blue-locked riser
 * (re-homes onto x = 0, anchor (0,1,1)), then the X lock for the third
 * segment — the lock direction normal to the frozen plane.
 */
function driveToLockedThirdSegment(tool: LineTool) {
  tool.onPointerDown(makeSnap([0, 0, 0], 'endpoint'), rayAt([0, 0, 0]))
  tool.onPointerMove(makeSnap([0, 1, 0], 'on-axis', { direction: [0, 1, 0] }), rayAt([0, 1, 0]))
  tool.onPointerDown(makeSnap([0, 1, 0], 'on-axis', { direction: [0, 1, 0] }), rayAt([0, 1, 0]))
  tool.onKey(makeKeyEvent('ArrowUp'))
  tool.onPointerMove(makeSnap([0, 1, 1], 'on-axis', { direction: [0, 0, 1] }), rayAt([0, 1, 1]))
  tool.onPointerDown(makeSnap([0, 1, 1], 'on-axis', { direction: [0, 0, 1] }), rayAt([0, 1, 1]))
  tool.onKey(makeKeyEvent('ArrowRight')) // switch the lock: blue -> red (X)
}

describe('LineTool — the degenerate-segment gate sits exactly on the kernel POINT_MERGE line', () => {
  // The band the legacy 1e-8 gate refused although both the kernel and the
  // inference engine accept it, plus points beyond it. Each must COMMIT: the
  // engine's lock-projection disqualification passes everything above
  // POINT_MERGE back to the tool as a real winner, and the kernel accepts
  // it, so the tool refusing any of these reopens the locked-click gap.
  const committable = [1.5e-9, 2e-9, 5e-9, 9.9e-9, 1.1e-8, 1e-7, 1e-3, 0.9]

  for (const d of committable) {
    it(`an X-locked click resolving ${d} m from the anchor commits (band probe)`, () => {
      const { scene, segmentCalls } = makeWasmScene()
      const { tool, onToast } = makeTool(scene)
      driveToLockedThirdSegment(tool)
      expect(segmentCalls).toHaveLength(2)

      const snap = makeSnap([d, 1, 1], 'on-axis', { direction: [1, 0, 0] })
      tool.onPointerMove(snap, rayAt([d, 1, 1]))
      tool.onPointerDown(snap, rayAt([d, 1, 1]))

      expect(onToast).not.toHaveBeenCalled()
      expect(segmentCalls).toHaveLength(3)
      expect(segmentCalls[2].a).toEqual([0, 1, 1])
      expect(segmentCalls[2].b).toEqual([d, 1, 1])
    })
  }

  it('AT exactly POINT_MERGE the tool refuses, matching the kernel inclusive boundary', () => {
    // The kernel's own test is `length_squared() <= tol * tol`, so a segment
    // of length EXACTLY POINT_MERGE is coincident to the kernel. A strict `<`
    // here would pass it through to be refused kernel-side under a different
    // message — agreeing on the magnitude but not the boundary is not
    // agreeing. This is the one value the band sweep above cannot reach.
    const { scene, segmentCalls } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)
    driveToLockedThirdSegment(tool)

    const at: [number, number, number] = [1e-9, 1, 1]
    expect(segmentLength([0, 1, 1], at)).toBe(1e-9) // bit-exact, not near
    const snap = makeSnap(at, 'on-axis', { direction: [1, 0, 0] })
    tool.onPointerMove(snap, rayAt(at))
    tool.onPointerDown(snap, rayAt(at))

    expect(segmentCalls).toHaveLength(2)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/same as the last/i)
  })

  it('below POINT_MERGE the honest refusal stands — the toast marks a segment the kernel itself would refuse', () => {
    const { scene, segmentCalls } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)
    driveToLockedThirdSegment(tool)

    const snap = makeSnap([5e-10, 1, 1], 'on-axis', { direction: [1, 0, 0] })
    tool.onPointerMove(snap, rayAt([5e-10, 1, 1]))
    tool.onPointerDown(snap, rayAt([5e-10, 1, 1]))

    expect(segmentCalls).toHaveLength(2)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/same as the last/i)
  })

  it('ground plane: the gate is the Euclidean kernel predicate, not the legacy per-axis one', () => {
    // All components below the legacy per-axis threshold, but Euclidean
    // length ~1.13e-9 > POINT_MERGE: the kernel accepts this segment, so
    // the tool must commit it (the old component-wise gate refused it).
    const { scene, segmentCalls } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap([2, 3, 0], 'ground'), rayAt([2, 3, 0]))
    const cursor: [number, number, number] = [2 + 8e-10, 3 + 8e-10, 0]
    tool.onPointerDown(makeSnap(cursor, 'ground'), rayAt(cursor))

    expect(onToast).not.toHaveBeenCalled()
    expect(segmentCalls).toHaveLength(1)
    expect(segmentCalls[0].a).toEqual([2, 3, 0])
    expect(segmentCalls[0].b).toEqual(cursor)
  })
})

describe('LineTool — face mode shares the same kernel-line gate', () => {
  function makeFaceWasmScene() {
    const pick = { object: () => 7n, face: () => 3n, instance: () => undefined, free: vi.fn() }
    const scene = {
      axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
      pick_face: vi.fn(() => pick),
      pick_sketch: vi.fn(() => undefined),
      node_parent: vi.fn(() => undefined),
      face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
      face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
      split_face: vi.fn(() => ({ free: vi.fn() })),
      split_face_in_instance: vi.fn(() => ({ free: vi.fn() })),
      clear_transient_segments: vi.fn(),
      add_transient_segment: vi.fn(),
    }
    return { scene: scene as unknown as WasmScene }
  }

  it('a second face click 5 nm away appends instead of toasting (band probe)', () => {
    const { scene } = makeFaceWasmScene()
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap([0, 0, 1], 'face'), rayAt([0, 0, 1]))
    expect(tool.capturingInput()).toBe(true)

    tool.onPointerDown(makeSnap([5e-9, 0, 1], 'face'), rayAt([5e-9, 0, 1]))
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a second face click below POINT_MERGE still toasts', () => {
    const { scene } = makeFaceWasmScene()
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap([0, 0, 1], 'face'), rayAt([0, 0, 1]))
    tool.onPointerDown(makeSnap([5e-10, 0, 1], 'face'), rayAt([5e-10, 0, 1]))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/same as the last/i)
  })
})
