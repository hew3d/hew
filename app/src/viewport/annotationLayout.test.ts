/**
 * annotationLayout tests — pure geometry backing the playtest-round-2
 * dimension fixes (docs/design/dimensions-playtest2.md) and the playtest-2
 * DELTA review that followed it. This module is new in this round, so there
 * is no "unfixed code" to red-check these against — the red-check for this
 * round's findings lives in the tests that exercise the OLD buggy call sites
 * (DimensionTool.test.ts, SceneRenderer.test.ts).
 *
 * `annotationViewBiasVector`/`ANNOTATION_BIAS_MAX_WORLD` (the §1 world-space
 * depth-bias nudge) used to be tested here. The DELTA review found that
 * mechanism geometrically broken at grazing/edge-on incidence — no cap size
 * could fix it, because a view-direction nudge has almost no component
 * perpendicular to a surface viewed edge-on, regardless of magnitude — and
 * it was replaced with a modest `glPolygonOffset` on the annotation
 * materials (`depthPolicy.ts`'s `DEPTH_BIAS.ANNOTATION`, applied in
 * `SceneRenderer.ts`), which is exercised end-to-end (real rendered pixels,
 * not just the geometry math) by `app/e2e/dimensions-depth.spec.ts` — there
 * is no pure-math equivalent to unit-test here since a `polygonOffset`'s
 * effect only exists at rasterization.
 */
import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import {
  computeLineLabelLayout,
  screenRoomBeyond,
  pickOutsideEnd,
  axisDimensionPlane,
  lockedDimensionPlaneNormal,
  antipodalTolerance,
  distPointToLine,
  chordPassesNearCentre,
  buildRadialGeometry,
  pushCenterTick,
} from './annotationLayout'

function perspCamera(pos: [number, number, number], target: [number, number, number], fov = 45): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(fov, 1, 0.1, 1000)
  cam.position.set(...pos)
  cam.lookAt(...target)
  cam.updateMatrixWorld(true)
  cam.updateProjectionMatrix()
  return cam
}

describe('computeLineLabelLayout — the three regimes (§2)', () => {
  // RED-CHECK: against the unfixed code, `_buildAnnotationLines` always
  // draws the whole a1-b1 segment through the label's own position with no
  // gap at all — every one of these three cases would fail against that
  // (there is no `mode` distinction, no gap, no outside placement).

  it('regime 1: label fits with plenty of room to spare -> broken, gap centered', () => {
    const layout = computeLineLabelLayout(10, 1, 0.3, () => 0)
    expect(layout.mode).toBe('broken')
    if (layout.mode !== 'broken') throw new Error('unreachable')
    const padding = 0.4 * 0.3
    const gapHalf = 0.5 + padding
    expect(layout.gapStart).toBeCloseTo(5 - gapHalf, 9)
    expect(layout.gapEnd).toBeCloseTo(5 + gapHalf, 9)
    // Segments either side of the gap are non-degenerate.
    expect(layout.gapStart).toBeGreaterThan(0)
    expect(layout.gapEnd).toBeLessThan(10)
  })

  it('regime 2: label nearly fills the line -> still broken, right at the edge', () => {
    // gapHalf*2 just under lineLen.
    const labelWidth = 9.9
    const labelHeight = 0
    const layout = computeLineLabelLayout(10, labelWidth, labelHeight, () => 0)
    expect(layout.mode).toBe('broken')
    if (layout.mode !== 'broken') throw new Error('unreachable')
    expect(layout.gapEnd - layout.gapStart).toBeCloseTo(labelWidth, 9)
    expect(layout.gapStart).toBeGreaterThan(0)
    expect(layout.gapEnd).toBeLessThan(10)
  })

  it('regime 3: label wider than the line -> whole line, label pushed outside', () => {
    const layout = computeLineLabelLayout(2, 5, 0.3, (end) => (end === 1 ? 10 : 3))
    expect(layout.mode).toBe('outside')
    if (layout.mode !== 'outside') throw new Error('unreachable')
    // More room past end 1 -> label goes past end 1, beyond t=lineLen.
    expect(layout.end).toBe(1)
    expect(layout.labelCenterT).toBeGreaterThan(2)
  })

  it('regime 3, tie -> end 1 (the "right/up" end)', () => {
    const layout = computeLineLabelLayout(2, 5, 0.3, () => 7)
    expect(layout.mode).toBe('outside')
    if (layout.mode !== 'outside') throw new Error('unreachable')
    expect(layout.end).toBe(1)
  })

  it('regime 3, more room at end 0 -> label goes past end 0 (negative t)', () => {
    const layout = computeLineLabelLayout(2, 5, 0.3, (end) => (end === 0 ? 20 : 1))
    expect(layout.mode).toBe('outside')
    if (layout.mode !== 'outside') throw new Error('unreachable')
    expect(layout.end).toBe(0)
    expect(layout.labelCenterT).toBeLessThan(0)
  })

  it('boundary: gapHalf*2 exactly equal to lineLen counts as "too wide" (not broken)', () => {
    // gapHalf = 1 -> gapHalf*2 = 2 = lineLen; the `<` comparison means this
    // is NOT strictly less, so it must fall to the outside path.
    const layout = computeLineLabelLayout(2, 2 - 2 * 0.4 * 0, 0, () => 5)
    expect(layout.mode).toBe('outside')
  })
})

