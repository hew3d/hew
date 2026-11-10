import { describe, it, expect, vi } from 'vitest'
import {
  groundDrawPlane,
  planeFromSketch,
  axisDrawPlane,
  pointOnPlane,
  planeKey,
  isGroundPlane,
  drawPlaneCue,
  drawPlaneThrough,
  idleHoverCue,
  PATCH_OVER_FACES,
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

describe('drawPlaneCue', () => {
  const TILTED: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }

  it('null when nothing is anchored and no idle lock is active', () => {
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null })).toBeNull()
  })

  it('null when anchored on the GROUND plane (the world grid already covers it)', () => {
    expect(
      drawPlaneCue({ anchoredPlane: groundDrawPlane(), anchoredThrough: [1, 2, 0], idleLock: null, idleHover: null }),
    ).toBeNull()
  })

  it('a ground pin through a named point in the air shows the lifted plane the click will use', () => {
    const cue = drawPlaneCue({ anchoredPlane: groundDrawPlane(), anchoredThrough: [1, 1, 2], idleLock: null, idleHover: null })
    expect(cue).not.toBeNull()
    expect(cue?.plane.origin).toEqual([1, 1, 2])
    expect(cue?.plane.normal).toEqual([0, 0, 1])
    expect(cue?.plane.ground).toBe(false)
    expect(cue?.through).toEqual([1, 1, 2])
  })

  it('a ground pin through a named point back on the ground still shows no cue', () => {
    expect(
      drawPlaneCue({ anchoredPlane: groundDrawPlane(), anchoredThrough: [1, 1, 0], idleLock: null, idleHover: null }),
    ).toBeNull()
  })

  it('the frozen plane, through the anchor, when anchored on a NON-ground plane', () => {
    const cue = drawPlaneCue({ anchoredPlane: TILTED, anchoredThrough: [3, 0, 5], idleLock: null, idleHover: null })
    expect(cue).toEqual({ plane: TILTED, through: [3, 0, 5] })
  })

  it('anchored takes priority over an idle lock (mutually exclusive in practice, but anchored wins)', () => {
    const cue = drawPlaneCue({ anchoredPlane: TILTED, anchoredThrough: [3, 0, 5], idleLock: 2, idleHover: [9, 9, 9] })
    expect(cue).toEqual({ plane: TILTED, through: [3, 0, 5] })
  })

  it('null when idle-locked but no hover has landed yet', () => {
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 0, idleHover: null })).toBeNull()
  })

  it('null when idle and a hover exists but no lock is active', () => {
    expect(
      drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: [1, 2, 3] }),
    ).toBeNull()
  })

  it('the locked axis plane through the tracked hover point, when idle-locked with a hover', () => {
    const cue = drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 0, idleHover: [4, 5, 6] })
    expect(cue).toEqual({ plane: axisDrawPlane(0, [4, 5, 6]), through: [4, 5, 6] })
  })

  it('null when a Z-axis lock through a z=0 hover resolves to the exact ground plane', () => {
    // axisDrawPlane(2, [x,y,0]) returns groundDrawPlane()'s exact frame — the
    // same "reuses the ground path" case the click-time commit special-cases
    // (begin_ground_sketch, not begin_sketch_on_plane). No cue either.
    expect(
      drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 2, idleHover: [1, 2, 0] }),
    ).toBeNull()
  })

  it('a Z-axis lock through a hover OFF z=0 does produce a cue (a horizontal plane elevated off ground)', () => {
    const cue = drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 2, idleHover: [1, 2, 5] })
    expect(cue).toEqual({ plane: axisDrawPlane(2, [1, 2, 5]), through: [1, 2, 5] })
  })

  // ── Movable drawing axes (tool-parity §4): the idle-lock cue must resolve
  // the CURRENT frame, not world identity — mirrors `resolveClickDrawTarget`'s
  // own moved-frame coverage above, since the cue must show exactly where a
  // click through the same hover point would land.
  const ROTATED_FRAME: DrawingAxes = {
    origin: [5, 0, 0],
    x: [0, 1, 0],
    y: [-1, 0, 0],
    z: [0, 0, 1],
  }

  it('under a moved frame, the cue plane equals what a click at the same point/lock would resolve onto', () => {
    const cue = drawPlaneCue({
      anchoredPlane: null,
      anchoredThrough: null,
      idleLock: 0,
      idleHover: [4, 5, 6],
      frame: ROTATED_FRAME,
    })
    const clickPlane = axisDrawPlane(0, [4, 5, 6], ROTATED_FRAME)
    expect(cue).toEqual({ plane: clickPlane, through: [4, 5, 6] })
    // Axis 0 (red) of the moved frame is world [0,1,0], not world [1,0,0] —
    // so the un-fixed bug (defaulting to world identity) would have produced
    // a different plane here.
    expect(cue!.plane.normal).toEqual([0, 1, 0])
  })

  it('the literal-ground suppression only fires at world identity — a moved frame\'s blue axis through z=0 still cues', () => {
    // The frame's blue axis happens to be world +Z here (same as ground's
    // normal) and the hover sits at z=0 — but the frame itself is not world
    // identity, so this must NOT suppress to null (mirrors `axisDrawPlane`'s
    // own "never ground for a moved frame" rule).
    const cue = drawPlaneCue({
      anchoredPlane: null,
      anchoredThrough: null,
      idleLock: 2,
      idleHover: [1, 2, 0],
      frame: ROTATED_FRAME,
    })
    expect(cue).not.toBeNull()
    expect(cue!.plane.ground).toBe(false)
  })

  it('omitting `frame` still defaults to world identity (source-compatible with every existing caller)', () => {
    const withDefault = drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 0, idleHover: [4, 5, 6] })
    const withExplicitWorld = drawPlaneCue({
      anchoredPlane: null,
      anchoredThrough: null,
      idleLock: 0,
      idleHover: [4, 5, 6],
      frame: { origin: [0, 0, 0], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] },
    })
    expect(withDefault).toEqual(withExplicitWorld)
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

