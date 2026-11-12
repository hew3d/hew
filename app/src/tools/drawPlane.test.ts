import { describe, it, expect, vi } from 'vitest'
import {
  groundDrawPlane,
  planeFromSketch,
  axisDrawPlane,
  pointOnPlane,
  planeKey,
  isGroundPlane,
  drawPlaneThrough,
  DrawPlaneTracker,
  pinnedPlaneThrough,
  FaceLoopCache,
  idleHoverKind,
  planePip,
  pipArrowKey,
  PIP_AXIS_TOLERANCE_DEG,
  CUSTOM_CANDIDATE_MS,
  customPipPlaneFor,
  pinCustomPlane,
  SketchPickCache,
  rayLandsOnSketch,
  resolveIdleDrawTarget,
  resolveClickDrawTarget,
  inferredAxis,
  inferredAxisPlane,
  GROUND_PREFERENCE,
  groundNaturalTarget,
  nextGestureLockPlane,
  type DrawPlane,
  type CameraView,
} from './drawPlane'
import { WORLD_DRAWING_AXES } from './drawingAxes'
import { PlanePin } from './planePin'
import type { Scene as WasmScene } from '../wasm/loader'
import { setPickRenderMode } from './pickPolicy'
import type { Ray } from '../viewport/math'
import type { EditContext } from './types'
import type { V3 } from '../viewport/geoHelpers'
import type { DrawingAxes } from './drawingAxes'

const TOP: EditContext = { kind: 'top' }

/** World-identity drawing axes (tool-parity §4) — the default frame every
 *  test here exercises unless it explicitly overrides `frame`. */
const WORLD_FRAME_FLAT = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]

function makeScene(
  sketchPlanes: Map<bigint, Float64Array | undefined>,
  frame: number[] = WORLD_FRAME_FLAT,
): WasmScene {
  return {
    sketch_plane: vi.fn((h: bigint) => sketchPlanes.get(h)),
    pick_sketch: vi.fn(),
    axes: vi.fn(() => new Float64Array(frame)),
  } as unknown as WasmScene
}

describe('groundDrawPlane', () => {
  it('returns the exact literal ground plane', () => {
    expect(groundDrawPlane()).toEqual({
      origin: [0, 0, 0],
      normal: [0, 0, 1],
      u: [1, 0, 0],
      v: [0, 1, 0],
      ground: true,
    })
  })
})

describe('isGroundPlane', () => {
  it('true for the origin/+Z ground plane', () => {
    expect(isGroundPlane([0, 0, 0], [0, 0, 1])).toBe(true)
  })

  it('true for a flipped-but-coincident (-Z through the origin) plane — orientation-free', () => {
    expect(isGroundPlane([0, 0, 0], [0, 0, -1])).toBe(true)
  })

  it('true for any in-plane point, not just the literal origin', () => {
    expect(isGroundPlane([5, -3, 0], [0, 0, 1])).toBe(true)
  })

  it('false for a plane tilted off Z', () => {
    expect(isGroundPlane([0, 0, 0], [0, -1, 0])).toBe(false)
  })

  it('false for a plane parallel to ground but offset in Z', () => {
    expect(isGroundPlane([0, 0, 2], [0, 0, 1])).toBe(false)
  })
})

describe('planeFromSketch', () => {
  it('returns null for a stale/hidden sketch (sketch_plane undefined)', () => {
    const scene = makeScene(new Map([[7n, undefined]]))
    expect(planeFromSketch(scene, 7n)).toBeNull()
  })

  it('returns the EXACT groundDrawPlane() when the sketch plane is the ground plane', () => {
    const scene = makeScene(new Map([[7n, new Float64Array([0, 0, 0, 0, 0, 1])]]))
    expect(planeFromSketch(scene, 7n)).toEqual(groundDrawPlane())
  })

  it('returns the EXACT groundDrawPlane() for a flipped-but-coincident ground sketch plane', () => {
    const scene = makeScene(new Map([[7n, new Float64Array([0, 0, 0, 0, 0, -1])]]))
    expect(planeFromSketch(scene, 7n)).toEqual(groundDrawPlane())
  })

  it('returns a non-ground plane with a facePlaneBasis-derived basis, ground: false', () => {
    // A sketch stood upright: y = 0 plane, -Y normal (rotated 90 deg about X).
    const scene = makeScene(new Map([[7n, new Float64Array([0, 0, 0, 0, -1, 0])]]))
    const plane = planeFromSketch(scene, 7n)
    expect(plane).not.toBeNull()
    expect(plane!.ground).toBe(false)
    expect(plane!.origin).toEqual([0, 0, 0])
    expect(plane!.normal).toEqual([0, -1, 0])
    // u, v, normal orthonormal and right-handed.
    const { u, v, normal } = plane!
    expect(Math.hypot(...u)).toBeCloseTo(1, 9)
    expect(Math.hypot(...v)).toBeCloseTo(1, 9)
    const cross: [number, number, number] = [
      u[1] * v[2] - u[2] * v[1],
      u[2] * v[0] - u[0] * v[2],
      u[0] * v[1] - u[1] * v[0],
    ]
    expect(cross[0]).toBeCloseTo(normal[0], 9)
    expect(cross[1]).toBeCloseTo(normal[1], 9)
    expect(cross[2]).toBeCloseTo(normal[2], 9)
  })

  describe('instance pose mapping (component-edit-parity.md phase A2)', () => {
    function instanceScene(plane: Float64Array, pose: Float64Array | undefined): WasmScene {
      return {
        sketch_plane: vi.fn(() => plane),
        instance_pose: vi.fn(() => pose),
      } as unknown as WasmScene
    }

    it('with no instance, returns the raw local plane unchanged (unchanged behavior)', () => {
      const scene = instanceScene(new Float64Array([0, 0, 0, 0, -1, 0]), undefined)
      const plane = planeFromSketch(scene, 7n)
      expect(plane!.origin).toEqual([0, 0, 0])
      expect(plane!.normal).toEqual([0, -1, 0])
    })

    it('with an instance, poses the LOCAL plane forward into WORLD space (translation)', () => {
      const pose = new Float64Array([1, 0, 0, 10, 0, 1, 0, 20, 0, 0, 1, 5])
      const scene = instanceScene(new Float64Array([0, 0, 0, 0, -1, 0]), pose)
      const plane = planeFromSketch(scene, 7n, 42n)
      expect(plane).not.toBeNull()
      expect(plane!.ground).toBe(false)
      expect(plane!.origin).toEqual([10, 20, 5])
      expect(plane!.normal).toEqual([0, -1, 0]) // translation doesn't rotate the normal
    })

    it('with an instance, poses the normal through the pose too (rotation)', () => {
      // 90 deg about Z: local +X normal becomes world +Y.
      const pose = new Float64Array([0, -1, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0])
      const scene = instanceScene(new Float64Array([0, 0, 0, 1, 0, 0]), pose)
      const plane = planeFromSketch(scene, 7n, 42n)
      expect(plane).not.toBeNull()
      expect(plane!.normal[0]).toBeCloseTo(0, 9)
      expect(plane!.normal[1]).toBeCloseTo(1, 9)
      expect(plane!.normal[2]).toBeCloseTo(0, 9)
    })

    it('returns null (never falls back to the raw local plane) for a stale/unknown instance', () => {
      const scene = instanceScene(new Float64Array([0, 0, 0, 0, -1, 0]), undefined)
      expect(planeFromSketch(scene, 7n, 999n)).toBeNull()
    })

    it('a non-uniformly scaled pose keeps the posed plane on the posed sketch (normal through the inverse-transpose)', () => {
      // Local plane through the origin with normal (1, 0, 1)/√2, instance
      // stretched 2× along X. The local in-plane tangent (1, 0, -1) maps to
      // (2, 0, -1) in world: the posed plane must hold it. The plain linear
      // part would give (2, 0, 1)/√5, tilted off the sketch.
      const h = Math.SQRT1_2
      const pose = new Float64Array([2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])
      const scene = instanceScene(new Float64Array([0, 0, 0, h, 0, h]), pose)
      const plane = planeFromSketch(scene, 7n, 42n)
      expect(plane).not.toBeNull()
      const n = plane!.normal
      expect(Math.hypot(...n)).toBeCloseTo(1, 12)
      expect(n[0] * 2 + n[1] * 0 + n[2] * -1).toBeCloseTo(0, 12)
      // The untouched in-plane direction (0, 1, 0) stays in the plane too.
      expect(n[1]).toBeCloseTo(0, 12)
    })
  })
})

describe('axisDrawPlane — the `ground` flag is a licence to flatten', () => {
  // Every draw tool has a ground fast path that returns `[snap.x, snap.y, 0]`,
  // discarding the snapped point's z outright. That is only safe because
  // `ground: true` is supposed to mean "this plane IS the world z=0 plane", so
  // the z being discarded is always already 0.
  //
  // Nothing in the type system ties those together. If a lock ever reported
  // `ground: true` for a plane that is not z=0 — a moved drawing frame whose
  // "ground-like" plane sits at a height, say — every draw tool would silently
  // snap points down to the world ground with no other symptom. That is the
  // shape of the defect the Line tool actually hit, where an axis lock could
  // take a segment off its plane and the fast path flattened it away.
  //
  // This sweep pins the coupling directly: whatever `axisDrawPlane` returns,
  // `ground: true` must imply that flattening to z=0 is lossless.
  it('never reports ground: true for a plane where discarding z would lose information', () => {
    const movedFrame = {
      origin: [3, -4, 2] as [number, number, number],
      x: [0, 1, 0] as [number, number, number],
      y: [-1, 0, 0] as [number, number, number],
      z: [0, 0, 1] as [number, number, number],
    }
    const throughs: Array<[number, number, number]> = [
      [0, 0, 0], [3, -2, 0], [-7, 11, 0],
      [0, 0, 5], [3, -2, 0.5], [1, 1, -2], [0, 0, 1e-9],
    ]
    const axes: Array<0 | 1 | 2> = [0, 1, 2]
    let groundCount = 0
    for (const frame of [undefined, movedFrame]) {
      for (const axis of axes) {
        for (const through of throughs) {
          const plane = frame === undefined
            ? axisDrawPlane(axis, through)
            : axisDrawPlane(axis, through, frame)
          if (!plane.ground) continue
          groundCount += 1
          // The two conditions that make `[x, y, 0]` a faithful projection.
          expect(plane.normal).toEqual([0, 0, 1])
          expect(plane.origin[2]).toBe(0)
        }
      }
    }
    // Guard the guard: if a refactor stopped producing ground planes here at
    // all, every assertion above would pass vacuously.
    expect(groundCount).toBeGreaterThan(0)
  })

  it('a moved drawing frame never yields a ground plane, however ground-like it looks', () => {
    // A frame lifted in z has a horizontal plane through its own origin, which
    // is emphatically NOT the world ground — flattening a point onto it would
    // move the point.
    const lifted = {
      origin: [0, 0, 4] as [number, number, number],
      x: [1, 0, 0] as [number, number, number],
      y: [0, 1, 0] as [number, number, number],
      z: [0, 0, 1] as [number, number, number],
    }
    expect(axisDrawPlane(2, [0, 0, 4], lifted).ground).toBe(false)
  })
})