describe('screenRoomBeyond / pickOutsideEnd (§2 tie-break)', () => {
  it('reports less room toward the near viewport edge than toward the far one', () => {
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    // fov=45, dist=10 -> the frustum half-width at this depth is
    // tan(22.5deg)*10 ~= 4.14 world units; 3.5 sits inside it, close to the
    // +X edge, so a 100x100 viewport shows it near screen x~92.
    const nearRightEdge = new THREE.Vector3(3.5, 0, 0)
    const roomRight = screenRoomBeyond(camera, nearRightEdge, new THREE.Vector3(1, 0, 0), 100, 100)
    const roomLeft = screenRoomBeyond(camera, nearRightEdge, new THREE.Vector3(-1, 0, 0), 100, 100)
    expect(roomRight).toBeLessThan(roomLeft)
  })

  it('pickOutsideEnd favors the end with more on-screen room', () => {
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    // a is near the left edge, b is centered -> more room exists past b (end 1).
    const a = new THREE.Vector3(-3.5, 0, 0)
    const b = new THREE.Vector3(0, 0, 0)
    expect(pickOutsideEnd(camera, a, b, 100, 100)).toBe(1)
  })

  it('pickOutsideEnd favors end 0 when a has more room than b', () => {
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    const a = new THREE.Vector3(0, 0, 0)
    const b = new THREE.Vector3(3.5, 0, 0)
    expect(pickOutsideEnd(camera, a, b, 100, 100)).toBe(0)
  })

  // RED-CHECK (both tests below): against the unfixed `screenRoomBeyond`, an
  // origin whose OWN projection is already outside the viewport still runs
  // the ray-to-edge computation as if it were inside — for a point pushed
  // back toward the visible box that reads as a large positive distance (the
  // near-full viewport span), for a point pushed further off-screen it reads
  // as a negative `t`, which the fallback branch turns into the SAME "plenty
  // of room" value a genuinely on-screen point gets. Either way, an
  // already-invisible endpoint looks like it has room to spare.

  it('reports zero room for an origin that has already scrolled off the viewport', () => {
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    // fov=45, dist=10 -> frustum half-width ~4.14 world units; 6 is well
    // outside it, so this projects off the right edge of a 100x100 viewport.
    const offScreen = new THREE.Vector3(6, 0, 0)
    expect(screenRoomBeyond(camera, offScreen, new THREE.Vector3(1, 0, 0), 100, 100)).toBe(0)
    expect(screenRoomBeyond(camera, offScreen, new THREE.Vector3(-1, 0, 0), 100, 100)).toBe(0)
  })

  it("pickOutsideEnd does not favor an end that is already off-screen over one that is genuinely visible", () => {
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    // a is on-screen (bounded room to its own left); b has already scrolled
    // off the right edge — the only genuine room is past a.
    const a = new THREE.Vector3(-1, 0, 0)
    const b = new THREE.Vector3(6, 0, 0)
    expect(pickOutsideEnd(camera, a, b, 100, 100)).toBe(0)
  })

  // RED-CHECK (both tests below, against the unfixed `screenRoomBeyond`):
  // a point directly BEHIND the camera projects, via `Vector3.project`'s
  // perspective divide, to NDC (0, 0) — screen center — because the sign
  // flip that puts the point behind the camera also flips the divide's `w`.
  // The pre-fix off-screen check only looked at the divided x/y, so it read
  // this as the MOST on-screen point possible: measured `screenRoomBeyond`
  // returning 50 (half the 100px viewport) for a point straight behind the
  // camera, instead of 0 — playtest-2 DELTA review finding 3.

  it('reports zero room for an origin directly behind the camera', () => {
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    // Camera sits at z=10 looking toward -Z (world); z=15 is further +Z than
    // the camera itself, i.e. behind it.
    const behind = new THREE.Vector3(0, 0, 15)
    expect(screenRoomBeyond(camera, behind, new THREE.Vector3(1, 0, 0), 100, 100)).toBe(0)
    expect(screenRoomBeyond(camera, behind, new THREE.Vector3(-1, 0, 0), 100, 100)).toBe(0)
  })

  it('pickOutsideEnd does not favor an end that is behind the camera over one that is genuinely visible', () => {
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    const a = new THREE.Vector3(-1, 0, 0) // on-screen
    const b = new THREE.Vector3(0, 0, 15) // behind the camera
    expect(pickOutsideEnd(camera, a, b, 100, 100)).toBe(0)
  })
})

