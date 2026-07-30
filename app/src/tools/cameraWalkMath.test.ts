import { describe, it, expect } from 'vitest'
import {
  DEFAULT_EYE_HEIGHT_M,
  PITCH_CLAMP_DEG,
  headingFromForward,
  forwardFromYawPitch,
  positionCameraClick,
  positionCameraDrag,
  lookAroundDrag,
  walkDrag,
  horizontalBasis,
  type V3,
} from './cameraWalkMath'

const DEG = Math.PI / 180

describe('headingFromForward', () => {
  it('extracts the compass yaw regardless of pitch', () => {
    // Facing +X, level.
    expect(headingFromForward([1, 0, 0])).toBeCloseTo(0, 12)
    // Facing +Y, level.
    expect(headingFromForward([0, 1, 0])).toBeCloseTo(Math.PI / 2, 12)
    // Facing +X but pitched steeply up — yaw is unaffected by pitch.
    expect(headingFromForward([1, 0, 5])).toBeCloseTo(0, 12)
    expect(headingFromForward([0, 2, -5])).toBeCloseTo(Math.PI / 2, 12)
  })

  it('is heading-preserving: a forward built from yaw/pitch round-trips its yaw exactly', () => {
    for (const yawDeg of [0, 30, 90, 137, 200, 359]) {
      for (const pitchDeg of [-80, -30, 0, 45, 80]) {
        const yaw = yawDeg * DEG
        const pitch = pitchDeg * DEG
        const forward = forwardFromYawPitch(yaw, pitch)
        const recovered = headingFromForward(forward)
        // atan2 wraps to (-pi, pi]; compare via the unit circle, not the raw angle.
        expect(Math.cos(recovered)).toBeCloseTo(Math.cos(yaw), 9)
        expect(Math.sin(recovered)).toBeCloseTo(Math.sin(yaw), 9)
      }
    }
  })

  it('returns 0 (not NaN) for a forward pointing straight up or down (no meaningful heading)', () => {
    expect(headingFromForward([0, 0, 1])).toBe(0)
    expect(headingFromForward([0, 0, -1])).toBe(0)
    expect(headingFromForward([1e-15, -1e-15, 1])).toBe(0)
  })
})

describe('forwardFromYawPitch', () => {
  it('yaw=0, pitch=0 is +X, level', () => {
    const f = forwardFromYawPitch(0, 0)
    expect(f[0]).toBeCloseTo(1, 12)
    expect(f[1]).toBeCloseTo(0, 12)
    expect(f[2]).toBeCloseTo(0, 12)
  })

  it('is always a unit vector', () => {
    for (const yawDeg of [0, 45, 123, 300]) {
      for (const pitchDeg of [-89, -45, 0, 45, 89]) {
        const f = forwardFromYawPitch(yawDeg * DEG, pitchDeg * DEG)
        const len = Math.hypot(f[0], f[1], f[2])
        expect(len).toBeCloseTo(1, 12)
      }
    }
  })

  it('clamps pitch to ±PITCH_CLAMP_DEG — an extreme request never reaches exactly vertical', () => {
    const straightUp = forwardFromYawPitch(0, Math.PI / 2)
    const straightDown = forwardFromYawPitch(0, -Math.PI / 2)
    const clampSin = Math.sin(PITCH_CLAMP_DEG * DEG)
    expect(straightUp[2]).toBeCloseTo(clampSin, 9)
    expect(straightDown[2]).toBeCloseTo(-clampSin, 9)
    expect(Math.abs(straightUp[2])).toBeLessThan(1)
  })

  it('positive pitch looks up (+Z component grows)', () => {
    const level = forwardFromYawPitch(0, 0)
    const up = forwardFromYawPitch(0, 30 * DEG)
    expect(up[2]).toBeGreaterThan(level[2])
  })
})