describe('axisDrawPlane', () => {
  it('axis 2 (Z) through a z=0 point returns the exact ground frame, ground: true', () => {
    const plane = axisDrawPlane(2, [3, -2, 0])
    expect(plane.origin).toEqual([3, -2, 0])
    expect(plane.normal).toEqual([0, 0, 1])
    expect(plane.u).toEqual([1, 0, 0])
    expect(plane.v).toEqual([0, 1, 0])
    expect(plane.ground).toBe(true)
  })

  it('axis 2 (Z) through a point off z=0 is a non-ground plane', () => {
    const plane = axisDrawPlane(2, [0, 0, 5])
    expect(plane.ground).toBe(false)
    expect(plane.normal).toEqual([0, 0, 1])
  })

  it('axis 0 (X) is a vertical plane through the given point, ground: false', () => {
    const plane = axisDrawPlane(0, [1, 2, 3])
    expect(plane.origin).toEqual([1, 2, 3])
    expect(plane.normal).toEqual([1, 0, 0])
    expect(plane.ground).toBe(false)
  })

  it('axis 1 (Y) is a vertical plane through the given point, ground: false', () => {
    const plane = axisDrawPlane(1, [1, 2, 3])
    expect(plane.normal).toEqual([0, 1, 0])
    expect(plane.ground).toBe(false)
  })

  // ── Movable drawing axes (tool-parity §4): a non-identity frame ──
  const ROTATED_FRAME: DrawingAxes = {
    origin: [5, 0, 0],
    x: [0, 1, 0],
    y: [-1, 0, 0],
    z: [0, 0, 1],
  }

  /** A valid right-handed in-plane basis: unit u/v, mutually perpendicular,
   *  cross(u, v) matching `normal` (within float tolerance). */
  function expectValidBasis(plane: DrawPlane): void {
    const dot = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    const len = (v: V3): number => Math.sqrt(dot(v, v))
    const cross = (a: V3, b: V3): V3 => [
      a[1] * b[2] - a[2] * b[1],
      a[2] * b[0] - a[0] * b[2],
      a[0] * b[1] - a[1] * b[0],
    ]
    expect(len(plane.u)).toBeCloseTo(1, 9)
    expect(len(plane.v)).toBeCloseTo(1, 9)
    expect(dot(plane.u, plane.v)).toBeCloseTo(0, 9)
    const c = cross(plane.u, plane.v)
    expect(c[0]).toBeCloseTo(plane.normal[0], 9)
    expect(c[1]).toBeCloseTo(plane.normal[1], 9)
    expect(c[2]).toBeCloseTo(plane.normal[2], 9)
  }

  it('axis 0 (red) under a rotated frame is a plane along the frame\'s X, never ground', () => {
    const plane = axisDrawPlane(0, [1, 2, 3], ROTATED_FRAME)
    expect(plane.normal).toEqual(ROTATED_FRAME.x)
    expect(plane.ground).toBe(false)
    expectValidBasis(plane)
  })

  it('axis 2 (blue) under a rotated frame through a z=0 point is STILL not the ground fast path', () => {
    // The frame's blue axis happens to be world +Z here, and `through` sits
    // at z=0 — but the frame itself is not world identity, so this must NOT
    // take the ground fast path (SketchUp's Axes tool doesn't re-grid the
    // world; a moved frame's ground-like plane is still just a plane).
    const plane = axisDrawPlane(2, [1, 2, 0], ROTATED_FRAME)
    expect(plane.ground).toBe(false)
    expect(plane.normal).toEqual(ROTATED_FRAME.z)
    expectValidBasis(plane)
  })

  it('axis 1 (green) under a rotated frame is a plane along the frame\'s Y', () => {
    const plane = axisDrawPlane(1, [0, 0, 0], ROTATED_FRAME)
    expect(plane.normal).toEqual(ROTATED_FRAME.y)
    expect(plane.ground).toBe(false)
    expectValidBasis(plane)
  })
})

describe('pointOnPlane', () => {
  it('intersects a straight-down ray with the ground plane', () => {
    const ray: Ray = { origin: [1, 2, 5], direction: [0, 0, -1] }
    const p = pointOnPlane(ray, groundDrawPlane())
    expect(p).toEqual([1, 2, 0])
  })

  it('returns null for a ray parallel to the plane', () => {
    const ray: Ray = { origin: [0, 0, 5], direction: [1, 0, 0] }
    const p = pointOnPlane(ray, groundDrawPlane())
    expect(p).toBeNull()
  })
})

describe('planeKey', () => {
  it('the ground plane and a coplanar-but-flipped-normal plane produce the same key', () => {
    const a = groundDrawPlane()
    const b: DrawPlane = { origin: [0, 0, 0], normal: [0, 0, -1], u: [1, 0, 0], v: [0, -1, 0], ground: false }
    expect(planeKey(a)).toBe(planeKey(b))
  })

  it('the ground plane and a plane through a DIFFERENT z=0 point produce the same key', () => {
    const a = groundDrawPlane()
    const b: DrawPlane = { origin: [7, -3, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0], ground: false }
    expect(planeKey(a)).toBe(planeKey(b))
  })

  it('a plane offset in Z from the ground plane produces a DIFFERENT key', () => {
    const a = groundDrawPlane()
    const b: DrawPlane = { origin: [0, 0, 2], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0], ground: false }
    expect(planeKey(a)).not.toBe(planeKey(b))
  })

  it('a tilted plane and its flipped-normal twin produce the same key', () => {
    const a: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }
    const b: DrawPlane = { origin: [0, 0, 0], normal: [0, 1, 0], u: [1, 0, 0], v: [0, 0, 1], ground: false }
    expect(planeKey(a)).toBe(planeKey(b))
  })

  it('two distinct tilted planes produce different keys', () => {
    const a: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }
    const b: DrawPlane = { origin: [0, 0, 0], normal: [1, 0, 0], u: [0, 1, 0], v: [0, 0, -1], ground: false }
    expect(planeKey(a)).not.toBe(planeKey(b))
  })
})

describe('DrawPlaneTracker.state — the plane, its hold, and the hover (view-die §3.4)', () => {
  const TILTED: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }
  const DOWN: Ray = { origin: [3, -2, 5], direction: [0, 0, -1] }
  const base = { anchored: null, lock: null, pinned: null, lockHover: null, fallback: groundDrawPlane(), frame: WORLD_DRAWING_AXES } as const

  it('before any hover: the fallback plane, inferred, no hover', () => {
    const fallback = axisDrawPlane(1, [0, 0, 0])
    const state = new DrawPlaneTracker().state({ ...base, fallback })
    expect(state).toEqual({ plane: fallback, pip: 'green', hold: 'inferred', hover: null })
  })

  it('anchored on the ground: the ground, held as anchored, the hover at the cursor ray\'s hit', () => {
    const t = new DrawPlaneTracker()
    t.move(DOWN)
    const state = t.state({ ...base, anchored: groundDrawPlane() })
    expect(state.plane.ground).toBe(true)
    expect(state.pip).toBe('blue')
    expect(state.hold).toBe('anchored')
    expect(state.hover).toEqual({ kind: 'space', point: [3, -2, 0] })
  })

  it('anchored on a non-ground plane: that plane, and the hover follows the cursor rather than the anchor', () => {
    const t = new DrawPlaneTracker()
    const ray: Ray = { origin: [2, -5, 1], direction: [0, 1, 0] }
    t.move(ray)
    const state = t.state({ ...base, anchored: TILTED })
    expect(state.plane).toBe(TILTED)
    expect(state.pip).toBe('green')
    expect(state.hold).toBe('anchored')
    expect(state.hover).toEqual({ kind: 'space', point: [2, 0, 1] })
  })

  it('a lock or pin outranks the anchored hold, on the gesture\'s own plane', () => {
    const t = new DrawPlaneTracker()
    expect(t.state({ ...base, anchored: TILTED, lock: 1 }).hold).toBe('locked')
    expect(t.state({ ...base, anchored: TILTED, lock: 1 }).plane).toBe(TILTED)
    expect(t.state({ ...base, anchored: TILTED, pinned: TILTED }).hold).toBe('pinned')
  })

  it('locked with no hover yet: the locked axis plane through the frame origin, no hover', () => {
    const state = new DrawPlaneTracker().state({ ...base, lock: 0 })
    expect(state.plane).toEqual(axisDrawPlane(0, [0, 0, 0]))
    expect(state.pip).toBe('red')
    expect(state.hold).toBe('locked')
    expect(state.hover).toBeNull()
  })

  it('locked with a hover: the locked axis plane through the tracked hover point, hovered there', () => {
    const t = new DrawPlaneTracker()
    t.move(DOWN)
    const state = t.state({ ...base, lock: 0, lockHover: [4, 5, 6] })
    expect(state.plane).toEqual(axisDrawPlane(0, [4, 5, 6]))
    expect(state.hover).toEqual({ kind: 'space', point: [4, 5, 6] })
  })

  it('a blue lock through a z = 0 hover is the ground itself, still held', () => {
    const t = new DrawPlaneTracker()
    t.move(DOWN)
    const state = t.state({ ...base, lock: 2, lockHover: [1, 2, 0] })
    expect(state.plane.ground).toBe(true)
    expect(state.pip).toBe('blue')
    expect(state.hold).toBe('locked')
  })

  it('pinned: the pinned plane, the hover at the cursor ray\'s hit on it', () => {
    const t = new DrawPlaneTracker()
    t.move({ origin: [1, -4, 2], direction: [0, 1, 0] })
    const state = t.state({ ...base, pinned: TILTED, lockHover: [9, 9, 9] })
    expect(state.plane).toBe(TILTED)
    expect(state.hold).toBe('pinned')
    expect(state.hover?.point).toEqual([1, 0, 2])
  })

  it('a ground pin through a named point in the air reports the lifted plane the click will use', () => {
    const pinned = pinnedPlaneThrough(groundDrawPlane(), [1, 1, 2])
    expect(pinned.origin).toEqual([1, 1, 2])
    expect(pinned.normal).toEqual([0, 0, 1])
    expect(pinned.ground).toBe(false)
    const t = new DrawPlaneTracker()
    t.move(DOWN)
    const state = t.state({ ...base, pinned, lockHover: [1, 1, 2] })
    expect(state.plane).toBe(pinned)
    expect(state.pip).toBe('blue')
    expect(state.hold).toBe('pinned')
    expect(state.hover).toEqual({ kind: 'space', point: [3, -2, 2] })
  })

  it('a ground pin through a point back on the ground stays the ground', () => {
    const ground = groundDrawPlane()
    expect(pinnedPlaneThrough(ground, [1, 1, 0])).toBe(ground)
    expect(pinnedPlaneThrough(ground, null)).toBe(ground)
  })

  // ── Movable drawing axes (tool-parity §4): the lock and the pip follow
  // the CURRENT frame, so the state names the plane a click would land on.
  const ROTATED_FRAME: DrawingAxes = {
    origin: [5, 0, 0],
    x: [0, 1, 0],
    y: [-1, 0, 0],
    z: [0, 0, 1],
  }

  it('under a moved frame, the locked plane equals what a click at the same point and lock resolves onto, and lights red', () => {
    const t = new DrawPlaneTracker()
    t.move(DOWN)
    const state = t.state({ ...base, lock: 0, lockHover: [4, 5, 6], frame: ROTATED_FRAME })
    expect(state.plane).toEqual(axisDrawPlane(0, [4, 5, 6], ROTATED_FRAME))
    // The moved frame's red axis is world +Y.
    expect(state.plane.normal).toEqual([0, 1, 0])
    expect(state.pip).toBe('red')
  })

  it('a moved frame\'s blue lock through z = 0 is not the literal ground', () => {
    const t = new DrawPlaneTracker()
    t.move(DOWN)
    const state = t.state({ ...base, lock: 2, lockHover: [1, 2, 0], frame: ROTATED_FRAME })
    expect(state.plane.ground).toBe(false)
    expect(state.pip).toBe('blue')
  })
})

describe('planePip — which pip a plane lights (view-die §3.1)', () => {
  const tilt = (deg: number): V3 => {
    const r = (deg * Math.PI) / 180
    return [Math.sin(r), 0, Math.cos(r)]
  }

  it('the three axis planes light red, green and blue at any offset and either direction', () => {
    expect(planePip(axisDrawPlane(0, [7, -3, 2]).normal)).toBe('red')
    expect(planePip([-1, 0, 0])).toBe('red')
    expect(planePip(axisDrawPlane(1, [0, 4, 0]).normal)).toBe('green')
    expect(planePip([0, -1, 0])).toBe('green')
    expect(planePip(groundDrawPlane().normal)).toBe('blue')
    expect(planePip(axisDrawPlane(2, [0, 0, 9]).normal)).toBe('blue')
    expect(planePip([0, 0, -1])).toBe('blue')
  })

  it('a Shift pin on a cube\'s front face lights green whatever its offset', () => {
    expect(planePip(drawPlaneThrough([0.3, -2, 0.7], [0, -1, 0])!.normal)).toBe('green')
  })

  it('the tolerance is 0.5° each way', () => {
    expect(PIP_AXIS_TOLERANCE_DEG).toBe(0.5)
    expect(planePip(tilt(0.45))).toBe('blue')
    expect(planePip(tilt(-0.45))).toBe('blue')
    expect(planePip(tilt(0.6))).toBe('custom')
  })

  it('a plane rotated 30° is custom', () => {
    const r = (30 * Math.PI) / 180
    // A cube's side face after a 30° turn about the blue axis.
    expect(planePip([Math.cos(r), Math.sin(r), 0])).toBe('custom')
  })

  it('follows moved drawing axes: the frame\'s own X lights red, world X then lights green', () => {
    const r = (30 * Math.PI) / 180
    const frame: DrawingAxes = { origin: [1, 2, 0], x: [Math.cos(r), Math.sin(r), 0], y: [-Math.sin(r), Math.cos(r), 0], z: [0, 0, 1] }
    expect(planePip([Math.cos(r), Math.sin(r), 0], frame)).toBe('red')
    expect(planePip([-Math.sin(r), Math.cos(r), 0], frame)).toBe('green')
    expect(planePip([1, 0, 0], frame)).toBe('custom')
    expect(planePip([0, 0, 1], frame)).toBe('blue')
  })

  it('a zero or non-finite normal is custom', () => {
    expect(planePip([0, 0, 0])).toBe('custom')
    expect(planePip([Number.NaN, 0, 1])).toBe('custom')
  })

  it('red, green and blue stand for →, ← and ↑; custom for no arrow', () => {
    expect(pipArrowKey('red')).toBe('ArrowRight')
    expect(pipArrowKey('green')).toBe('ArrowLeft')
    expect(pipArrowKey('blue')).toBe('ArrowUp')
    expect(pipArrowKey('custom')).toBeNull()
  })
})

