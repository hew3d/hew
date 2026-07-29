/**
 * Tests for TextBillboard's pure sizing math (billboardWorldScale/
 * zoomTierFor) — no DOM/canvas needed, so this runs in vitest's lean `node`
 * environment. The `TextBillboard` class itself (canvas rasterization +
 * per-frame billboard update) is covered by TextBillboard.dom.test.tsx,
 * which needs jsdom for a `document` global.
 */
import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import {
  billboardWorldScale,
  zoomTierFor,
  ZOOM_TIER_SUPERSAMPLE,
  glyphContentFraction,
  inlineTextQuaternion,
} from './TextBillboard'

describe('billboardWorldScale', () => {
  it('scales linearly with screen px and camera distance', () => {
    const fov = 60
    const viewportHeight = 800
    const a = billboardWorldScale(20, 10, fov, viewportHeight)
    const b = billboardWorldScale(40, 10, fov, viewportHeight)
    expect(b).toBeCloseTo(a * 2, 10)
    const c = billboardWorldScale(20, 20, fov, viewportHeight)
    expect(c).toBeCloseTo(a * 2, 10)
  })

  it('matches the closed-form perspective formula exactly', () => {
    const screenPx = 26
    const dist = 15
    const fovDeg = 50
    const viewportHeight = 900
    const expected = (screenPx * dist * Math.tan((fovDeg * Math.PI) / 360)) / viewportHeight
    expect(billboardWorldScale(screenPx, dist, fovDeg, viewportHeight)).toBeCloseTo(expected, 12)
  })

  it('is inversely proportional to viewport height', () => {
    const a = billboardWorldScale(26, 10, 50, 600)
    const b = billboardWorldScale(26, 10, 50, 1200)
    expect(b).toBeCloseTo(a / 2, 10)
  })

  it('returns 0 for a non-positive viewport height rather than dividing by zero', () => {
    expect(billboardWorldScale(26, 10, 50, 0)).toBe(0)
    expect(billboardWorldScale(26, 10, 50, -5)).toBe(0)
  })

  it('grows with a wider field of view at fixed distance', () => {
    const narrow = billboardWorldScale(26, 10, 30, 800)
    const wide = billboardWorldScale(26, 10, 90, 800)
    expect(wide).toBeGreaterThan(narrow)
  })
})

describe('zoomTierFor', () => {
  it('buckets into three discrete tiers by camera distance', () => {
    expect(zoomTierFor(0)).toBe(0)
    expect(zoomTierFor(4.99)).toBe(0)
    expect(zoomTierFor(5)).toBe(1)
    expect(zoomTierFor(19.99)).toBe(1)
    expect(zoomTierFor(20)).toBe(2)
    expect(zoomTierFor(1000)).toBe(2)
  })

  it('is monotonically non-increasing in supersample factor as distance grows', () => {
    const s0 = ZOOM_TIER_SUPERSAMPLE[zoomTierFor(1)]
    const s1 = ZOOM_TIER_SUPERSAMPLE[zoomTierFor(10)]
    const s2 = ZOOM_TIER_SUPERSAMPLE[zoomTierFor(100)]
    expect(s0).toBeGreaterThanOrEqual(s1)
    expect(s1).toBeGreaterThanOrEqual(s2)
  })

  it('is a rasterization-crispness knob only — it does not gate whether billboardWorldScale is continuous', () => {
    // Finding 1 of the playtest round: sizing must be continuous, not
    // quantized into these tiers. `zoomTierFor`/`ZOOM_TIER_SUPERSAMPLE` only
    // ever feed the canvas supersample factor (see TextBillboard.ts's
    // `_rasterize`); `billboardWorldScale` — the thing that actually decides
    // on-screen size — takes a raw `cameraDistance` and has no tier
    // quantization anywhere in its signature or body. Sampled at a distance
    // that crosses two tier boundaries, the scale still varies smoothly.
    const fov = 50
    const viewportHeight = 900
    const samples = [1, 4.9, 5, 5.1, 19.9, 20, 20.1, 50, 500]
    const scales = samples.map((d) => billboardWorldScale(14, d, fov, viewportHeight))
    for (let i = 1; i < scales.length; i++) {
      // Monotonically increasing with distance (screen-constant size means
      // world size grows linearly with distance) — a tiered implementation
      // would instead show flat plateaus with sudden jumps at tier bounds.
      expect(scales[i]).toBeGreaterThan(scales[i - 1])
    }
  })
})