describe('drawPlaneCue — the idle hover plane (view-plane §5)', () => {
  const GREEN = axisDrawPlane(1, [0, 0, 0])
  const ray: Ray = { origin: [0.4, -5, 1], direction: [0, 1, -0.1] }
  const idle = { anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null } as const

  it('a patch on the inferred plane through the ray\'s hit on it', () => {
    const cue = drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null, idleHoverCue: { plane: GREEN, ray } })
    expect(cue?.plane).toBe(GREEN)
    expect(cue?.through[0]).toBeCloseTo(0.4, 12)
    expect(cue?.through[1]).toBeCloseTo(0, 12)
    expect(cue?.through[2]).toBeCloseTo(0.5, 12)
  })

  it('null for the ground (the world grid is the cue)', () => {
    const down: Ray = { origin: [0, -5, 3], direction: [0, 1, -1] }
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null, idleHoverCue: { plane: groundDrawPlane(), ray: down } })).toBeNull()
  })

  it('a moved frame\'s blue plane does cue (D1)', () => {
    const raised = inferredAxisPlane({ ...WORLD_DRAWING_AXES, origin: [0, 0, 0.75] }, HOME_VIEW)
    const down: Ray = { origin: [0, -5, 3], direction: [0, 1, -1] }
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null, idleHoverCue: { plane: raised, ray: down } })?.through[2]).toBeCloseTo(0.75, 12)
  })

  it('an active arrow lock beats it, with or without a hover', () => {
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 0, idleHover: null, idleHoverCue: { plane: GREEN, ray } })).toBeNull()
    const locked = drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 0, idleHover: [1, 2, 3], idleHoverCue: { plane: GREEN, ray } })
    expect(locked?.plane.normal).toEqual([1, 0, 0])
  })

  it('an anchored gesture beats it', () => {
    const anchored = axisDrawPlane(0, [1, 1, 1])
    expect(drawPlaneCue({ anchoredPlane: anchored, anchoredThrough: [1, 1, 1], idleLock: null, idleHover: null, idleHoverCue: { plane: GREEN, ray } })?.plane).toBe(anchored)
  })

  it('null when the ray never reaches the plane (parallel, or pointing away)', () => {
    const parallel: Ray = { origin: [0, -5, 1], direction: [1, 0, 0] }
    const away: Ray = { origin: [0, -5, 1], direction: [0, -1, 0] }
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null, idleHoverCue: { plane: GREEN, ray: parallel } })).toBeNull()
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null, idleHoverCue: { plane: GREEN, ray: away } })).toBeNull()
  })
  it('a hovered sketch\'s plane (+X through the origin): a patch through the ray\'s foot at x = 0, on the plane passed in', () => {
    const red = drawPlaneThrough([0, 0, 0], [1, 0, 0])!
    const crossing: Ray = { origin: [3, -3, 0.5], direction: [-3, 3.9, -0.13] }
    const cue = drawPlaneCue({ ...idle, idleHoverCue: { plane: red, ray: crossing } })
    expect(cue?.plane).toBe(red)
    expect(cue?.through[0]).toBeCloseTo(0, 12)
    expect(cue?.through[1]).toBeCloseTo(0.9, 12)
    expect(cue?.through[2]).toBeCloseTo(0.37, 12)
  })

  it('an arrow lock wins over a hovered sketch\'s plane', () => {
    const red = drawPlaneThrough([0, 0, 0], [1, 0, 0])!
    const locked = drawPlaneCue({ ...idle, idleLock: 1, idleHover: [1, 2, 3], idleHoverCue: { plane: red, ray } })
    expect(locked?.plane.normal).toEqual([0, 1, 0])
    expect(locked?.through).toEqual([1, 2, 3])
  })

  it('a tilted plane (a sloped face) still shows a patch, through the ray\'s foot on it', () => {
    const tilted = drawPlaneThrough([0, 0, 0], [0, -Math.SQRT1_2, Math.SQRT1_2])!
    const cue = drawPlaneCue({ ...idle, idleHoverCue: { plane: tilted, ray } })
    expect(cue?.plane).toBe(tilted)
    // On the plane: y = z.
    expect(cue!.through[1]).toBeCloseTo(cue!.through[2], 12)
    expect(cue!.through[0]).toBeCloseTo(0.4, 12)
  })
})