describe('SketchPickCache', () => {
  const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

  it('memoizes pick_sketch per Ray reference', () => {
    const scene = { pick_sketch: vi.fn(() => 9n) } as unknown as WasmScene
    const cache = new SketchPickCache()

    expect(cache.pickFor(scene, RAY)).toBe(9n)
    expect(cache.pickFor(scene, RAY)).toBe(9n)
    expect(scene.pick_sketch).toHaveBeenCalledTimes(1)

    cache.pickFor(scene, { ...RAY })
    expect(scene.pick_sketch).toHaveBeenCalledTimes(2)
  })

  it('caches a miss (undefined) as null', () => {
    const scene = { pick_sketch: vi.fn(() => undefined) } as unknown as WasmScene
    const cache = new SketchPickCache()
    expect(cache.pickFor(scene, RAY)).toBeNull()
    expect(cache.pickFor(scene, RAY)).toBeNull()
    expect(scene.pick_sketch).toHaveBeenCalledTimes(1)
  })

  it('a render-mode switch invalidates the memo for the same Ray', () => {
    const scene = { pick_sketch: vi.fn(() => 9n) } as unknown as WasmScene
    const cache = new SketchPickCache()
    const target = { plane: { origin: [0, 0, 0], normal: [0, 0, 1] }, target: { kind: 'new' } } as unknown as Parameters<SketchPickCache['targetFor']>[1] extends () => infer R ? R : never
    const compute = vi.fn(() => target)
    try {
      cache.pickFor(scene, RAY)
      cache.targetFor(RAY, compute)
      setPickRenderMode('wireframe')
      cache.pickFor(scene, RAY)
      cache.targetFor(RAY, compute)
      expect(scene.pick_sketch).toHaveBeenCalledTimes(2)
      expect(compute).toHaveBeenCalledTimes(2)
    } finally {
      setPickRenderMode('shaded')
    }
  })
})

// ------------------------------------------------- sketch-mode adoption gate

/**
 * The regression this gate exists for, in numbers. A vertical sketch (y = 0)
 * seen from a 3/4 camera at (8, 6, 8) is met at ~28° incidence, so
 * `pick_sketch`'s cone — which measures the PERPENDICULAR distance from the
 * ray axis — reports a 0.245 m miss for an edge that is 0.52 m away IN THE
 * PLANE. The cone's half-angle is 0.02 rad, so the pick "hits" (0.245 / 12.8
 * = 0.0192 rad) even though the user is pointing half a metre off the sketch,
 * at the world origin on the ground.
 */
describe('rayLandsOnSketch', () => {
  const WALL_SKETCH = 7n
  /** The y = 0 wall plane, +Y normal. */
  const WALL: DrawPlane = { origin: [0, 0, 0], normal: [0, 1, 0], u: [0, 0, 1], v: [1, 0, 0], ground: false }
  /** A 0.6 m square standing on the wall plane, spanning x ∈ [-0.3, 0.3], z ∈ [0.4, 1]. */
  const SQUARE = new Float32Array([
    -0.3, 0, 0.4, 0.3, 0, 0.4,
    0.3, 0, 0.4, 0.3, 0, 1,
    0.3, 0, 1, -0.3, 0, 1,
    -0.3, 0, 1, -0.3, 0, 0.4,
  ])

  function wallScene(lines: Float32Array | (() => never) = SQUARE): WasmScene {
    return {
      sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 1, 0])),
      pick_sketch: vi.fn(() => WALL_SKETCH),
      sketch_lines: vi.fn(() => (typeof lines === 'function' ? lines() : lines)),
    } as unknown as WasmScene
  }

  /** A ray from the 3/4 camera through `target`. */
  function rayThrough(target: [number, number, number]): Ray {
    const o: [number, number, number] = [8, 6, 8]
    return { origin: o, direction: [target[0] - o[0], target[1] - o[1], target[2] - o[2]] }
  }

  it('rejects the grazing near-miss: aiming at the world origin does NOT land on the standing square', () => {
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, rayThrough([0, 0, 0]))).toBe(false)
  })

  it('accepts a ray aimed at the square\'s own edge', () => {
    // Distance is measured to the sketch's EDGES, matching what
    // `pick_sketch`'s cone tests — a ray through the middle of a closed
    // loop is never a pick_sketch hit in the first place, so the gate never
    // sees it.
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, rayThrough([0, 0, 0.4]))).toBe(true)
  })

  it('accepts a ray just outside an edge but within the cone radius at that depth', () => {
    // Pierce point (0, 0, 0.35) — 0.05 m below the bottom edge. The cone
    // radius at ~12.9 m is 0.02 × 12.9 ≈ 0.26 m, so this still lands.
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, rayThrough([0, 0, 0.35]))).toBe(true)
  })

  it('rejects a ray PARALLEL to the plane — it never pierces it at all', () => {
    const parallel: Ray = { origin: [5, 0, 0.4], direction: [-1, 0, 0] }
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, parallel)).toBe(false)
  })

  it('rejects a sketch whose handle went stale (sketch_lines throws)', () => {
    const stale = wallScene(() => { throw new Error('UnknownSketch') })
    expect(rayLandsOnSketch(stale, WALL_SKETCH, WALL, rayThrough([0, 0, 0.4]))).toBe(false)
  })

  it('rejects an empty sketch — no geometry to land on', () => {
    expect(rayLandsOnSketch(wallScene(new Float32Array([])), WALL_SKETCH, WALL, rayThrough([0, 0, 0.4]))).toBe(false)
  })

  describe('instance pose mapping (component-edit-parity.md phase A2)', () => {
    // The SAME square, but `sketch_lines` answers in LOCAL (definition)
    // space; `plane` (the WORLD-mapped plane the caller already computed via
    // `planeFromSketch(..., instance)`) is posed by a translation of
    // (10, 20, 0) — so the WORLD wall plane/square sit at that offset.
    const POSED_WALL: DrawPlane = {
      origin: [10, 20, 0],
      normal: [0, 1, 0],
      u: [0, 0, 1],
      v: [1, 0, 0],
      ground: false,
    }
    const POSE = new Float64Array([1, 0, 0, 10, 0, 1, 0, 20, 0, 0, 1, 0])

    // No default: an explicit `undefined` must mean a genuinely stale
    // instance, not silently substitute POSE (a default parameter would
    // treat an explicit `undefined` argument the same as an omitted one).
    function posedScene(pose: Float64Array | undefined): WasmScene {
      return {
        sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 1, 0])),
        pick_sketch: vi.fn(() => WALL_SKETCH),
        sketch_lines: vi.fn(() => SQUARE), // LOCAL coordinates, unmapped
        instance_pose: vi.fn(() => pose),
      } as unknown as WasmScene
    }

    function rayThroughWorld(target: [number, number, number]): Ray {
      const o: [number, number, number] = [8 + 10, 6 + 20, 8]
      return { origin: o, direction: [target[0] - o[0], target[1] - o[1], target[2] - o[2]] }
    }

    it('maps each LOCAL sketch_lines segment through the pose before measuring against the WORLD hit point', () => {
      // The square's edge at its posed WORLD position: local (0,0,0.4) + (10,20,0).
      expect(
        rayLandsOnSketch(posedScene(POSE), WALL_SKETCH, POSED_WALL, rayThroughWorld([10, 20, 0.4]), 42n),
      ).toBe(true)
    })

    it('a ray at the sketch\'s stale LOCAL (unmapped) position misses — proof the pose is genuinely applied', () => {
      expect(
        rayLandsOnSketch(posedScene(POSE), WALL_SKETCH, POSED_WALL, rayThroughWorld([0, 0, 0.4]), 42n),
      ).toBe(false)
    })

    it('a stale/unknown instance is a miss, never a throw or a silent unmapped fallback', () => {
      expect(
        rayLandsOnSketch(posedScene(undefined), WALL_SKETCH, POSED_WALL, rayThroughWorld([10, 20, 0.4]), 42n),
      ).toBe(false)
    })
  })
})

describe('resolveIdleDrawTarget', () => {
  const WALL_SKETCH = 7n
  const SQUARE = new Float32Array([
    -0.3, 0, 0.4, 0.3, 0, 0.4,
    0.3, 0, 0.4, 0.3, 0, 1,
    0.3, 0, 1, -0.3, 0, 1,
    -0.3, 0, 1, -0.3, 0, 0.4,
  ])

  /** `pick: null` means `pick_sketch` MISSES (returns undefined). */
  function scene(plane: Float64Array, pick: bigint | null = WALL_SKETCH): WasmScene {
    return {
      sketch_plane: vi.fn(() => plane),
      pick_sketch: vi.fn(() => pick ?? undefined),
      pick_sketch_region: vi.fn(() => undefined),
      pick_sketch_region_in_instance: vi.fn(() => undefined),
      sketch_lines: vi.fn(() => SQUARE),
    } as unknown as WasmScene
  }

  function rayThrough(target: [number, number, number]): Ray {
    const o: [number, number, number] = [8, 6, 8]
    return { origin: o, direction: [target[0] - o[0], target[1] - o[1], target[2] - o[2]] }
  }

  const WALL_PLANE_ARR = new Float64Array([0, 0, 0, 0, 1, 0])

  it('adopts the hovered sketch when the ray lands on it', () => {
    const resolved = resolveIdleDrawTarget(scene(WALL_PLANE_ARR), new SketchPickCache(), rayThrough([0, 0, 0.4]), TOP)
    expect(resolved.target).toEqual({ kind: 'existing', handle: WALL_SKETCH, instance: null })
    expect(resolved.plane.ground).toBe(false)
  })

  it('never adopts a DETACHED shape: a plain shape drawn over it lands on the plane beneath (separate-shapes D3)', () => {
    const sc = scene(WALL_PLANE_ARR) as unknown as Record<string, unknown>
    sc.history_generation = vi.fn(() => 1n)
    sc.plane_units_all_json = vi.fn(() => JSON.stringify([
      { plane: [0, 0, 0, 0, 1, 0], sketch: Number(WALL_SKETCH), detached: true, units: { units: [], regions: [] } },
    ]))
    const resolved = resolveIdleDrawTarget(sc as unknown as WasmScene, new SketchPickCache(), rayThrough([0, 0, 0.4]), TOP)
    expect(resolved.target.kind).toBe('plane')
  })

  it('falls back to the GROUND plane on a grazing pick the ray does not land on', () => {
    const resolved = resolveIdleDrawTarget(scene(WALL_PLANE_ARR), new SketchPickCache(), rayThrough([0, 0, 0]), TOP)
    expect(resolved.plane).toEqual(groundDrawPlane())
    expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane(), instance: null })
  })

  it('falls back to the ground plane when nothing is picked', () => {
    const resolved = resolveIdleDrawTarget(
      scene(WALL_PLANE_ARR, null), new SketchPickCache(), rayThrough([0, 0, 0.4]), TOP,
    )
    expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane(), instance: null })
  })

  it('a sketch on the ground plane gives the ground plane-mode target, never sketch mode', () => {
    const groundArr = new Float64Array([0, 0, 0, 0, 0, 1])
    const resolved = resolveIdleDrawTarget(scene(groundArr), new SketchPickCache(), rayThrough([0, 0, 0]), TOP)
    expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane(), instance: null })
  })

  describe('inside an instance editing context (component-edit-parity.md phase A2)', () => {
    const INSTANCE = 42n
    const COMPONENT = 5n
    const INSTANCE_CTX: EditContext = { kind: 'instance', id: INSTANCE, component: COMPONENT }

    const IDENTITY_POSE = new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])

    function instanceScene(
      plane: Float64Array,
      pick: bigint | null,
      memberSketches: bigint[],
      pose: Float64Array = IDENTITY_POSE,
    ): WasmScene {
      return {
        sketch_plane: vi.fn(() => plane),
        pick_sketch: vi.fn(() => pick ?? undefined),
        pick_sketch_region: vi.fn(() => undefined),
        pick_sketch_region_in_instance: vi.fn(() => undefined),
        sketch_lines: vi.fn(() => SQUARE),
        component_member_sketches: vi.fn(() => new BigUint64Array(memberSketches)),
        instance_pose: vi.fn(() => pose),
      } as unknown as WasmScene
    }

    it('adopts a hovered sketch that IS a member of the entered definition', () => {
      const resolved = resolveIdleDrawTarget(
        instanceScene(WALL_PLANE_ARR, WALL_SKETCH, [WALL_SKETCH]),
        new SketchPickCache(),
        rayThrough([0, 0, 0.4]),
        INSTANCE_CTX,
      )
      expect(resolved.target).toEqual({ kind: 'existing', handle: WALL_SKETCH, instance: INSTANCE })
    })

    it('never adopts a DETACHED definition-member sketch (the kernel answers per handle; the world listing omits members)', () => {
      const sc = instanceScene(WALL_PLANE_ARR, WALL_SKETCH, [WALL_SKETCH]) as unknown as Record<string, unknown>
      sc.sketch_is_detached = vi.fn((h: bigint) => h === WALL_SKETCH)
      const resolved = resolveIdleDrawTarget(
        sc as unknown as WasmScene, new SketchPickCache(), rayThrough([0, 0, 0.4]), INSTANCE_CTX,
      )
      expect(resolved.target.kind).toBe('plane')
    })

    it('refuses to adopt a hovered sketch that belongs to a DIFFERENT (or no) definition, falling to a def-owned plane target instead', () => {
      const resolved = resolveIdleDrawTarget(
        instanceScene(WALL_PLANE_ARR, WALL_SKETCH, [] /* WALL_SKETCH is not a member */),
        new SketchPickCache(),
        rayThrough([0, 0, 0.4]),
        INSTANCE_CTX,
      )
      // Falls through to the plane-mode default — carrying the instance
      // handle, so `runSketchGesture` mints a NEW def-owned sketch rather
      // than drawing into the foreign one under the cursor.
      expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane(), instance: INSTANCE })
    })

    it('the empty-space plane-mode default carries the instance handle (the fix for plane-mode escaping to the world)', () => {
      const resolved = resolveIdleDrawTarget(
        instanceScene(WALL_PLANE_ARR, null, []),
        new SketchPickCache(),
        rayThrough([0, 0, 0.4]),
        INSTANCE_CTX,
      )
      expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane(), instance: INSTANCE })
    })

    // The frame-conversion finding: every test above uses an IDENTITY pose,
    // where local and world coordinates coincide — a mapping bug would go
    // unnoticed. These use a genuinely non-identity (translated) pose.
    const POSED_INSTANCE_POSE = new Float64Array([1, 0, 0, 10, 0, 1, 0, 20, 0, 0, 1, 0])

    it('adopts a hovered MEMBER sketch through a genuinely non-identity (translated) instance pose', () => {
      const resolved = resolveIdleDrawTarget(
        instanceScene(WALL_PLANE_ARR, WALL_SKETCH, [WALL_SKETCH], POSED_INSTANCE_POSE),
        new SketchPickCache(),
        rayThrough([10, 20, 0.4]), // the sketch's WORLD position under this pose
        INSTANCE_CTX,
      )
      expect(resolved.target).toEqual({ kind: 'existing', handle: WALL_SKETCH, instance: INSTANCE })
      expect(resolved.plane.ground).toBe(false)
      // The returned plane is genuinely WORLD-space — posed forward, not
      // the raw local plane `sketch_plane` reported.
      expect(resolved.plane.origin).toEqual([10, 20, 0])
    })

    it('a ray at the sketch\'s stale LOCAL position misses under a posed instance — proof the pose is genuinely applied, not silently skipped', () => {
      const resolved = resolveIdleDrawTarget(
        instanceScene(WALL_PLANE_ARR, WALL_SKETCH, [WALL_SKETCH], POSED_INSTANCE_POSE),
        new SketchPickCache(),
        // The sketch's RAW local position — no longer where it sits once
        // posed. Before the fix, `planeFromSketch`/`rayLandsOnSketch` would
        // test this local-frame plane directly, coincidentally "landing"
        // regardless of instance pose. After the fix, it correctly misses.
        rayThrough([0, 0, 0.4]),
        INSTANCE_CTX,
      )
      expect(resolved.target).toEqual({
        kind: 'plane',
        plane: groundDrawPlane(),
        instance: INSTANCE,
      })
    })
  })
})