describe('positionCameraClick', () => {
  it('places the eye eyeHeight above the click point', () => {
    const click: V3 = [3, 4, 0.5]
    const { eye } = positionCameraClick(click, DEFAULT_EYE_HEIGHT_M, [1, 0, 0])
    expect(eye[0]).toBeCloseTo(3, 12)
    expect(eye[1]).toBeCloseTo(4, 12)
    expect(eye[2]).toBeCloseTo(0.5 + DEFAULT_EYE_HEIGHT_M, 12)
  })

  it('looks horizontally (pitch 0) along the pre-click heading, ignoring the pre-click pitch', () => {
    const preClickForward: V3 = [0, 1, 5] // steeply pitched up, heading +Y
    const { forward } = positionCameraClick([0, 0, 0], 1.68, preClickForward)
    expect(forward[2]).toBeCloseTo(0, 9)
    expect(headingFromForward(forward)).toBeCloseTo(Math.PI / 2, 9)
  })
})

describe('positionCameraDrag', () => {
  it('eye sits eyeHeight above the press point', () => {
    const press: V3 = [1, 1, 0]
    const release: V3 = [5, 1, 0]
    const { eye } = positionCameraDrag(press, release, 1.68, [1, 0, 0])
    expect(eye).toEqual([1, 1, 1.68])
  })

  it('looks toward the release point in true 3D, not flattened to horizontal', () => {
    const press: V3 = [0, 0, 0]
    const release: V3 = [0, 0, 10] // straight up from the eye
    const { eye, forward } = positionCameraDrag(press, release, 1.68, [1, 0, 0])
    const toRelease = [release[0] - eye[0], release[1] - eye[1], release[2] - eye[2]]
    const lenToRelease = Math.hypot(...(toRelease as [number, number, number]))
    expect(forward[0]).toBeCloseTo(toRelease[0] / lenToRelease, 9)
    expect(forward[1]).toBeCloseTo(toRelease[1] / lenToRelease, 9)
    expect(forward[2]).toBeCloseTo(toRelease[2] / lenToRelease, 9)
  })

  it('press and release at the same ground point looks straight down (a well-defined 3D direction, not degenerate — the eye sits eyeHeight above the shared point)', () => {
    const p: V3 = [2, 2, 0]
    const { forward } = positionCameraDrag(p, p, 1.68, [0, 1, 0])
    expect(forward).toEqual([0, 0, -1])
  })

  it('falls back to the pre-click heading, level, ONLY for the genuinely zero-length case (eye height 0 too, so eye == release)', () => {
    const p: V3 = [2, 2, 0]
    const { forward } = positionCameraDrag(p, p, 0, [0, 1, 0])
    expect(forward[2]).toBeCloseTo(0, 9)
    expect(headingFromForward(forward)).toBeCloseTo(Math.PI / 2, 9)
  })
})

describe('lookAroundDrag', () => {
  it('dragging right DECREASES yaw (turns the view right, screen-space ground-truthed below)', () => {
    const { yawRad } = lookAroundDrag(0, 0, 100, 0, 0.01)
    expect(yawRad).toBeCloseTo(-1, 9)
  })

  it('dragging left increases yaw (turns left)', () => {
    const { yawRad } = lookAroundDrag(0, 0, -100, 0, 0.01)
    expect(yawRad).toBeCloseTo(1, 9)
  })

  it('dragging up (negative dyPx) increases pitch (looks up)', () => {
    const { pitchRad } = lookAroundDrag(0, 0, 0, -100, 0.01)
    expect(pitchRad).toBeCloseTo(1, 9)
  })

  it('dragging down decreases pitch (looks down)', () => {
    const { pitchRad } = lookAroundDrag(0, 0, 0, 100, 0.01)
    expect(pitchRad).toBeCloseTo(-1, 9)
  })

  it('clamps pitch to ±PITCH_CLAMP_DEG regardless of how far the drag goes', () => {
    const clampRad = PITCH_CLAMP_DEG * DEG
    const { pitchRad: up } = lookAroundDrag(0, 0, 0, -1_000_000, 0.01)
    const { pitchRad: down } = lookAroundDrag(0, 0, 0, 1_000_000, 0.01)
    expect(up).toBeCloseTo(clampRad, 9)
    expect(down).toBeCloseTo(-clampRad, 9)
  })

  it('yaw is unbounded — many full turns accumulate rather than wrapping or clamping', () => {
    const { yawRad } = lookAroundDrag(0, 0, 100_000, 0, 0.01)
    expect(yawRad).toBeCloseTo(-1000, 9)
  })
})