describe('axisDimensionPlane / lockedDimensionPlaneNormal (§3, corrected by the angle-dimensions fix)', () => {
  // These replace the tests of `viewFacingPlaneNormal`, which was removed
  // WITH its tests: the function was the angle-dimensions defect itself
  // (its screen-parallel plane is a 45-degree world diagonal from any ISO
  // camera), and keeping a correct-looking, well-tested helper around whose
  // whole documented purpose is the refuted rule invites its reuse. The
  // behavioral red-check for this fix lives in DimensionTool.test.ts's
  // "axis-aligned working plane" block and e2e/dimensions-oblique.spec.ts
  // (offset committed at the 45-degree diagonal (0, 0.64, 0.82)·k instead
  // of (0,0,1.2) from a real ISO gesture); these here pin the replacement
  // helper's own contract, which has no unfixed counterpart to run against.

  // Baseline (0,0,2)->(2,0,2) along +X; the standard ISO camera at (8,-8,8)
  // looking at (1,1,1), viewDir ∝ (-7, 9, -7).
  const A: [number, number, number] = [0, 0, 2]
  const BASE: [number, number, number] = [1, 0, 0]
  const ISO_EYE: [number, number, number] = [8, -8, 8]
  const ISO_VIEW: [number, number, number] = [-7 / 13.38, 9 / 13.38, -7 / 13.38]

  it('a drag above the baseline picks the vertical axis plane (the drag direction aligns with its offset axis on screen)', () => {
    // Ray through world (1, 0, 3.2), straight above the baseline midpoint —
    // an on-screen drag mostly along the +Z offset direction.
    const picked = axisDimensionPlane(A, BASE, ISO_EYE, [-7, 8, -4.8], ISO_VIEW)
    expect(picked).not.toBeNull()
    expect(Math.abs(picked!.normal[1])).toBeCloseTo(1, 9) // the y=0 plane
    expect(picked!.hit[0]).toBeCloseTo(1, 9)
    expect(picked!.hit[1]).toBeCloseTo(0, 9)
    expect(picked!.hit[2]).toBeCloseTo(3.2, 9)
  })

  it('a drag back across the flat plane picks it (the drag direction aligns with the flat offset axis on screen)', () => {
    // Ray through world (1, 3, 2), on the flat plane through the baseline.
    const picked = axisDimensionPlane(A, BASE, ISO_EYE, [-7, 11, -6], ISO_VIEW)
    expect(picked).not.toBeNull()
    expect(Math.abs(picked!.normal[2])).toBeCloseTo(1, 9) // the z=2 plane
    expect(picked!.hit[1]).toBeCloseTo(3, 9)
    expect(picked!.hit[2]).toBeCloseTo(2, 9)
  })

  it('a candidate whose offset direction lies along the view axis cannot win — it is undraggable from that camera', () => {
    // Camera along +Y (level with the baseline): the flat candidate's
    // offset direction (±Y) IS the view axis, so it projects to zero screen
    // motion and scores 0; the vertical plane (offset ±Z, fully visible)
    // wins for a ray aimed above the baseline.
    const picked = axisDimensionPlane([0, 0, 3], BASE, [1, -10, 3], [0, 10, 3], [0, 1, 0])
    expect(picked).not.toBeNull()
    expect(Math.abs(picked!.normal[1])).toBeCloseTo(1, 9)
    expect(picked!.hit[2]).toBeCloseTo(6, 9)
  })

  it('hysteresis: a latched plane keeps winning near the decision boundary; a decisive challenger takes over', () => {
    // From ISO, camera-facing measures tie EXACTLY between the two
    // candidates (equal-magnitude view components on all world axes) — the
    // drag direction decides, and the latch then holds the choice. The
    // up-drag latches the vertical plane; re-evaluating a mildly diagonal
    // drag with that latch keeps the vertical plane even though a fresh
    // evaluation of the same ray would narrowly prefer the flat plane; a
    // decisively flat drag still overthrows the latch.
    const up = axisDimensionPlane(A, BASE, ISO_EYE, [-7, 8, -4.8], ISO_VIEW)
    expect(Math.abs(up!.normal[1])).toBeCloseTo(1, 9)

    // Ray through world (2, 0, 3.2), above the baseline's far END — an
    // ambiguous aim (fresh evaluation narrowly prefers the flat reading).
    const endAim: [number, number, number] = [-6, 8, -4.8]
    const fresh = axisDimensionPlane(A, BASE, ISO_EYE, endAim, ISO_VIEW)
    expect(Math.abs(fresh!.normal[2])).toBeCloseTo(1, 9)
    const latched = axisDimensionPlane(A, BASE, ISO_EYE, endAim, ISO_VIEW, {
      latched: up!.normal, demoted: [],
    })
    expect(Math.abs(latched!.normal[1])).toBeCloseTo(1, 9) // latch holds

    // A decisively flat drag — through world (2.574, 2, 2), whose
    // baseline-perpendicular screen direction is the flat plane's own
    // offset direction almost exactly — clears PLANE_SWITCH_RATIO times
    // the incumbent's remaining score and takes over despite the latch,
    // demoting the overthrown plane for the rest of the gesture.
    const state = { latched: up!.normal, demoted: [] as [number, number, number][] }
    const overthrown = axisDimensionPlane(A, BASE, ISO_EYE, [-5.426, 10, -6], ISO_VIEW, state)
    expect(Math.abs(overthrown!.normal[2])).toBeCloseTo(1, 9)
    expect(state.demoted.length).toBe(1)
    expect(Math.abs(state.demoted[0][1])).toBeCloseTo(1, 9)
  })

  it('perpendicular view (viewDir ⊥ base): the drag carries no plane information and the most face-on plane wins — pinned deliberate behavior, not an accident', () => {
    // A pure-pitch orbit off the Front view: eye (1,-10,3.64) looking at
    // (1,0,0), viewDir = (0, 0.940, -0.342), viewDir·base = 0 exactly for
    // base = X. Every candidate offset direction lies in the plane
    // perpendicular to the baseline, which here CONTAINS viewDir — so all
    // P(d), and the drag direction itself, collapse onto one screen line,
    // both candidate planes project onto the SAME screen strip, and no
    // score of the cursor ray could tell them apart (the ambiguity is the
    // view's, not the score's). Deliberate resolution, per the
    // AMBIGUOUS-POSES doc: the most face-on plane (normal Y, |n·view| =
    // 0.94, vs 0.342 edge-on for the flat plane) wins for EVERY drag —
    // four drags aimed to express +Z, -Z, +Y and -Y offsets all resolve
    // identically; the arrow lock is the documented way to reach the flat
    // plane from this pose (pinned at tool level in DimensionTool.test.ts).
    const eye: [number, number, number] = [1, -10, 3.64]
    const len = Math.hypot(10, 3.64)
    const view: [number, number, number] = [0, 10 / len, -3.64 / len]
    for (const aim of [[1, 0, 2], [1, 0, -2], [1, 2, 0], [1, -2, 0]] as const) {
      const dir: [number, number, number] = [aim[0] - eye[0], aim[1] - eye[1], aim[2] - eye[2]]
      const picked = axisDimensionPlane([0, 0, 0], [1, 0, 0], eye, dir, view)
      expect(picked).not.toBeNull()
      expect(Math.abs(picked!.normal[1]), `drag toward ${aim.join(',')}`).toBeCloseTo(1, 9)
    }
  })

  it('the perpendicular-view collapse is a neighbourhood: at |viewDir·base| = 0.05 a flat-intent drag still resolves face-on', () => {
    // Same pose yawed slightly so viewDir·base ≈ 0.05 — the drag has
    // regained only O(0.05) discriminating power, far below the face-on
    // gap (measured 0.94 vs 0.34 for a drag aimed square across the flat
    // plane), so the face-on plane still wins. Documented extent of the
    // limitation, not a target to "fix": the information genuinely is not
    // on screen this close to perpendicular.
    const raw: [number, number, number] = [0.05, 0.9397, -0.342]
    const rawLen = Math.hypot(...raw)
    const view: [number, number, number] = [raw[0] / rawLen, raw[1] / rawLen, raw[2] / rawLen]
    const eye: [number, number, number] = [1 - 10 * view[0], -10 * view[1], -10 * view[2]]
    const aim: [number, number, number] = [1, 2, 0] // square across the flat plane
    const dir: [number, number, number] = [aim[0] - eye[0], aim[1] - eye[1], aim[2] - eye[2]]
    const picked = axisDimensionPlane([0, 0, 0], [1, 0, 0], eye, dir, view)
    expect(picked).not.toBeNull()
    expect(Math.abs(picked!.normal[1])).toBeCloseTo(1, 9)
  })

  it('returns null when the ray pierces no candidate plane (sighting down the baseline)', () => {
    // Baseline along +Z, ray along -Z: parallel to every plane containing
    // the baseline.
    expect(axisDimensionPlane([0, 0, 0], [0, 0, 1], [0, 0, 5], [0, 0, -1], [0, 0, -1])).toBeNull()
  })

  it('candidate planes always contain the baseline: every normal is perpendicular to base', () => {
    // Skew baseline — no axis-aligned shortcut available.
    const base: [number, number, number] = [
      1 / Math.sqrt(3), 1 / Math.sqrt(3), 1 / Math.sqrt(3),
    ]
    const picked = axisDimensionPlane([0, 0, 0], base, [5, -5, 5], [-5, 5.5, -4.5], ISO_VIEW)
    expect(picked).not.toBeNull()
    const n = picked!.normal
    expect(n[0] * base[0] + n[1] * base[1] + n[2] * base[2]).toBeCloseTo(0, 9)
    expect(Math.hypot(...n)).toBeCloseTo(1, 9)
  })

  it('lockedDimensionPlaneNormal: an axis perpendicular to the baseline gives exactly that axis plane', () => {
    expect(lockedDimensionPlaneNormal(2, [1, 0, 0])).toEqual([0, 0, 1]) // blue lock, X baseline -> flat plane
    expect(lockedDimensionPlaneNormal(1, [1, 0, 0])).toEqual([0, 1, 0]) // green lock -> vertical y-normal plane
  })

  it('lockedDimensionPlaneNormal: a tilted baseline gets the nearest plane that still CONTAINS it', () => {
    const base: [number, number, number] = [Math.cos(0.2), 0, Math.sin(0.2)] // X tilted 0.2 rad up
    const n = lockedDimensionPlaneNormal(2, base)! // blue lock
    expect(n[0] * base[0] + n[1] * base[1] + n[2] * base[2]).toBeCloseTo(0, 9) // contains the baseline
    expect(Math.hypot(...n)).toBeCloseTo(1, 9)
    expect(n[2]).toBeGreaterThan(0.9) // still essentially the flat plane
  })

  it('lockedDimensionPlaneNormal: an axis parallel to the baseline is unusable (null)', () => {
    expect(lockedDimensionPlaneNormal(0, [1, 0, 0])).toBeNull()
  })
})