describe('resolveClickDrawTarget', () => {
  const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
  const scene = {
    pick_sketch: vi.fn(() => undefined),
    pick_sketch_region: vi.fn(() => undefined),
    pick_sketch_region_in_instance: vi.fn(() => undefined),
    axes: vi.fn(() => new Float64Array(WORLD_FRAME_FLAT)),
  } as unknown as WasmScene

  it('an active idle plane lock beats sketch/ground resolution', () => {
    const resolved = resolveClickDrawTarget(
      scene, new SketchPickCache(), 1, { x: 2, y: 3, z: 4, kind: 'endpoint' }, RAY, TOP,
    )
    expect(resolved).toEqual({ plane: axisDrawPlane(1, [2, 3, 4]), target: { kind: 'plane', plane: axisDrawPlane(1, [2, 3, 4]), instance: null } })
  })

  it('a lock with no snap yet resolves to null — nothing to click through', () => {
    expect(resolveClickDrawTarget(scene, new SketchPickCache(), 1, null, RAY, TOP)).toBeNull()
  })

  it('with no lock it defers to resolveIdleDrawTarget', () => {
    const resolved = resolveClickDrawTarget(scene, new SketchPickCache(), null, null, RAY, TOP)
    expect(resolved).toEqual({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane(), instance: null } })
  })

  it('an idle-locked click inside an instance context carries the instance handle (THE original axis-lock symptom)', () => {
    const instanceCtx: EditContext = { kind: 'instance', id: 42n, component: 5n }
    const resolved = resolveClickDrawTarget(
      scene, new SketchPickCache(), 1, { x: 2, y: 3, z: 4, kind: 'endpoint' }, RAY, instanceCtx,
    )
    expect(resolved).toEqual({
      plane: axisDrawPlane(1, [2, 3, 4]),
      target: { kind: 'plane', plane: axisDrawPlane(1, [2, 3, 4]), instance: 42n },
    })
  })

  it('an idle lock reads the CURRENT drawing-axes frame from the scene, not literal world axes', () => {
    const rotatedScene = {
      pick_sketch: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => undefined),
      pick_sketch_region_in_instance: vi.fn(() => undefined),
      axes: vi.fn(() => new Float64Array([5, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1])),
    } as unknown as WasmScene
    const resolved = resolveClickDrawTarget(
      rotatedScene, new SketchPickCache(), 0, { x: 2, y: 3, z: 4, kind: 'endpoint' }, RAY, TOP,
    )
    // Axis 0 (red) of the moved frame is world [0,1,0], not world [1,0,0].
    expect(resolved?.plane.normal).toEqual([0, 1, 0])
    expect(resolved?.plane.ground).toBe(false)
  })
})

// ---------------------------------------------------------------------------
// The drawing plane follows the view (view-plane design §3, picker step 7).
// ---------------------------------------------------------------------------

const DEG = Math.PI / 180

/** The camera at `eye` looking at the origin. */
function lookAtOrigin(eye: V3): CameraView {
  const l = Math.hypot(eye[0], eye[1], eye[2])
  return { dir: [-eye[0] / l, -eye[1] / l, -eye[2] / l], eye }
}

/** A camera looking along `dir` from 10 units back along it, so every axis
 *  plane through the origin it faces lies in front of the eye. */
function fwd(dir: V3): CameraView {
  return { dir, eye: [-10 * dir[0], -10 * dir[1], -10 * dir[2]] }
}

/** A camera `elevDeg` above level (negative: below, looking up), turned
 *  `azDeg` off square to the green (y) wall toward red, looking at the
 *  origin from 10 units away. */
function viewAt(elevDeg: number, azDeg: number): CameraView {
  const e = elevDeg * DEG
  const a = azDeg * DEG
  return fwd([Math.cos(e) * Math.sin(a), Math.cos(e) * Math.cos(a), -Math.sin(e)])
}

// The Viewport's own view poses (math.ts HOME_EYE_OFFSET, Viewport.tsx
// STANDARD_VIEWS, POLE_TILT = 0.001).
const HOME_VIEW = lookAtOrigin([3.5, -3.0, 2.5])
const FRONT_VIEW = lookAtOrigin([0, -1, 0])
const BACK_VIEW = lookAtOrigin([0, 1, 0])
const RIGHT_VIEW = lookAtOrigin([1, 0, 0])
const LEFT_VIEW = lookAtOrigin([-1, 0, 0])
const TOP_VIEW = lookAtOrigin([0, -0.001, 1])
const BOTTOM_VIEW = lookAtOrigin([0, -0.001, -1])
const ISO_VIEW = lookAtOrigin([1, -1, 1])

function rotatedAboutZ(deg: number, origin: V3 = [0, 0, 0]): DrawingAxes {
  const c = Math.cos(deg * DEG)
  const s = Math.sin(deg * DEG)
  return { origin, x: [c, s, 0], y: [-s, c, 0], z: [0, 0, 1] }
}

describe('inferredAxisPlane (view-plane §3)', () => {
  it('GROUND_PREFERENCE is cot(20°)', () => {
    expect(GROUND_PREFERENCE).toBeCloseTo(2.7475, 4)
    expect(Math.atan(1 / GROUND_PREFERENCE) / DEG).toBeCloseTo(20, 12)
  })

  it('the home view stays on the ground — the EXACT groundDrawPlane(), bit for bit', () => {
    // The pure "most facing" rule would stand the first rectangle up here
    // (|d·x| ≈ 0.66 beats |d·z| ≈ 0.47); the weight keeps the ground.
    expect(Math.abs(HOME_VIEW.dir[0])).toBeGreaterThan(Math.abs(HOME_VIEW.dir[2]))
    const plane = inferredAxisPlane(WORLD_DRAWING_AXES, HOME_VIEW)
    expect(plane).toEqual(groundDrawPlane())
    expect(plane.ground).toBe(true)
    for (const k of ['origin', 'normal', 'u', 'v'] as const) {
      plane[k].forEach((c, i) => expect(Object.is(c, groundDrawPlane()[k][i]), `${k}[${i}]`).toBe(true))
    }
  })

  it('Front and Back give green, Right and Left give red, through the origin', () => {
    for (const view of [FRONT_VIEW, BACK_VIEW]) {
      const plane = inferredAxisPlane(WORLD_DRAWING_AXES, view)
      expect(inferredAxis(WORLD_DRAWING_AXES, view)).toBe(1)
      expect(plane).toEqual(axisDrawPlane(1, [0, 0, 0]))
      expect(plane.normal).toEqual([0, 1, 0])
      expect(plane.ground).toBe(false)
    }
    for (const view of [RIGHT_VIEW, LEFT_VIEW]) {
      expect(inferredAxis(WORLD_DRAWING_AXES, view)).toBe(0)
      expect(inferredAxisPlane(WORLD_DRAWING_AXES, view)).toEqual(axisDrawPlane(0, [0, 0, 0]))
    }
  })

  it('Top, Bottom and Iso give the ground (Iso is a three-way tie, and the ground wins ties)', () => {
    for (const view of [TOP_VIEW, BOTTOM_VIEW, ISO_VIEW]) {
      expect(inferredAxis(WORLD_DRAWING_AXES, view)).toBe(2)
      expect(inferredAxisPlane(WORLD_DRAWING_AXES, view)).toEqual(groundDrawPlane())
    }
  })

  it('square-on to a wall the switch sits at 20°: 19° gives green, 21° the ground', () => {
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(19, 0))).toBe(1)
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(21, 0))).toBe(2)
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(19.9, 0))).toBe(1)
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(20.1, 0))).toBe(2)
  })

  it('at the home view\'s azimuth (40.6°) the switch sits at 15.4°', () => {
    // Analytically: tan(elevation) = cos(40.6°) · tan(20°).
    const sw = Math.atan(Math.cos(40.6 * DEG) * Math.tan(20 * DEG)) / DEG
    expect(sw).toBeCloseTo(15.4, 1)
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(sw - 0.5, 40.6))).toBe(1)
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(sw + 0.5, 40.6))).toBe(2)
    // Turned the other way, past 45°, red faces more than green.
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(sw - 0.5, 90 - 40.6))).toBe(0)
    // Corner-on (45°): 14.4°.
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(13.9, 45))).not.toBe(2)
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(14.9, 45))).toBe(2)
  })

  it('below the ground mirrors above it', () => {
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(-19, 0))).toBe(1)
    expect(inferredAxis(WORLD_DRAWING_AXES, viewAt(-21, 0))).toBe(2)
    expect(inferredAxisPlane(WORLD_DRAWING_AXES, viewAt(-21, 0))).toEqual(groundDrawPlane())
  })

  it('reads the frame: rotated 30° about Z, Front gives the frame\'s own green, not world Y', () => {
    const frame = rotatedAboutZ(30)
    const plane = inferredAxisPlane(frame, FRONT_VIEW)
    expect(inferredAxis(frame, FRONT_VIEW)).toBe(1)
    expect(plane.normal).toEqual(frame.y)
    expect(plane.ground).toBe(false)
    // Rotated a full 90°, the frame's red axis is world +Y, so Front faces red.
    const quarter = rotatedAboutZ(90)
    expect(inferredAxis(quarter, FRONT_VIEW)).toBe(0)
    expect(inferredAxis(quarter, RIGHT_VIEW)).toBe(1)
    expect(inferredAxisPlane(quarter, RIGHT_VIEW).normal).toEqual(quarter.y)
  })

  it('D1: a frame origin at z = 0.75 moves the blue plane with it', () => {
    const frame: DrawingAxes = { ...WORLD_DRAWING_AXES, origin: [0, 0, 0.75] }
    const plane = inferredAxisPlane(frame, HOME_VIEW)
    expect(inferredAxis(frame, HOME_VIEW)).toBe(2)
    expect(plane.ground).toBe(false)
    expect(plane.normal).toEqual([0, 0, 1])
    expect(plane.origin).toEqual([0, 0, 0.75])
    // A vertical choice passes through the frame's origin too.
    const raised: DrawingAxes = { ...WORLD_DRAWING_AXES, origin: [2, 3, 0.75] }
    expect(inferredAxisPlane(raised, FRONT_VIEW).origin).toEqual([2, 3, 0.75])
  })

  it('a zero or non-finite view direction answers the ground', () => {
    expect(inferredAxis(WORLD_DRAWING_AXES, fwd([0, 0, 0]))).toBe(2)
    expect(inferredAxis(WORLD_DRAWING_AXES, fwd([Number.NaN, 1, 0]))).toBe(2)
    expect(inferredAxis(WORLD_DRAWING_AXES, fwd([Number.POSITIVE_INFINITY, 0, 0]))).toBe(2)
  })

  it('behind the eye (§3.3): Front\'s direction from y = −2 gives green, from y = +2 or exactly on y = 0 the ground', () => {
    const dir: V3 = [0, 1, 0] // Front: looking along +Y
    expect(inferredAxis(WORLD_DRAWING_AXES, { dir, eye: [0, -2, 0] })).toBe(1)
    // Past the green plane, still looking along +Y: it is behind the eye.
    expect(inferredAxis(WORLD_DRAWING_AXES, { dir, eye: [0, 2, 0] })).toBe(2)
    expect(inferredAxisPlane(WORLD_DRAWING_AXES, { dir, eye: [0, 2, 0] })).toEqual(groundDrawPlane())
    // Standing in the plane counts as behind.
    expect(inferredAxis(WORLD_DRAWING_AXES, { dir, eye: [0.5, 0, 0.5] })).toBe(2)
    // Looking the other way (Back) from y = +2, the plane is in front again.
    expect(inferredAxis(WORLD_DRAWING_AXES, { dir: [0, -1, 0], eye: [0, 2, 0] })).toBe(1)
    // Red behaves the same way.
    expect(inferredAxis(WORLD_DRAWING_AXES, { dir: [-1, 0, 0], eye: [3, 0, 0] })).toBe(0)
    expect(inferredAxis(WORLD_DRAWING_AXES, { dir: [-1, 0, 0], eye: [-3, 0, 0] })).toBe(2)
  })

  it('behind the eye is measured from the frame\'s origin, and blue stays blue even when behind', () => {
    const frame: DrawingAxes = { ...WORLD_DRAWING_AXES, origin: [0, 5, 0] }
    // Eye at y = 2 is in front of the frame's green plane at y = 5.
    expect(inferredAxis(frame, { dir: [0, 1, 0], eye: [0, 2, 0] })).toBe(1)
    expect(inferredAxis(frame, { dir: [0, 1, 0], eye: [0, 6, 0] })).toBe(2)
    // Top view from BELOW the ground, still looking down: blue regardless.
    expect(inferredAxis(WORLD_DRAWING_AXES, { dir: [0, 0, -1], eye: [0, 0, -3] })).toBe(2)
    const raised: DrawingAxes = { ...WORLD_DRAWING_AXES, origin: [0, 0, 0.75] }
    expect(inferredAxisPlane(raised, { dir: [0, 0, -1], eye: [0, 0, 0.2] }).origin).toEqual([0, 0, 0.75])
  })

  it('the view direction need not be unit length', () => {
    expect(inferredAxis(WORLD_DRAWING_AXES, fwd([0, 40, -0.1]))).toBe(1)
    expect(inferredAxis(WORLD_DRAWING_AXES, fwd([3.5 * 7, -3 * 7, -2.5 * 7]))).toBe(2)
  })
})