/**
 * SCREEN-SPACE GROUND TRUTH for the turn direction (CRITICAL fix, camera P2
 * review): the tests above pin the SIGN of the internal yaw delta, but that
 * internal convention is exactly what was wrong before this fix (the old
 * code also "consistently" turned yaw positive for a rightward drag — it was
 * just turning the camera the wrong way on screen). This block instead
 * builds an actual camera projection from this module's own functions and
 * checks what a real drag does to a real point's SCREEN position, which is
 * the one thing a sign convention can't hide behind: dragging right must
 * turn the view right, which swings the rest of the world LEFT under the
 * cursor (exactly like grabbing and dragging a photograph of the scene).
 */
describe('lookAroundDrag / walkDrag — screen-space ground truth (turn direction)', () => {
  /** A minimal pinhole-camera projection (right-handed, +Z up, matching
   * this module's own convention) — just enough to turn an eye/forward/up
   * triple and a world point into a screen-space X coordinate, with no
   * dependency on three.js or the app's real render pipeline. */
  function screenX(eye: V3, forward: V3, point: V3): number {
    const f = forward
    const worldUp: V3 = [0, 0, 1]
    // right = forward x worldUp (then normalize) — a standard camera basis;
    // this is independently derived from `horizontalBasis`, not reused from
    // it, so the test doesn't just check the math against itself.
    const rightRaw: V3 = [
      f[1] * worldUp[2] - f[2] * worldUp[1],
      f[2] * worldUp[0] - f[0] * worldUp[2],
      f[0] * worldUp[1] - f[1] * worldUp[0],
    ]
    const rightLen = Math.hypot(rightRaw[0], rightRaw[1], rightRaw[2])
    const right: V3 = [rightRaw[0] / rightLen, rightRaw[1] / rightLen, rightRaw[2] / rightLen]
    const toPoint: V3 = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]]
    const depth = toPoint[0] * f[0] + toPoint[1] * f[1] + toPoint[2] * f[2]
    const across = toPoint[0] * right[0] + toPoint[1] * right[1] + toPoint[2] * right[2]
    // Perspective divide: screen-X grows with `across/depth` (a point to the
    // camera's right of the view axis renders on the right of the frame).
    return across / depth
  }

  it('lookAroundDrag: dragging right moves a straight-ahead reference point LEFT on screen (the view turned right)', () => {
    const eye: V3 = [0, 0, 0]
    const forward0 = forwardFromYawPitch(0, 0) // facing +X, level
    const reference: V3 = [10, 0, 0] // straight ahead — screenX(before) === 0
    expect(screenX(eye, forward0, reference)).toBeCloseTo(0, 9)

    const { yawRad, pitchRad } = lookAroundDrag(0, 0, 50, 0, 0.01) // drag right
    const forward1 = forwardFromYawPitch(yawRad, pitchRad)
    expect(screenX(eye, forward1, reference)).toBeLessThan(0) // swung left
  })

  it('lookAroundDrag: dragging left moves the same reference point RIGHT on screen', () => {
    const eye: V3 = [0, 0, 0]
    const reference: V3 = [10, 0, 0]
    const { yawRad, pitchRad } = lookAroundDrag(0, 0, -50, 0, 0.01) // drag left
    const forward1 = forwardFromYawPitch(yawRad, pitchRad)
    expect(screenX(eye, forward1, reference)).toBeGreaterThan(0) // swung right
  })

  it('walkDrag (no Shift): the horizontal-drag turn matches the same screen-space convention as lookAroundDrag', () => {
    const eye: V3 = [0, 0, 0]
    const reference: V3 = [10, 0, 0]
    const forward0 = forwardFromYawPitch(0, 0)
    expect(screenX(eye, forward0, reference)).toBeCloseTo(0, 9)

    const delta = walkDrag(50, 0, 0.01, 0.02, false) // drag right, no Shift
    const forward1 = forwardFromYawPitch(delta.yawDeltaRad, 0)
    expect(screenX(eye, forward1, reference)).toBeLessThan(0) // swung left, same as Look Around
  })
})