describe('antipodalTolerance / distPointToLine / chordPassesNearCentre (§4)', () => {
  it('a chord through the exact centre is a diameter', () => {
    expect(chordPassesNearCentre([0, 0, 0], 2, [2, 0, 0], [-2, 0, 0])).toBe(true)
  })

  it('a chord well off-centre is not a diameter', () => {
    expect(chordPassesNearCentre([0, 0, 0], 2, [2, 0, 0], [0, 2, 0])).toBe(false)
  })

  it('boundary: just inside the tolerance counts as a diameter', () => {
    const radius = 10
    const tol = antipodalTolerance(radius)
    // A chord whose closest approach to the centre is 0.9*tol.
    const offset = tol * 0.9
    const a: [number, number, number] = [10, offset, 0]
    const b: [number, number, number] = [-10, offset, 0]
    expect(distPointToLine([0, 0, 0], a, b)).toBeCloseTo(offset, 9)
    expect(chordPassesNearCentre([0, 0, 0], radius, a, b)).toBe(true)
  })

  it('boundary: just outside the tolerance does not count as a diameter', () => {
    const radius = 10
    const tol = antipodalTolerance(radius)
    const offset = tol * 1.1
    const a: [number, number, number] = [10, offset, 0]
    const b: [number, number, number] = [-10, offset, 0]
    expect(chordPassesNearCentre([0, 0, 0], radius, a, b)).toBe(false)
  })

  it('the absolute floor keeps a tiny circle from treating every chord as a diameter', () => {
    // radius so small that 0.02*radius is below the 1e-6 floor.
    expect(antipodalTolerance(1e-5)).toBeCloseTo(1e-6, 12)
  })
})