describe('resolveIdleDrawTarget over nothing — step 7', () => {
  function emptyScene(frame: number[] = WORLD_FRAME_FLAT) {
    return {
      sketch_plane: vi.fn(() => undefined),
      pick_sketch: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => undefined),
      pick_sketch_region_in_instance: vi.fn(() => undefined),
      sketch_lines: vi.fn(() => new Float32Array()),
      axes: vi.fn(() => new Float64Array(frame)),
    }
  }
  const RAY: Ray = { origin: [0, -5, 1], direction: [0, 1, -0.05] }

  it('from Front: the green plane through the origin, carrying inferredAxis', () => {
    const resolved = resolveIdleDrawTarget(emptyScene() as unknown as WasmScene, new SketchPickCache(), RAY, TOP, undefined, '', FRONT_VIEW)
    const green = axisDrawPlane(1, [0, 0, 0])
    expect(resolved).toEqual({ plane: green, target: { kind: 'plane', plane: green, instance: null }, inferredAxis: 1 })
  })

  it('from the home view: the exact ground, shaped exactly as before (no inferredAxis)', () => {
    const resolved = resolveIdleDrawTarget(emptyScene() as unknown as WasmScene, new SketchPickCache(), RAY, TOP, undefined, '', HOME_VIEW)
    expect(resolved).toStrictEqual({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane(), instance: null } })
  })

  it('without a view direction: the literal ground, and the frame is never read', () => {
    const scene = emptyScene(rotatedAboutZFlat(30, [0, 0, 2]))
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RAY, TOP)
    expect(resolved).toStrictEqual({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane(), instance: null } })
    expect(resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RAY, TOP, undefined, '', null).plane).toEqual(groundDrawPlane())
    expect(scene.axes).not.toHaveBeenCalled()
  })

  it('D1: a moved frame\'s blue plane carries inferredAxis 2 (the cue shows it)', () => {
    const scene = emptyScene([0, 0, 0.75, 1, 0, 0, 0, 1, 0, 0, 0, 1])
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RAY, TOP, undefined, '', HOME_VIEW)
    expect(resolved.inferredAxis).toBe(2)
    expect(resolved.plane.origin).toEqual([0, 0, 0.75])
    expect(resolved.plane.ground).toBe(false)
  })

  it('inside an instance context the inferred plane carries the instance handle', () => {
    const ctx: EditContext = { kind: 'instance', id: 42n, component: 5n }
    const scene = { ...emptyScene(), component_member_sketches: vi.fn(() => new BigUint64Array()) }
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RAY, ctx, undefined, '', FRONT_VIEW)
    expect(resolved.target).toEqual({ kind: 'plane', plane: axisDrawPlane(1, [0, 0, 0]), instance: 42n })
  })

  it('a foreign face beats the inference (step 6 before step 7)', () => {
    const resolved = resolveIdleDrawTarget(
      emptyScene() as unknown as WasmScene, new SketchPickCache(), RAY, TOP,
      () => ({ point: [0, 0, 0.3], normal: [0, 0, 1], depth: 4 }), '', FRONT_VIEW,
    )
    expect(resolved.plane.normal).toEqual([0, 0, 1])
    expect(resolved.plane.origin).toEqual([0, 0, 0.3])
    expect(resolved.inferredAxis).toBeUndefined()
  })

  it('the memo never serves the old plane when the camera turns under a reused ray (§12 lens 2)', () => {
    const scene = emptyScene()
    const cache = new SketchPickCache()
    const wasm = scene as unknown as WasmScene
    expect(resolveIdleDrawTarget(wasm, cache, RAY, TOP, undefined, '', FRONT_VIEW).plane.normal).toEqual([0, 1, 0])
    expect(resolveIdleDrawTarget(wasm, cache, RAY, TOP, undefined, '', HOME_VIEW).plane).toEqual(groundDrawPlane())
    expect(resolveIdleDrawTarget(wasm, cache, RAY, TOP, undefined, '', RIGHT_VIEW).plane.normal).toEqual([1, 0, 0])
    // The same view twice is served from the memo: one pick for the ray.
    const again = resolveIdleDrawTarget(wasm, cache, RAY, TOP, undefined, '', RIGHT_VIEW)
    expect(again).toBe(resolveIdleDrawTarget(wasm, cache, RAY, TOP, undefined, '', RIGHT_VIEW))
    expect(scene.pick_sketch).toHaveBeenCalledTimes(1)
  })

  it('…nor when the drawing axes move under a reused ray with the same axis chosen', () => {
    let frame = WORLD_FRAME_FLAT
    const scene = { ...emptyScene(), axes: vi.fn(() => new Float64Array(frame)) }
    const cache = new SketchPickCache()
    const wasm = scene as unknown as WasmScene
    expect(resolveIdleDrawTarget(wasm, cache, RAY, TOP, undefined, '', FRONT_VIEW).plane.origin).toEqual([0, 0, 0])
    frame = [0, 4, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]
    expect(resolveIdleDrawTarget(wasm, cache, RAY, TOP, undefined, '', FRONT_VIEW).plane.origin).toEqual([0, 4, 0])
  })
})

function rotatedAboutZFlat(deg: number, origin: V3): number[] {
  const f = rotatedAboutZ(deg, origin)
  return [...f.origin, ...f.x, ...f.y, ...f.z]
}

describe('resolveClickDrawTarget with a view direction', () => {
  const scene = {
    pick_sketch: vi.fn(() => undefined),
    pick_sketch_region: vi.fn(() => undefined),
    pick_sketch_region_in_instance: vi.fn(() => undefined),
    axes: vi.fn(() => new Float64Array(WORLD_FRAME_FLAT)),
  } as unknown as WasmScene
  const RAY: Ray = { origin: [0, -5, 1], direction: [0, 1, -0.05] }

  it('unlocked, it passes the view through to the inference', () => {
    expect(resolveClickDrawTarget(scene, new SketchPickCache(), null, null, RAY, TOP, undefined, '', FRONT_VIEW)?.plane)
      .toEqual(axisDrawPlane(1, [0, 0, 0]))
  })

  it('an arrow lock beats the inference: ↑ from Front draws on the ground through the click', () => {
    const resolved = resolveClickDrawTarget(scene, new SketchPickCache(), 2, { x: 1, y: 2, z: 0, kind: 'free' }, RAY, TOP, undefined, '', FRONT_VIEW)
    expect(resolved?.plane.ground).toBe(true)
    expect(resolved?.inferredAxis).toBeUndefined()
  })
})