describe('walkDrag', () => {
  it('without Shift: vertical delta walks forward/back, horizontal delta turns (right drag = negative yaw delta, screen-space ground-truthed above)', () => {
    const d = walkDrag(50, -20, 0.01, 0.02, false)
    expect(d.yawDeltaRad).toBeCloseTo(-0.5, 9)
    expect(d.forwardDeltaM).toBeCloseTo(0.4, 9) // dyPx=-20 (drag up) -> forward
    expect(d.strafeDeltaM).toBe(0)
    expect(d.heightDeltaM).toBe(0)
  })

  it('dragging down (positive dyPx) walks backward', () => {
    const d = walkDrag(0, 30, 0.01, 0.02, false)
    expect(d.forwardDeltaM).toBeCloseTo(-0.6, 9)
  })

  it('with Shift: horizontal delta strafes, vertical delta changes eye height', () => {
    const d = walkDrag(50, -20, 0.01, 0.02, true)
    expect(d.yawDeltaRad).toBe(0)
    expect(d.forwardDeltaM).toBe(0)
    expect(d.strafeDeltaM).toBeCloseTo(1.0, 9)
    expect(d.heightDeltaM).toBeCloseTo(0.4, 9) // dyPx=-20 (drag up) -> raises the eye
  })

  it('Shift + dragging down lowers the eye', () => {
    const d = walkDrag(0, 40, 0.01, 0.02, true)
    expect(d.heightDeltaM).toBeCloseTo(-0.8, 9)
  })
})

describe('horizontalBasis', () => {
  it('yaw=0: forward is +X, right is +Y... actually right is -Y-ish per the rotation convention — assert against forwardFromYawPitch directly', () => {
    const { forward, right } = horizontalBasis(0)
    const levelForward = forwardFromYawPitch(0, 0)
    expect(forward[0]).toBeCloseTo(levelForward[0], 12)
    expect(forward[1]).toBeCloseTo(levelForward[1], 12)
    expect(forward[2]).toBe(0)
    // right is perpendicular to forward in the XY plane, and unit length.
    expect(forward[0] * right[0] + forward[1] * right[1]).toBeCloseTo(0, 12)
    expect(Math.hypot(right[0], right[1], right[2])).toBeCloseTo(1, 12)
  })

  it('forward always matches forwardFromYawPitch(yaw, 0) horizontally', () => {
    for (const yawDeg of [0, 37, 90, 180, 271]) {
      const yaw = yawDeg * DEG
      const { forward } = horizontalBasis(yaw)
      const expected = forwardFromYawPitch(yaw, 0)
      expect(forward[0]).toBeCloseTo(expected[0], 9)
      expect(forward[1]).toBeCloseTo(expected[1], 9)
    }
  })

  it('right and forward are always perpendicular unit vectors in the XY plane', () => {
    for (const yawDeg of [10, 95, 210, 333]) {
      const { forward, right } = horizontalBasis(yawDeg * DEG)
      expect(forward[0] * right[0] + forward[1] * right[1]).toBeCloseTo(0, 9)
      expect(right[2]).toBe(0)
      expect(Math.hypot(right[0], right[1])).toBeCloseTo(1, 9)
    }
  })
})