describe('buildRadialGeometry (§4 — "the drawn geometry must show the measurement")', () => {
  // RED-CHECK: the unfixed renderer draws only `anchor -> anchor+leaderDir`
  // for a radial dimension (`_buildAnnotationLines`'s 'radial' branch,
  // `DimensionTool.ts` ~408-410's `_updatePreview`) — it never touches the
  // centre at all. Both assertions below (the segment starting at the
  // centre, and the diameter spanning `2r`) fail against that: the old code
  // has no "measured" segment distinct from the leader, so `measured[0]`
  // would be `anchor` (not `center`) for a radius, and there is no
  // antipodal point construction for a diameter at all.

  it('radius: the measured segment runs from the centre to the rim anchor', () => {
    const center: [number, number, number] = [1, 2, 0]
    const anchor: [number, number, number] = [4, 2, 0] // radius 3 along +X
    const geo = buildRadialGeometry(center, anchor, 'radius')
    expect(geo.measured[0]).toEqual(center)
    expect(geo.measured[1]).toEqual(anchor)
    expect(geo.farEnd).toEqual(center)
  })

  it('diameter: the measured segment spans 2r through the centre (rim to rim)', () => {
    const center: [number, number, number] = [0, 0, 0]
    const anchor: [number, number, number] = [5, 0, 0]
    const geo = buildRadialGeometry(center, anchor, 'diameter')
    const [p, q] = geo.measured
    const len = Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2])
    expect(len).toBeCloseTo(10, 9)
    // Both ends are equidistant from the centre (both on the rim).
    expect(Math.hypot(...p.map((c, i) => c - center[i]))).toBeCloseTo(5, 9)
    expect(Math.hypot(...q.map((c, i) => c - center[i]))).toBeCloseTo(5, 9)
    // farEnd is the antipode, not the centre.
    expect(geo.farEnd).not.toEqual(center)
  })

  it('diameter: the antipode is exactly opposite the anchor through the centre', () => {
    const center: [number, number, number] = [1, 1, 1]
    const anchor: [number, number, number] = [1, 1, 4] // +Z, radius 3
    const geo = buildRadialGeometry(center, anchor, 'diameter')
    expect(geo.farEnd).toEqual([1, 1, -2])
  })
})

describe('pushCenterTick', () => {
  it('pushes a two-segment cross centered exactly at the given point', () => {
    const positions: number[] = []
    pushCenterTick(positions, [1, 2, 3], [0, 0, 1], 0.05)
    expect(positions.length).toBe(12) // 2 segments * 2 points * 3 coords
    // Each segment's midpoint is the center.
    for (const seg of [0, 6]) {
      const midX = (positions[seg] + positions[seg + 3]) / 2
      const midY = (positions[seg + 1] + positions[seg + 4]) / 2
      const midZ = (positions[seg + 2] + positions[seg + 5]) / 2
      expect(midX).toBeCloseTo(1, 9)
      expect(midY).toBeCloseTo(2, 9)
      expect(midZ).toBeCloseTo(3, 9)
    }
  })
})