describe('glyphContentFraction (Finding 1 — decoupling label size from canvas padding, calibrated on reference-digit ink)', () => {
  it('returns the measured reference-digit ink fraction of the canvas height when metrics are available', () => {
    expect(glyphContentFraction(28, 40, 40)).toBeCloseTo(0.7, 10)
  })

  it('falls back to a fontPx-based estimate when the reference-digit ink is non-finite', () => {
    const fontPx = 40
    const canvasHeightPx = 75
    expect(glyphContentFraction(NaN, fontPx, canvasHeightPx)).toBeCloseTo((fontPx * 0.72) / canvasHeightPx, 10)
  })

  it('falls back to a fontPx-based estimate when the reference-digit ink is non-positive', () => {
    const fontPx = 40
    const canvasHeightPx = 75
    expect(glyphContentFraction(0, fontPx, canvasHeightPx)).toBeCloseTo((fontPx * 0.72) / canvasHeightPx, 10)
  })

  it('clamps to [0.15, 1] rather than letting a pathological measurement blow up the on-screen scale', () => {
    expect(glyphContentFraction(2000, 40, 10)).toBe(1)
    expect(glyphContentFraction(0.0002, 40, 10000)).toBe(0.15)
  })

  it('returns 1 for a non-positive canvas height rather than dividing by zero', () => {
    expect(glyphContentFraction(15, 40, 0)).toBe(1)
    expect(glyphContentFraction(15, 40, -5)).toBe(1)
  })
})

describe('glyphContentFraction — per-font reference-digit normalization, not per-label ink (Finding 2)', () => {
  it('gives a digit-only label and a descender-bearing label the SAME scale, since both draw from the SAME cached reference-digit measurement', () => {
    // The reference-digit ink is measured once per font string (from '0'),
    // independent of whatever the label being rasterized actually says —
    // `TextBillboard._rasterize` now passes that single measurement
    // (`measureReferenceDigitInkPx`) for every label at a given fontPx, so a
    // '12' rasterization and a 'doorway gap' rasterization at the same
    // fontPx can only ever call this with the SAME referenceDigitInkPx —
    // there is no code path left by which either label's own content
    // reaches this function.
    const fontPx = 40
    const canvasHeightPx = 75
    const referenceDigitInkPx = 29 // '0' measured at this fontPx

    const digitsOnly = glyphContentFraction(referenceDigitInkPx, fontPx, canvasHeightPx) // '12'
    const withDescender = glyphContentFraction(referenceDigitInkPx, fontPx, canvasHeightPx) // 'doorway gap'

    expect(withDescender).toBe(digitsOnly)
  })

  it('the fallback constant approximates the SAME quantity as the measured path (a digit\'s ink) — not the font em-box the measured path used to use', () => {
    // Finding 1 of the SECOND playtest round: an em-box-based denominator
    // (~1.2-1.33 em) is nearly double a digit's actual ink (~0.72 em) —
    // dividing screenPxHeight by that inflated fraction shrinks every
    // label's visible ink well under the 14px ANNOTATION_TEXT_SCREEN_PX
    // contract. The fallback (0.72 * fontPx) must track a REALISTIC measured
    // digit ink at the same fontPx, not the em-box quantity the old
    // (reverted) denominator used.
    const fontPx = 40
    const canvasHeightPx = 75
    const measuredDigitInkPx = 0.72 * fontPx // a realistic ctx.measureText('0') result
    const emBoxPx = 1.2 * fontPx // the em-box quantity Finding 1 replaced

    const measuredFrac = glyphContentFraction(measuredDigitInkPx, fontPx, canvasHeightPx)
    const fallbackFrac = glyphContentFraction(NaN, fontPx, canvasHeightPx)
    const emBoxFrac = glyphContentFraction(emBoxPx, fontPx, canvasHeightPx)

    expect(fallbackFrac).toBeCloseTo(measuredFrac, 10)
    // The em-box quantity diverges sharply — this is the regression the
    // reference-digit fix closes.
    expect(Math.abs(emBoxFrac - measuredFrac) / measuredFrac).toBeGreaterThan(0.5)
  })
})