describe('idleHoverCue — which idle hovers show a patch (view-plane §5)', () => {
  const ray: Ray = { origin: [0, -5, 1], direction: [0, 1, 0] }
  const GREEN = axisDrawPlane(1, [0, 0, 0])

  it('a face shows a patch while PATCH_OVER_FACES is on', () => {
    expect(PATCH_OVER_FACES).toBe(true)
    expect(idleHoverCue({ plane: GREEN, onFace: true }, ray, false)).toEqual({ plane: GREEN, ray })
  })

  it('a sketch or the inferred plane shows a patch at the top level', () => {
    expect(idleHoverCue({ plane: GREEN }, ray, false)).toEqual({ plane: GREEN, ray })
  })

  it('inside an entered object only a face of it does: a click off it lands nowhere', () => {
    expect(idleHoverCue({ plane: GREEN }, ray, true)).toBeNull()
    expect(idleHoverCue({ plane: GREEN, onFace: true }, ray, true)).toEqual({ plane: GREEN, ray })
  })

  it('resolveIdleDrawTarget flags only an ineligible face as onFace', () => {
    const scene = {
      pick_sketch: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => undefined),
      axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    } as unknown as WasmScene
    const face = () => ({ point: [0, 0, 0.3] as V3, normal: [0, 0, 1] as V3, depth: 4 })
    expect(resolveIdleDrawTarget(scene, new SketchPickCache(), ray, TOP, face, '', FRONT_VIEW).onFace).toBe(true)
    expect(resolveIdleDrawTarget(scene, new SketchPickCache(), ray, TOP, undefined, '', FRONT_VIEW).onFace).toBeUndefined()
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