describe('DrawPlaneTracker — the idle hover plane (view-plane §5, view-die §3.4)', () => {
  const GREEN = axisDrawPlane(1, [0, 0, 0])
  const ray: Ray = { origin: [0.4, -5, 1], direction: [0, 1, -0.1] }
  const SCENE = {} as unknown as WasmScene
  const base = { anchored: null, lock: null, pinned: null, lockHover: null, fallback: groundDrawPlane(), frame: WORLD_DRAWING_AXES } as const

  it('the inferred plane over empty space: kind space, through the ray\'s hit on it', () => {
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [9, 9, 9], false)
    const state = t.state(base)
    expect(state.plane).toBe(GREEN)
    expect(state.pip).toBe('green')
    expect(state.hold).toBe('inferred')
    expect(state.hover?.kind).toBe('space')
    expect(state.hover?.point[0]).toBeCloseTo(0.4, 12)
    expect(state.hover?.point[1]).toBeCloseTo(0, 12)
    expect(state.hover?.point[2]).toBeCloseTo(0.5, 12)
  })

  it('the ground over nothing: kind ground', () => {
    const t = new DrawPlaneTracker()
    const down: Ray = { origin: [0, -5, 3], direction: [0, 1, -1] }
    t.trackIdle(SCENE, { plane: groundDrawPlane() }, down, [0, 0, 0], false)
    expect(t.state(base).hover?.kind).toBe('ground')
    expect(t.state(base).pip).toBe('blue')
  })

  it('a moved frame\'s blue plane over nothing is space, not ground (D1)', () => {
    const raised = inferredAxisPlane({ ...WORLD_DRAWING_AXES, origin: [0, 0, 0.75] }, HOME_VIEW)
    const down: Ray = { origin: [0, -5, 3], direction: [0, 1, -1] }
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: raised, inferredAxis: 2 }, down, [0, 0, 0], false)
    expect(t.state(base).hover?.kind).toBe('space')
    expect(t.state(base).hover?.point[2]).toBeCloseTo(0.75, 12)
  })

  it('an active arrow lock beats it, with or without a hover', () => {
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], false)
    expect(t.state({ ...base, lock: 0 }).plane.normal).toEqual([1, 0, 0])
    expect(t.state({ ...base, lock: 0, lockHover: [1, 2, 3] }).plane).toEqual(axisDrawPlane(0, [1, 2, 3]))
  })

  it('an anchored gesture beats it', () => {
    const anchored = axisDrawPlane(0, [1, 1, 1])
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], false)
    expect(t.state({ ...base, anchored }).plane).toBe(anchored)
  })

  it('a ray that never reaches the plane falls back to the snapped point', () => {
    const parallel: Ray = { origin: [0, -5, 1], direction: [1, 0, 0] }
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, parallel, [2, 0, 1], false)
    expect(t.state(base).hover?.point).toEqual([2, 0, 1])
  })

  it('a hovered sketch\'s plane (+X through the origin): kind sketch, through the ray\'s foot at x = 0', () => {
    const red = drawPlaneThrough([0, 0, 0], [1, 0, 0])!
    const crossing: Ray = { origin: [3, -3, 0.5], direction: [-3, 3.9, -0.13] }
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: red, target: { kind: 'existing', handle: 11n, instance: null } }, crossing, [0, 0, 0], false)
    const state = t.state(base)
    expect(state.plane).toBe(red)
    expect(state.pip).toBe('red')
    expect(state.hover?.kind).toBe('sketch')
    expect(state.hover?.point[0]).toBeCloseTo(0, 12)
    expect(state.hover?.point[1]).toBeCloseTo(0.9, 12)
    expect(state.hover?.point[2]).toBeCloseTo(0.37, 12)
  })

  it('a tilted face lights the custom pip', () => {
    const tilted = drawPlaneThrough([0, 0, 0], [0, -Math.SQRT1_2, Math.SQRT1_2])!
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: tilted, onFace: true }, ray, [0, 0, 0], false)
    expect(t.state(base).pip).toBe('custom')
    expect(t.state(base).hover?.kind).toBe('face')
  })

  it('inside an entered object only a face of it is a hover: a click off it lands nowhere', () => {
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], true)
    expect(t.state(base).hover).toBeNull()
    t.trackIdle(SCENE, { plane: GREEN, onFace: true }, ray, [0, 0, 0], true)
    expect(t.state(base).hover?.kind).toBe('face')
  })

  it('the pointer leaving the canvas drops the hover but keeps the tracked plane on the pip', () => {
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], false)
    t.leave()
    const off = t.state(base)
    expect(off.hover).toBeNull()
    expect(off.plane).toBe(GREEN)
    expect(off.pip).toBe('green')
    expect(off.hold).toBe('inferred')
    // A lock made off the canvas (a pip click) shows at once, and so does
    // its release back to the tracked plane.
    expect(t.state({ ...base, lock: 2 }).hold).toBe('locked')
    expect(t.state(base).plane).toBe(GREEN)
    // The next hover brings it back (a bare ray, before the picker has
    // resolved anything, does not).
    t.move(ray)
    expect(t.state(base).hover).toBeNull()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], false)
    expect(t.state(base).hover?.kind).toBe('space')
  })

  it('an idle hover that resolves to nothing (no snap) keeps the plane but drops the hover, so no cue lingers', () => {
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], false)
    t.clearHover()
    expect(t.state(base).hover).toBeNull()
    expect(t.state(base).plane).toBe(GREEN)
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], false)
    expect(t.state(base).hover?.kind).toBe('space')
  })

  it('clearIdle forgets the tracked plane; the fallback answers again', () => {
    const t = new DrawPlaneTracker()
    t.trackIdle(SCENE, { plane: GREEN, inferredAxis: 1 }, ray, [0, 0, 0], false)
    t.clearIdle()
    expect(t.state(base).plane).toEqual(groundDrawPlane())
  })
})

describe('DrawPlaneTracker — the custom pip\'s candidate (view-die §3.3, D4)', () => {
  const r = (30 * Math.PI) / 180
  const TURNED = drawPlaneThrough([0, 0, 0], [Math.cos(r), Math.sin(r), 0])!
  const GROUND = groundDrawPlane()
  const RAY: Ray = { origin: [5, 1, 0.5], direction: [-1, 0, 0] }
  const DOWN: Ray = { origin: [3, -2, 5], direction: [0, 0, -1] }

  /** A scene whose revision, axes and open sessions the test can change. */
  function world() {
    const state = { gen: 1n, frame: [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1], sessions: [] as { kind: string; id: bigint }[] }
    const scene = {
      history_generation: () => state.gen,
      axes: () => new Float64Array(state.frame),
      session_stack: () => state.sessions.map((n) => ({ ...n, free: () => undefined })),
    } as unknown as WasmScene
    return { scene, state }
  }
  function setup() {
    let t = 1000
    const clock = { advance: (ms: number) => { t += ms } }
    const tracker = new DrawPlaneTracker(() => t)
    const { scene, state } = world()
    const overTurned = () => tracker.trackIdle(scene, { plane: TURNED, onFace: true }, RAY, [0, 1, 0.5], false)
    const overGround = () => tracker.trackIdle(scene, { plane: GROUND }, DOWN, [3, -2, 0], false)
    return { tracker, scene, state, clock, overTurned, overGround }
  }

  it('set by a hover over a custom plane, through the ray\'s hit; an axis-plane hover leaves it alone', () => {
    const { tracker, scene, overTurned, overGround } = setup()
    expect(tracker.customCandidate(scene)).toBeNull()
    overTurned()
    overGround()
    const c = tracker.customCandidate(scene)
    expect(c?.plane).toBe(TURNED)
    // On the turned plane, where the ray met it.
    const p = c!.through
    expect(p[0] * Math.cos(r) + p[1] * Math.sin(r)).toBeCloseTo(0, 9)
    expect(p[1]).toBeCloseTo(1, 9)
  })

  it('live while the cursor is over it, however long; expires 2000 ms after that hover ends', () => {
    const { tracker, scene, clock, overTurned, overGround } = setup()
    expect(CUSTOM_CANDIDATE_MS).toBe(2000)
    overTurned()
    clock.advance(5000) // resting on the face
    expect(tracker.customCandidate(scene)).not.toBeNull()
    expect(tracker.customCandidateTtl()).toBeNull() // no expiry while hovered
    overGround() // the hover ends now
    expect(tracker.customCandidateTtl()).toBe(2000)
    clock.advance(1500)
    overGround()
    expect(tracker.customCandidateTtl()).toBe(500)
    clock.advance(500)
    expect(tracker.customCandidate(scene)).not.toBeNull()
    clock.advance(1)
    expect(tracker.customCandidate(scene)).toBeNull()
    expect(tracker.customCandidateTtl()).toBeNull()
  })

  it('resting on the face, then straight off the canvas: still live at the pip', () => {
    const { tracker, scene, clock, overTurned } = setup()
    overTurned()
    clock.advance(5000)
    tracker.leave()
    clock.advance(5000)
    expect(tracker.customCandidate(scene)?.plane).toBe(TURNED)
  })

  it('live at the pointer leaving the canvas: no timeout until the pointer moves on the canvas again', () => {
    const { tracker, scene, clock, overTurned, overGround } = setup()
    overTurned()
    clock.advance(800)
    overGround()
    tracker.leave()
    expect(tracker.customCandidateTtl()).toBeNull() // latched: nothing to wait for
    clock.advance(60_000)
    expect(tracker.customCandidate(scene)?.plane).toBe(TURNED)
    // Back on the canvas: the window counts from the original hover again.
    tracker.move(DOWN)
    expect(tracker.customCandidate(scene)).toBeNull()
  })

  it('back on the canvas inside the window, it stays live until the window closes', () => {
    const { tracker, scene, clock, overTurned } = setup()
    overTurned()
    tracker.leave()
    clock.advance(500)
    tracker.move(DOWN)
    expect(tracker.customCandidate(scene)).not.toBeNull()
    clock.advance(1501)
    expect(tracker.customCandidate(scene)).toBeNull()
  })

  it('expired before the leave: dead, and it stays dead off the canvas (D5)', () => {
    const { tracker, scene, clock, overTurned, overGround } = setup()
    overTurned()
    overGround()
    clock.advance(2001)
    tracker.leave()
    expect(tracker.customCandidate(scene)).toBeNull()
    tracker.move(DOWN)
    expect(tracker.customCandidate(scene)).toBeNull()
  })

  it('cleared by clearCandidate, clearIdle (a cancel) and reset', () => {
    for (const clear of ['clearCandidate', 'clearIdle', 'reset'] as const) {
      const { tracker, scene, overTurned } = setup()
      overTurned()
      tracker.leave()
      tracker[clear]()
      expect(tracker.customCandidate(scene)).toBeNull()
    }
  })

  it('stale after a document change (an edit, undo or redo), a drawing-axes move, or a session entered or left', () => {
    const change: ((s: ReturnType<typeof world>['state']) => void)[] = [
      (s) => { s.gen = 2n },
      (s) => { s.frame = [1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1] },
      (s) => { s.sessions = [{ kind: 'group', id: 4n }] },
    ]
    for (const apply of change) {
      const { tracker, scene, state, overTurned } = setup()
      overTurned()
      tracker.leave()
      apply(state)
      expect(tracker.customCandidate(scene)).toBeNull()
    }
  })

  it('a camera move does not clear it (a new fallback plane is no hover)', () => {
    const { tracker, scene, overTurned } = setup()
    overTurned()
    tracker.leave()
    tracker.state({ anchored: null, lock: null, pinned: null, lockHover: null, fallback: axisDrawPlane(1, [0, 0, 0]), frame: WORLD_DRAWING_AXES })
    expect(tracker.customCandidate(scene)).not.toBeNull()
  })

  it('the custom pip acts on a custom pin or the live candidate; its click pins the candidate, not the tracked ground', () => {
    const { tracker, scene, overTurned, overGround } = setup()
    const pin = new PlanePin()
    expect(customPipPlaneFor(scene, pin, tracker)).toBeNull()
    // A tracked hover alone counts for nothing: it carries no invalidation.
    pin.trackHover(TURNED, [0, 1, 0.5])
    expect(customPipPlaneFor(scene, pin, tracker)).toBeNull()
    expect(pinCustomPlane(scene, pin, tracker)).toBe(false)
    overTurned()
    expect(customPipPlaneFor(scene, pin, tracker)).toBe(TURNED) // under the cursor
    pin.trackHover(GROUND, [3, -2, 0]) // the pointer crossed the ground
    overGround()
    tracker.leave()
    expect(customPipPlaneFor(scene, pin, tracker)).toBe(TURNED) // the candidate
    expect(pinCustomPlane(scene, pin, tracker)).toBe(true)
    expect(pin.current?.plane).toBe(TURNED)
    expect(customPipPlaneFor(scene, pin, tracker)).toBe(TURNED) // pinned: a click releases
    expect(pinCustomPlane(scene, pin, tracker)).toBe(true)
    expect(pin.current).toBeNull()
  })

  it('an undo with the pointer on the die disables the pip and pins nothing stale, even with the face still tracked', () => {
    const { tracker, scene, state, overTurned } = setup()
    const pin = new PlanePin()
    pin.trackHover(TURNED, [0, 1, 0.5])
    overTurned()
    tracker.leave()
    expect(customPipPlaneFor(scene, pin, tracker)).toBe(TURNED)
    state.gen = 2n // undo
    expect(customPipPlaneFor(scene, pin, tracker)).toBeNull()
    expect(pinCustomPlane(scene, pin, tracker)).toBe(false)
    expect(pin.current).toBeNull()
  })

  it('a pinned axis plane gives the custom pip nothing to act on', () => {
    const { tracker, scene } = setup()
    const pin = new PlanePin()
    pin.pinPlane({ plane: GROUND, through: [0, 0, 0] })
    expect(customPipPlaneFor(scene, pin, tracker)).toBeNull()
  })
})

describe('idleHoverKind — what the cursor is over (view-die §4)', () => {
  const GREEN = axisDrawPlane(1, [0, 0, 0])
  const face = (boundary: boolean, offObject: boolean) => ({ object: 7n, face: 2n, instance: null, boundary, offObject })

  it('a face under the plain ray is face; a boundary lean onto one is lean, off the object leanExtended', () => {
    expect(idleHoverKind({ plane: GREEN, onFace: true, face: face(false, false) })).toBe('face')
    expect(idleHoverKind({ plane: GREEN, onFace: true, face: face(true, false) })).toBe('lean')
    expect(idleHoverKind({ plane: GREEN, onFace: true, face: face(true, true) })).toBe('leanExtended')
    expect(idleHoverKind({ plane: GREEN, onFace: true })).toBe('face')
  })

  it('a moved frame\'s blue plane that coincides with the ground is the ground', () => {
    const shifted = axisDrawPlane(2, [2, 3, 0], { origin: [2, 3, 0], x: [0, 1, 0], y: [-1, 0, 0], z: [0, 0, 1] })
    expect(idleHoverKind({ plane: shifted, inferredAxis: 2 })).toBe('ground')
    expect(idleHoverKind({ plane: axisDrawPlane(2, [0, 0, 0.75], { ...WORLD_DRAWING_AXES, origin: [0, 0, 0.75] }), inferredAxis: 2 })).toBe('space')
  })

  it('a sketch, the inferred plane, and the ground', () => {
    expect(idleHoverKind({ plane: GREEN, target: { kind: 'existing', handle: 3n, instance: null } })).toBe('sketch')
    expect(idleHoverKind({ plane: GREEN, inferredAxis: 1, target: { kind: 'plane', plane: GREEN, instance: null } })).toBe('space')
    expect(idleHoverKind({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane(), instance: null } })).toBe('ground')
  })

  it('resolveIdleDrawTarget flags only an ineligible face as onFace, and carries the face that decided it', () => {
    const ray: Ray = { origin: [0, -5, 1], direction: [0, 1, 0] }
    const scene = {
      pick_sketch: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => undefined),
      axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    } as unknown as WasmScene
    const ref = { object: 4n, face: 9n, instance: 6n, boundary: true, offObject: false }
    const hit = () => ({ point: [0, 0, 0.3] as V3, normal: [0, 0, 1] as V3, depth: 4, face: ref })
    const resolved = resolveIdleDrawTarget(scene, new SketchPickCache(), ray, TOP, hit, '', FRONT_VIEW)
    expect(resolved.onFace).toBe(true)
    expect(resolved.face).toEqual(ref)
    expect(idleHoverKind(resolved)).toBe('lean')
    expect(resolveIdleDrawTarget(scene, new SketchPickCache(), ray, TOP, undefined, '', FRONT_VIEW).onFace).toBeUndefined()
  })
})