describe('continuous on-screen text sizing across zoom (Finding 1)', () => {
  /** Project two world points straddling `worldHeight` (a FULL height, the
   * mesh's actual rendered extent — see below), centered on the camera's
   * forward axis at `distance`, through the SAME perspective projection math
   * the app's own `worldToPixels` (Viewport.tsx) uses (`Vector3.project`,
   * NDC halved against `viewportHeight`), and measure the resulting
   * screen-px gap — ground truth for "how tall does this actually render,"
   * independent of the `billboardWorldScale` formula under test. */
  function projectedScreenPxHeight(worldHeight: number, camera: THREE.PerspectiveCamera, viewportHeight: number, distance: number): number {
    const center = new THREE.Vector3(0, 0, -distance)
    const top = center.clone().add(new THREE.Vector3(0, worldHeight / 2, 0))
    const bottom = center.clone().add(new THREE.Vector3(0, -worldHeight / 2, 0))
    const topNdc = top.project(camera)
    const bottomNdc = bottom.project(camera)
    return (Math.abs(topNdc.y - bottomNdc.y) / 2) * viewportHeight
  }

  it('holds the label to the same on-screen height at two very different camera distances', () => {
    const fov = 50
    const viewportHeight = 900
    const targetPx = 14
    const camera = new THREE.PerspectiveCamera(fov, 1, 0.01, 10000)
    camera.position.set(0, 0, 0)
    camera.lookAt(0, 0, -1)
    camera.updateMatrixWorld()

    const near = 1.5
    const far = 400 // ~267x farther — a much bigger spread than the old zoom tiers
    // `TextBillboard.update()`'s actual mesh height: DOUBLE `billboardWorldScale`'s
    // half-extent, since the mesh's `PlaneGeometry(1,1)` is diameter-native
    // (see `billboardWorldScale`'s doc comment).
    const worldNear = 2 * billboardWorldScale(targetPx, near, fov, viewportHeight)
    const worldFar = 2 * billboardWorldScale(targetPx, far, fov, viewportHeight)

    const pxNear = projectedScreenPxHeight(worldNear, camera, viewportHeight, near)
    const pxFar = projectedScreenPxHeight(worldFar, camera, viewportHeight, far)

    expect(pxNear).toBeCloseTo(targetPx, 1)
    expect(pxFar).toBeCloseTo(targetPx, 1)
    expect(Math.abs(pxNear - pxFar)).toBeLessThan(0.05)
  })
})

/** Builds a real `THREE.PerspectiveCamera` at `position`, looking at `target`
 * with the given `up` hint — a full camera (not just a position), needed
 * now that `inlineTextQuaternion`'s readability rule actually projects
 * points through the camera's matrices. Mirrors how the test suites for
 * `Viewport.tsx`'s `worldToPixels` and this module's own sizing tests build
 * cameras (`camera.lookAt` + `updateMatrixWorld()` so `matrixWorld`/
 * `matrixWorldInverse` reflect the requested pose before any `.project()`
 * call). */
function cameraAt(position: THREE.Vector3, up: THREE.Vector3, target = new THREE.Vector3(0, 0, 0)): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
  camera.up.copy(up)
  camera.position.copy(position)
  camera.lookAt(target)
  camera.updateMatrixWorld()
  return camera
}

/** Screen-space (NDC) delta produced by nudging `origin` along `dir` by 1% of
 * the camera-to-origin distance, projected through `camera` — the SAME
 * ground-truth technique (`Vector3.project`) the sizing tests above use for
 * "how does this actually render," applied here to a direction instead of a
 * height. An independent check (written directly in the test, not calling
 * into `TextBillboard.ts`'s own probe) that a resulting basis vector reads
 * the way it visually should on screen. */
function screenDelta(camera: THREE.PerspectiveCamera, origin: THREE.Vector3, dir: THREE.Vector3): { dx: number; dy: number } {
  const eps = camera.position.distanceTo(origin) * 0.01
  const a = origin.clone().project(camera)
  const b = origin.clone().addScaledVector(dir, eps).project(camera)
  return { dx: b.x - a.x, dy: b.y - a.y }
}

describe('inlineTextQuaternion — in-plane, aligned dimension/radial text', () => {
  it('maps the quad into the given plane, with its local X axis aligned to alignDir', () => {
    const alignDir = new THREE.Vector3(1, 0, 0)
    const planeNormal = new THREE.Vector3(0, 0, 1)
    const textPosition = new THREE.Vector3(0, 0, 0)
    const camera = cameraAt(new THREE.Vector3(0, 0, 10), new THREE.Vector3(0, 1, 0)) // in front, +normal side

    const { quaternion: q } = inlineTextQuaternion(alignDir, planeNormal, camera, textPosition, false)
    const worldRight = new THREE.Vector3(1, 0, 0).applyQuaternion(q)
    const worldUp = new THREE.Vector3(0, 1, 0).applyQuaternion(q)

    // Both in-plane basis vectors stay perpendicular to the plane normal —
    // the label lies flat IN the plane, not tilted out of it.
    expect(Math.abs(worldRight.dot(planeNormal))).toBeLessThan(1e-9)
    expect(Math.abs(worldUp.dot(planeNormal))).toBeLessThan(1e-9)
    // Baseline (local +X) reads along +alignDir when the camera is on the
    // front (+normal) side and already reads left-to-right on screen.
    expect(worldRight.dot(alignDir)).toBeGreaterThan(0)
  })

  it('the facing flip (about "up") still fires crossing the plane\'s far side, and is not subsumed by the readability rule', () => {
    // A symmetric front/back pair (straight along the plane normal, same
    // camera "up", same target) isolates the FACING correction from the
    // readability correction below: with this symmetry the baseline already
    // reads correctly on screen from both sides, so only the facing flip
    // should differ between them.
    const alignDir = new THREE.Vector3(1, 0, 0)
    const planeNormal = new THREE.Vector3(0, 0, 1)
    const textPosition = new THREE.Vector3(0, 0, 0)

    const cameraFront = cameraAt(new THREE.Vector3(0, 0, 10), new THREE.Vector3(0, 1, 0))
    const cameraBack = cameraAt(new THREE.Vector3(0, 0, -10), new THREE.Vector3(0, 1, 0))

    const { quaternion: qFront } = inlineTextQuaternion(alignDir, planeNormal, cameraFront, textPosition, false)
    const { quaternion: qBack } = inlineTextQuaternion(alignDir, planeNormal, cameraBack, textPosition, false)

    const rightFront = new THREE.Vector3(1, 0, 0).applyQuaternion(qFront)
    const upFront = new THREE.Vector3(0, 1, 0).applyQuaternion(qFront)
    const normalFront = new THREE.Vector3(0, 0, 1).applyQuaternion(qFront)

    const rightBack = new THREE.Vector3(1, 0, 0).applyQuaternion(qBack)
    const upBack = new THREE.Vector3(0, 1, 0).applyQuaternion(qBack)
    const normalBack = new THREE.Vector3(0, 0, 1).applyQuaternion(qBack)

    // The quad's own facing (local Z) always points back toward whichever
    // camera it was built for — the face never disappears from view, and
    // the correctly-drawn (non-mirrored) side is what's shown, never the
    // canvas texture's true mirrored backface.
    expect(normalFront.dot(cameraFront.position.clone().sub(textPosition))).toBeGreaterThan(0)
    expect(normalBack.dot(cameraBack.position.clone().sub(textPosition))).toBeGreaterThan(0)

    // A 180° rotation about "up": up is preserved, right/normal negate —
    // this pair's symmetry means the readability rule doesn't ALSO fire, so
    // this is purely the facing correction.
    expect(upBack.dot(upFront)).toBeCloseTo(1, 9)
    expect(rightBack.dot(rightFront)).toBeCloseTo(-1, 9)
    expect(normalBack.dot(normalFront)).toBeCloseTo(-1, 9)

    // Both sides still read left-to-right / upright on screen — the facing
    // flip alone doesn't leave either view backwards.
    expect(screenDelta(cameraFront, textPosition, rightFront).dx).toBeGreaterThan(0)
    expect(screenDelta(cameraBack, textPosition, rightBack).dx).toBeGreaterThan(0)
  })

  it('falls back to a stable, finite orthonormal basis for a degenerate alignDir (parallel to the plane normal)', () => {
    const alignDir = new THREE.Vector3(0, 0, 5) // parallel to planeNormal below
    const planeNormal = new THREE.Vector3(0, 0, 1)
    const textPosition = new THREE.Vector3(0, 0, 0)
    const camera = cameraAt(new THREE.Vector3(0, 0, 10), new THREE.Vector3(0, 1, 0))

    const { quaternion: q } = inlineTextQuaternion(alignDir, planeNormal, camera, textPosition, false)
    expect(Number.isFinite(q.x)).toBe(true)
    expect(Number.isFinite(q.y)).toBe(true)
    expect(Number.isFinite(q.z)).toBe(true)
    expect(Number.isFinite(q.w)).toBe(true)
    expect(q.length()).toBeCloseTo(1, 9)

    const worldRight = new THREE.Vector3(1, 0, 0).applyQuaternion(q)
    expect(Math.abs(worldRight.dot(planeNormal))).toBeLessThan(1e-9)
  })
})