describe('FaceLoopCache — the leaned-to face\'s outline in world space (view-die §4)', () => {
  /** A square face's outer loop in its object's own frame, and a mesh whose
   *  vertex normals are flat (all +Z) unless `curved`. */
  function loopScene(opts: { curved?: boolean; pose?: number[]; gen?: () => bigint } = {}) {
    const square = new Float32Array([0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1])
    const flat = [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]
    const bent = [1, 0, 0, 1, 0, 0, 0.9, 0.4359, 0, 0.9, 0.4359, 0]
    const mesh = { normals: vi.fn(() => new Float32Array(opts.curved === true ? bent : flat)), free: vi.fn() }
    const scene = {
      history_generation: vi.fn(opts.gen ?? (() => 1n)),
      face_boundary: vi.fn(() => square),
      object_mesh: vi.fn(() => mesh),
      face_mesh_range: vi.fn(() => new Uint32Array([0, 4])),
      instance_pose: vi.fn(() => (opts.pose === undefined ? undefined : new Float64Array(opts.pose))),
    }
    return { scene: scene as unknown as WasmScene, raw: scene, mesh }
  }
  const ref = (instance: bigint | null = null) => ({ object: 7n, face: 2n, instance, boundary: true, offObject: false })

  it('a world face\'s loop as stored', () => {
    const { scene } = loopScene()
    expect(new FaceLoopCache().loopFor(scene, ref())).toEqual([[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]])
  })

  it('a face inside an instance is posed through that instance into world space', () => {
    // Translate by (10, 0, 0) and turn 90° about Z: (x, y, z) → (10 − y, x, z).
    const pose = [0, -1, 0, 10, 1, 0, 0, 0, 0, 0, 1, 0]
    const { scene } = loopScene({ pose })
    const loop = new FaceLoopCache().loopFor(scene, ref(5n))!
    expect(loop.map((p) => p.map((c) => Math.round(c * 1e9) / 1e9))).toEqual([[10, 0, 1], [10, 1, 1], [9, 1, 1], [9, 0, 1]])
  })

  it('an instance with no pose gives no loop', () => {
    const { scene } = loopScene()
    expect(new FaceLoopCache().loopFor(scene, ref(5n))).toBeNull()
  })

  it('a curved face (a chord facet of a wall) gives no loop: the cue shows the patch instead', () => {
    const { scene, mesh } = loopScene({ curved: true })
    expect(new FaceLoopCache().loopFor(scene, ref())).toBeNull()
    expect(mesh.free).toHaveBeenCalled()
  })

  it('fails closed to curved (no loop, so the patch) when flatness cannot be read', () => {
    const throwing = loopScene()
    throwing.raw.object_mesh.mockImplementation(() => { throw new Error('UnknownObject') })
    expect(new FaceLoopCache().loopFor(throwing.scene, ref())).toBeNull()
    const noRange = loopScene()
    noRange.raw.face_mesh_range.mockImplementation(() => undefined as unknown as Uint32Array<ArrayBuffer>)
    expect(new FaceLoopCache().loopFor(noRange.scene, ref())).toBeNull()
    const short = loopScene()
    short.raw.face_mesh_range.mockImplementation(() => new Uint32Array([0]))
    expect(new FaceLoopCache().loopFor(short.scene, ref())).toBeNull()
    const past = loopScene()
    past.raw.face_mesh_range.mockImplementation(() => new Uint32Array([2, 4])) // runs past the 4 normals
    expect(new FaceLoopCache().loopFor(past.scene, ref())).toBeNull()
    const missing = loopScene()
    delete (missing.raw as Partial<typeof missing.raw>).object_mesh
    expect(new FaceLoopCache().loopFor(missing.scene, ref())).toBeNull()
  })

  it('a stale face gives no loop', () => {
    const { scene, raw } = loopScene()
    raw.face_boundary.mockImplementation(() => { throw new Error('UnknownFace') })
    expect(new FaceLoopCache().loopFor(scene, ref())).toBeNull()
  })

  it('memoized per document revision: one tessellation per edit', () => {
    let gen = 1n
    const { scene, raw } = loopScene({ gen: () => gen })
    const cache = new FaceLoopCache()
    cache.loopFor(scene, ref())
    cache.loopFor(scene, ref())
    expect(raw.object_mesh).toHaveBeenCalledTimes(1)
    gen = 2n
    cache.loopFor(scene, ref())
    expect(raw.object_mesh).toHaveBeenCalledTimes(2)
  })
})

describe('groundNaturalTarget — the plane a released lock returns to (view-plane §6)', () => {
  const anchor: V3 = [1, 0.5, 0.3]

  it('a locked click from Front releases to green through the anchor, not to ground', () => {
    const natural = groundNaturalTarget(TOP, anchor, FRONT_VIEW, WORLD_DRAWING_AXES)
    expect(natural.plane).toEqual(axisDrawPlane(1, anchor))
    expect(natural.plane.normal).toEqual([0, 1, 0])
    // Locked to red at the click, then → again mid-gesture: the lock clears
    // and the gesture lands where an unlocked click would have, through the anchor.
    const released = nextGestureLockPlane(0, 'ArrowRight', anchor, natural, WORLD_DRAWING_AXES, TOP)
    expect(released.lock).toBeNull()
    expect(released.plane.normal).toEqual([0, 1, 0])
    expect(released.plane.origin).toEqual(anchor)
  })

  it('from the home view: a level plane through the anchor; the exact ground when the anchor is at z = 0', () => {
    expect(groundNaturalTarget(TOP, anchor, HOME_VIEW, WORLD_DRAWING_AXES).plane).toEqual(axisDrawPlane(2, anchor))
    const onGround = groundNaturalTarget(TOP, [1, 2, 0], HOME_VIEW, WORLD_DRAWING_AXES)
    expect(onGround.plane.ground).toBe(true)
    expect(onGround).toEqual(groundNaturalTarget(TOP, [1, 2, 0]))
  })

  it('D1: under a moved frame the blue choice follows the frame; without a view, the literal Z as before', () => {
    // A frame tilted 90° about X: its blue axis is world −Y.
    const tilted: DrawingAxes = { origin: [0, 0, 0], x: [1, 0, 0], y: [0, 0, 1], z: [0, -1, 0] }
    expect(inferredAxis(tilted, TOP_VIEW)).not.toBe(2)
    expect(inferredAxis(tilted, FRONT_VIEW)).toBe(2)
    expect(groundNaturalTarget(TOP, anchor, FRONT_VIEW, tilted).plane.normal).toEqual([0, -1, 0])
    expect(groundNaturalTarget(TOP, anchor).plane.normal).toEqual([0, 0, 1])
    expect(groundNaturalTarget(TOP, anchor, null, tilted).plane.normal).toEqual([0, 0, 1])
  })

  it('carries the instance handle', () => {
    const ctx: EditContext = { kind: 'instance', id: 42n, component: 5n }
    expect(groundNaturalTarget(ctx, anchor, FRONT_VIEW, WORLD_DRAWING_AXES).target).toEqual({ kind: 'plane', plane: axisDrawPlane(1, anchor), instance: 42n })
  })
})

// ---------------------------------------------------------------------------
// Over a sketch's filled interior (view-plane design §2 step 4b), and a
// ground sketch keeping the ground from a low camera.
// ---------------------------------------------------------------------------