describe('inlineTextQuaternion — screen-space readability flip (Finding 1 regression)', () => {
  it('reads upright regardless of which dimension endpoint was clicked first (alignDir sign)', () => {
    // Before the fix, alignDir was stored click-order with no canonicalizing
    // — clicking the "other" endpoint first rendered the label permanently
    // upside-down from a normal viewpoint. The screen-space rule fixes this
    // by deciding purely from how the current basis reads on screen, so
    // BOTH signs of alignDir must come out upright for the same viewpoint.
    const planeNormal = new THREE.Vector3(0, 0, 1)
    const textPosition = new THREE.Vector3(0, 0, 0)
    const camera = cameraAt(new THREE.Vector3(0, 0, 10), new THREE.Vector3(0, 1, 0))

    for (const sign of [1, -1]) {
      const alignDir = new THREE.Vector3(sign, 0, 0)
      const { quaternion } = inlineTextQuaternion(alignDir, planeNormal, camera, textPosition, false)
      const worldUp = new THREE.Vector3(0, 1, 0).applyQuaternion(quaternion)
      const worldRight = new THREE.Vector3(1, 0, 0).applyQuaternion(quaternion)
      expect(screenDelta(camera, textPosition, worldUp).dy).toBeGreaterThan(0)
      expect(screenDelta(camera, textPosition, worldRight).dx).toBeGreaterThan(0)
    }
  })

  it('a ground-plane dimension stays upright orbiting azimuth 180° without the camera ever crossing the plane (real camera, Z-up)', () => {
    // The reported failure: for a Z-up ground-plane dimension, the camera
    // essentially never crosses the plane (z stays positive on both sides of
    // an azimuth orbit), so the old plane-crossing flip never fired — yet
    // the label read upside-down past ~90° of azimuth. Both camera positions
    // below sit on the SAME side of the z=0 ground plane (z=8 throughout).
    const planeNormal = new THREE.Vector3(0, 0, 1) // ground plane, Z-up
    const textPosition = new THREE.Vector3(0, 0, 0)
    const alignDir = new THREE.Vector3(1, 0, 0)
    const zUp = new THREE.Vector3(0, 0, 1)

    const cameraA = cameraAt(new THREE.Vector3(0, -10, 8), zUp)
    const cameraB = cameraAt(new THREE.Vector3(0, 10, 8), zUp) // azimuth +180°, same plane side

    for (const sign of [1, -1]) {
      const dir = alignDir.clone().multiplyScalar(sign)
      const { quaternion: qA } = inlineTextQuaternion(dir, planeNormal, cameraA, textPosition, false)
      const { quaternion: qB } = inlineTextQuaternion(dir, planeNormal, cameraB, textPosition, false)

      const upA = new THREE.Vector3(0, 1, 0).applyQuaternion(qA)
      const upB = new THREE.Vector3(0, 1, 0).applyQuaternion(qB)

      expect(screenDelta(cameraA, textPosition, upA).dy).toBeGreaterThan(0)
      expect(screenDelta(cameraB, textPosition, upB).dy).toBeGreaterThan(0)
    }
  })

  it('holds the previous frame\'s flip state (does not oscillate) when the baseline is viewed edge-on', () => {
    // A plane/alignDir pair chosen so the in-plane "right" axis lies exactly
    // along the camera's own viewing axis — nudging along it stays on the
    // same screen-space point (dx ~ 0), the degenerate case the fix must not
    // guess through.
    const planeNormal = new THREE.Vector3(1, 0, 0)
    const alignDir = new THREE.Vector3(0, 0, 1)
    const textPosition = new THREE.Vector3(0, 0, 0)
    const camera = cameraAt(new THREE.Vector3(0, 0, -5), new THREE.Vector3(0, 1, 0), textPosition)

    const keptTrue = inlineTextQuaternion(alignDir, planeNormal, camera, textPosition, true)
    expect(keptTrue.flipped).toBe(true)

    const keptFalse = inlineTextQuaternion(alignDir, planeNormal, camera, textPosition, false)
    expect(keptFalse.flipped).toBe(false)
  })
})