describe('resolveIdleDrawTarget over a sketch fill (view-plane §2 step 4b)', () => {
  const RED_SKETCH = 11n
  const GROUND_SKETCH = 12n
  const RED_PLANE = new Float64Array([0, 0, 0, 1, 0, 0])
  const GROUND_PLANE = new Float64Array([0, 0, 0, 0, 0, 1])
  // A 1 × 1 square on the ground, its corner at the origin.
  const GROUND_SQUARE = new Float32Array([
    0, 0, 0, 1, 0, 0,
    1, 0, 0, 1, 1, 0,
    1, 1, 0, 0, 1, 0,
    0, 1, 0, 0, 0, 0,
  ])

  function fakePick(sketch: bigint, detached = false) {
    return { sketch: () => sketch, region: () => 1n, depth: () => 99, detached: () => detached, free: vi.fn() }
  }

  /** A scene whose edge pick (`pick_sketch`) answers `edge` and whose fill
   *  pick (`pick_sketch_region` and its `_in_instance` sibling) answers a
   *  fresh `fill()` pick per call. */
  function fillScene(opts: {
    edge?: bigint
    fill?: () => ReturnType<typeof fakePick> | undefined
    lines?: Float32Array
    pose?: Float64Array
  }) {
    const planes = new Map<bigint, Float64Array>([[RED_SKETCH, RED_PLANE], [GROUND_SKETCH, GROUND_PLANE]])
    const picks: ReturnType<typeof fakePick>[] = []
    const pickFill = (..._ray: unknown[]) => {
      const p = opts.fill?.()
      if (p !== undefined) picks.push(p)
      return p
    }
    return {
      picks,
      sketch_plane: vi.fn((h: bigint) => planes.get(h)),
      pick_sketch: vi.fn(() => opts.edge),
      sketch_lines: vi.fn(() => opts.lines ?? new Float32Array()),
      sketch_is_detached: vi.fn(() => false),
      pick_sketch_region: vi.fn(pickFill),
      pick_sketch_region_in_instance: vi.fn(pickFill),
      instance_pose: vi.fn(() => opts.pose ?? new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])),
      axes: vi.fn(() => new Float64Array(WORLD_FRAME_FLAT)),
    }
  }

  /** A ray from the Front side, off-centre, into the middle of a rectangle
   *  standing on the red plane x = 0 (the ray crosses that plane 4.9 m out). */
  const RED_RAY: Ray = { origin: [3, -3, 0.5], direction: [-3, 3.9, -0.13] }

  it('a fill on the red plane beats step 7 from a Front view: that sketch, not the inferred green plane', () => {
    const scene = fillScene({ fill: () => fakePick(RED_SKETCH) })
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RED_RAY, TOP, undefined, '', FRONT_VIEW)
    expect(resolved.target).toEqual({ kind: 'existing', handle: RED_SKETCH, instance: null })
    expect(resolved.plane.normal[0]).toBeCloseTo(1, 12)
    expect(resolved.plane.normal[1]).toBeCloseTo(0, 12)
    expect(resolved.plane.normal[2]).toBeCloseTo(0, 12)
    expect(resolved.inferredAxis).toBeUndefined()
  })

  const LOW_VIEW = viewAt(15, 0) // below the 20° switch: step 7 would infer green
  /** From `LOW_VIEW`'s eye down onto the ground at (0.5, 0.5, 0). */
  const GROUND_RAY: Ray = {
    origin: LOW_VIEW.eye,
    direction: [0.5 - LOW_VIEW.eye[0], 0.5 - LOW_VIEW.eye[1], -LOW_VIEW.eye[2]],
  }
  /** …and onto the square's edge at (0.5, 0, 0). */
  const GROUND_EDGE_RAY: Ray = {
    origin: LOW_VIEW.eye,
    direction: [0.5 - LOW_VIEW.eye[0], -LOW_VIEW.eye[1], -LOW_VIEW.eye[2]],
  }

  it('a ground-sketch fill from a 15° view gives the ground, not the inferred vertical plane', () => {
    expect(inferredAxis(WORLD_DRAWING_AXES, LOW_VIEW)).toBe(1)
    const scene = fillScene({ fill: () => fakePick(GROUND_SKETCH) })
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), GROUND_RAY, TOP, undefined, '', LOW_VIEW)
    expect(resolved).toStrictEqual({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane(), instance: null } })
  })

  it('a ground-sketch EDGE from a 15° view gives the ground too', () => {
    const scene = fillScene({ edge: GROUND_SKETCH, lines: GROUND_SQUARE })
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), GROUND_EDGE_RAY, TOP, undefined, '', LOW_VIEW)
    expect(resolved).toStrictEqual({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane(), instance: null } })
    expect(scene.pick_sketch_region).not.toHaveBeenCalled() // the edge settled it
  })

  it('a DETACHED fill is never adopted (the pick\'s own flag): the hover falls through to step 7', () => {
    const scene = fillScene({ fill: () => fakePick(RED_SKETCH, true) })
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RED_RAY, TOP, undefined, '', FRONT_VIEW)
    expect(resolved.inferredAxis).toBe(1)
    expect(resolved.target.kind).toBe('plane')
  })

  it('an ineligible face strictly nearer than the fill wins; a farther one loses', () => {
    const face = (depth: number) => () => ({ point: [0, 0, 0.3] as V3, normal: [0, 0, 1] as V3, depth })
    const near = resolveIdleDrawTarget(
      fillScene({ fill: () => fakePick(RED_SKETCH) }) as unknown as WasmScene, new SketchPickCache(), RED_RAY, TOP, face(1), '', FRONT_VIEW,
    )
    expect(near.plane.normal).toEqual([0, 0, 1])
    expect(near.target).toEqual({ kind: 'plane', plane: near.plane, instance: null })
    const far = resolveIdleDrawTarget(
      fillScene({ fill: () => fakePick(RED_SKETCH) }) as unknown as WasmScene, new SketchPickCache(), RED_RAY, TOP, face(10), '', FRONT_VIEW,
    )
    expect(far.target).toEqual({ kind: 'existing', handle: RED_SKETCH, instance: null })
  })

  it('inside an instance context it asks pick_sketch_region_in_instance, and poses the plane through that instance', () => {
    const ctx: EditContext = { kind: 'instance', id: 3n, component: 5n }
    // The definition's red-plane sketch, posed +0.5 along x into the world.
    const scene = fillScene({ fill: () => fakePick(RED_SKETCH), pose: new Float64Array([1, 0, 0, 0.5, 0, 1, 0, 0, 0, 0, 1, 0]) })
    const resolved = resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RED_RAY, ctx, undefined, '', FRONT_VIEW)
    expect(scene.pick_sketch_region_in_instance).toHaveBeenCalledTimes(1)
    expect(scene.pick_sketch_region_in_instance.mock.calls[0][0]).toBe(3n)
    expect(scene.pick_sketch_region).not.toHaveBeenCalled()
    expect(scene.instance_pose).toHaveBeenCalledWith(3n)
    expect(resolved.target).toEqual({ kind: 'existing', handle: RED_SKETCH, instance: 3n })
    expect(resolved.plane.origin).toEqual([0.5, 0, 0])
  })

  it('frees every fill pick exactly once, hit or not', () => {
    const scene = fillScene({ fill: () => fakePick(RED_SKETCH) })
    resolveIdleDrawTarget(scene as unknown as WasmScene, new SketchPickCache(), RED_RAY, TOP, undefined, '', FRONT_VIEW)
    const detached = fillScene({ fill: () => fakePick(RED_SKETCH, true) })
    resolveIdleDrawTarget(detached as unknown as WasmScene, new SketchPickCache(), RED_RAY, TOP, undefined, '', FRONT_VIEW)
    for (const s of [scene, detached]) {
      expect(s.picks).toHaveLength(1)
      expect(s.picks[0].free).toHaveBeenCalledTimes(1)
    }
  })

  it('the fill pick is memoized per ray, even when a camera turn re-resolves the target', () => {
    const scene = fillScene({ fill: () => fakePick(RED_SKETCH) })
    const cache = new SketchPickCache()
    const wasm = scene as unknown as WasmScene
    expect(resolveIdleDrawTarget(wasm, cache, RED_RAY, TOP, undefined, '', FRONT_VIEW).target.kind).toBe('existing')
    expect(resolveIdleDrawTarget(wasm, cache, RED_RAY, TOP, undefined, '', HOME_VIEW).target.kind).toBe('existing')
    expect(resolveIdleDrawTarget(wasm, cache, RED_RAY, TOP, undefined, '', HOME_VIEW).target.kind).toBe('existing')
    expect(scene.pick_sketch_region).toHaveBeenCalledTimes(1)
    // A new ray asks again.
    resolveIdleDrawTarget(wasm, cache, { ...RED_RAY }, TOP, undefined, '', HOME_VIEW)
    expect(scene.pick_sketch_region).toHaveBeenCalledTimes(2)
  })
})

describe('resolveIdleDrawTarget at a named point (R18)', () => {
  const GROUND_SKETCH = 12n
  const WALL_SKETCH = 13n
  const GROUND_PLANE = new Float64Array([0, 0, 0, 0, 0, 1])
  const GREEN_WALL = new Float64Array([0, 0, 0, 0, 1, 0])
  const RED_WALL = new Float64Array([0, 0, 0, 1, 0, 0])
  const GROUND_SQUARE = new Float32Array([0, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0])

  function scene(edge: bigint | undefined, lines: Float32Array = GROUND_SQUARE) {
    const planes = new Map<bigint, Float64Array>([[GROUND_SKETCH, GROUND_PLANE], [WALL_SKETCH, GREEN_WALL]])
    return {
      sketch_plane: vi.fn((h: bigint) => planes.get(h)),
      pick_sketch: vi.fn(() => edge),
      pick_sketch_region: vi.fn(() => undefined),
      sketch_lines: vi.fn(() => lines),
      sketch_is_detached: vi.fn(() => false),
      axes: vi.fn(() => new Float64Array(WORLD_FRAME_FLAT)),
    } as unknown as WasmScene
  }

  const named = (p: V3, kind = 'endpoint') => ({ x: p[0], y: p[1], z: p[2], kind }) as never

  /** The Rectangle's far corner (0, 1, 0), seen from low and to the left
   *  (green inferred); the ray ends AT the corner. */
  const LEFT = viewAt(15, 0) // eye below the 20 degree switch, square to the green wall
  const ray = (view: CameraView, to: V3): Ray => ({
    origin: view.eye,
    direction: [to[0] - view.eye[0], to[1] - view.eye[1], to[2] - view.eye[2]],
  })
  const FAR: V3 = [0, 1, 0]

  it('a ground sketch\'s far corner from a green view starts on the green plane THROUGH the corner (not the ground, not projected)', () => {
    expect(inferredAxis(WORLD_DRAWING_AXES, LEFT)).toBe(1)
    const r = ray(LEFT, FAR)
    const resolved = resolveIdleDrawTarget(scene(GROUND_SKETCH), new SketchPickCache(), r, TOP, undefined, '', LEFT, named(FAR))
    expect(resolved.plane.normal).toEqual([0, 1, 0])
    expect(resolved.plane.origin).toEqual(FAR)
    expect(resolved.plane.ground).toBe(false)
    expect(resolved.target).toEqual({ kind: 'plane', plane: resolved.plane, instance: null })
  })

  it('the lifted result carries the inferred axis (the Facing hint shows), and none for an exact ground lift', () => {
    const r = ray(LEFT, FAR)
    const resolved = resolveIdleDrawTarget(scene(GROUND_SKETCH), new SketchPickCache(), r, TOP, undefined, '', LEFT, named(FAR))
    expect(resolved.inferredAxis).toBe(1)
    const grounded = resolveIdleDrawTarget(scene(undefined), new SketchPickCache(), ray(HOME_VIEW, [2, 2, 0]), TOP, undefined, '', HOME_VIEW, named([2, 2, 0]))
    expect(grounded.plane.ground).toBe(true)
    expect(grounded.inferredAxis).toBeUndefined()
  })

  it('the same corner from an ordinary (blue) view keeps the ground sketch\'s plane', () => {
    const r = ray(HOME_VIEW, FAR)
    const resolved = resolveIdleDrawTarget(scene(GROUND_SKETCH), new SketchPickCache(), r, TOP, undefined, '', HOME_VIEW, named(FAR))
    expect(resolved.plane.ground).toBe(true)
  })

  it('a corner on the green plane itself keeps that orientation (through the corner)', () => {
    const near: V3 = [0, 0, 0]
    const resolved = resolveIdleDrawTarget(scene(GROUND_SKETCH), new SketchPickCache(), ray(LEFT, near), TOP, undefined, '', LEFT, named(near))
    expect(resolved.plane.normal).toEqual([0, 1, 0])
    expect(resolved.plane.origin).toEqual(near)
  })

  it('no snap, or a snap that names no point, leaves the ground sketch\'s ground', () => {
    const r = ray(LEFT, FAR)
    expect(resolveIdleDrawTarget(scene(GROUND_SKETCH), new SketchPickCache(), r, TOP, undefined, '', LEFT, null).plane.ground).toBe(true)
    expect(resolveIdleDrawTarget(scene(GROUND_SKETCH), new SketchPickCache(), { ...r }, TOP, undefined, '', LEFT, named(FAR, 'on-edge')).plane.ground).toBe(true)
  })

  it('a named point off a hovered non-ground sketch\'s plane lifts it, nearer or not (a plane hides nothing)', () => {
    // A wall sketch on the green plane; the point is 1 m behind it from the eye.
    const wallScene = scene(WALL_SKETCH, new Float32Array([0, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0]))
    const r = ray(LEFT, [0.5, 0, 0])
    const behind: V3 = [0.5, 1, 0]
    const resolved = resolveIdleDrawTarget(wallScene, new SketchPickCache(), r, TOP, undefined, '', LEFT, named(behind))
    expect(resolved.plane.origin).toEqual(behind)
    expect(resolved.plane.normal).toEqual([0, 1, 0])
  })

  it('a named point on a hovered wall sketch that already faces the view keeps that sketch (the parallel early return)', () => {
    expect(inferredAxis(WORLD_DRAWING_AXES, LEFT)).toBe(1)
    const wallScene = scene(WALL_SKETCH, new Float32Array([0, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0]))
    const onWall: V3 = [0.5, 0, 0]
    const resolved = resolveIdleDrawTarget(wallScene, new SketchPickCache(), ray(LEFT, onWall), TOP, undefined, '', LEFT, named(onWall))
    expect(resolved.target.kind).toBe('existing')
    expect(resolved.plane.normal).toEqual([0, 1, 0])
    expect(resolved.plane.origin).toEqual([0, 0, 0])
  })

  it('a point on a face the picker chose still needs to be nearer the eye than the face to lift it', () => {
    const face = () => ({ point: [0, 0, 0.3] as V3, normal: [0, 0, 1] as V3, depth: 1 })
    const r: Ray = { origin: [0.5, 0.5, 5], direction: [0, 0, -1] }
    const hidden = resolveIdleDrawTarget(scene(undefined), new SketchPickCache(), r, TOP, face, '', null, named([0.5, 0.5, -1]))
    expect(hidden.onFace).toBe(true)
    expect(hidden.plane.origin).toEqual([0, 0, 0.3])
    const nearer = resolveIdleDrawTarget(scene(undefined), new SketchPickCache(), { ...r }, TOP, face, '', null, named([0.5, 0.5, 2]))
    expect(nearer.plane.origin).toEqual([0.5, 0.5, 2])
  })

  it('a red view lifts to the red plane through a corner of a ground sketch', () => {
    const RIGHT = viewAt(15, 90)
    expect(inferredAxis(WORLD_DRAWING_AXES, RIGHT)).toBe(0)
    const corner: V3 = [1, 1, 0]
    const resolved = resolveIdleDrawTarget(scene(GROUND_SKETCH), new SketchPickCache(), ray(RIGHT, corner), TOP, undefined, '', RIGHT, named(corner))
    expect(resolved.plane.normal).toEqual([1, 0, 0])
    expect(resolved.plane.origin).toEqual(corner)
  })

  it('a red wall sketch\'s corner from a green view takes the green plane (the sketch\'s own orientation does not hold)', () => {
    const planes = new Map<bigint, Float64Array>([[WALL_SKETCH, RED_WALL]])
    const s = {
      sketch_plane: vi.fn((h: bigint) => planes.get(h)),
      pick_sketch: vi.fn(() => WALL_SKETCH),
      pick_sketch_region: vi.fn(() => undefined),
      sketch_lines: vi.fn(() => new Float32Array([0, 0, 0, 0, 1, 0])),
      sketch_is_detached: vi.fn(() => false),
      axes: vi.fn(() => new Float64Array(WORLD_FRAME_FLAT)),
    } as unknown as WasmScene
    const corner: V3 = [0, 1, 0]
    const resolved = resolveIdleDrawTarget(s, new SketchPickCache(), ray(LEFT, corner), TOP, undefined, '', LEFT, named(corner))
    expect(resolved.plane.normal).toEqual([0, 1, 0])
    expect(resolved.plane.origin).toEqual(corner)
  })
})
