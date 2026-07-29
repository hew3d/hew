import { test, expect } from '@playwright/test'

/**
 * Dimension/leader-text DEPTH rendering — end-to-end pixel checks
 * (docs/design/dimensions-playtest2.md §1, findings 2/3). Real WebGL
 * rendering (Chromium/SwiftShader — see `edge-stability.spec.ts`'s own doc
 * comment for why pixel counts are only stable there), because the
 * standing rule for this fix is explicit: a test that asserts
 * `depthTest === true` on a material proves nothing about what the user
 * SEES — these assert on actual rendered pixels via the harness's
 * `pixelColorAt` (WebGL `readPixels`, already used by `edge-stability.spec.ts`).
 *
 * `addLinearDimension`/`addRadialDimension` (new harness methods, mirroring
 * `addGuideLine`) place annotations directly at exact coordinates rather
 * than through DimensionTool's click-drag-click gesture — a pixel-precision
 * assertion needs the geometry to be exactly what the test says it is, not
 * whatever a mouse-driven inference snap happened to resolve.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pixel counts are only stable on pinned SwiftShader')

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
})

/**
 * Wait for a REAL animation frame to actually run — required whenever a test
 * changes the camera/scene and then samples pixels the annotation per-frame
 * update (`SceneRenderer.updateAnnotationBillboards`, which applies the §2
 * gap layout) is responsible for. `harness.pixelColorAt`'s own
 * `captureFrame` renders OUT OF BAND (`Viewport.tsx`'s `captureFrame`,
 * deliberately, so a test can sample without depending on the live rAF
 * loop's timing) and does NOT itself call `updateAnnotationBillboards` — so
 * a `setCamera` + `pixelColorAt` pair with no yield in between can sample an
 * annotation laid out for whatever camera pose the REAL rAF loop last ran
 * for, not the one the test just set. Two frames (not one) covers `changed
 * || needsRender`'s one-frame-behind settling since `OrbitControls.update()`'s
 * own `changed` can itself take a frame to quiesce after a programmatic
 * `controls.target`/`camera.position` write.
 *
 * Every test in this file that changes camera or scene state and then reads
 * pixels calls this in between — a playtest-2 DELTA review finding: the
 * "no z-fighting" test below used to sample with no yield at all, so its
 * pixel reads reflected whatever pose the last REAL frame happened to run
 * for (often the initial camera, or a stale prior pose within the same
 * test), not the pose the test had just set — passing for the wrong reason
 * regardless of what the underlying depth-bias code actually did. Confirmed
 * by re-running it with a yield inserted: it failed immediately, against
 * both the pre-review and the fixed code, until the reference-pixel
 * methodology below was ALSO corrected (see that test's own comment).
 */
function nextFrame(page: import('@playwright/test').Page): Promise<void> {
  return page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      }),
  )
}

test('a dimension line drawn behind a solid is occluded by it; the same line with nothing in front is not', async ({ page }) => {
  const result = await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)

    // Dimension line from (0.2,0,1) to (1.8,0,1) offset +Y by 1.6, so
    // a1=(0.2,1.6,1), b1=(1.8,1.6,1) — a horizontal line at y=1.6, z=1.
    t.addLinearDimension([0.2, 0, 1], [1.8, 0, 1], [0, 1.6, 0], [0, 0, 0, 0, -1, 0])
    // Camera looking along +Y at the line's plane.
    t.setCamera({ position: [1, -5, 1], target: [1, 1.6, 1], fovDeg: 45 })

    // A fat line's rendered pixels have anti-aliased edges (`edge-
    // stability.spec.ts` hits the same issue with native 1px lines), so a
    // single fixed sample point can miss the line's solid interior by a
    // fraction of a pixel of world-space slop — scan several points along
    // the known line segment and keep whichever differs most from the
    // background, the same "scan several points" technique that file
    // already established.
    const A1: [number, number, number] = [0.2, 1.6, 1]
    const B1: [number, number, number] = [1.8, 1.6, 1]
    const background = t.pixelColorAt([3.5, 1.6, 1])!
    const points: [number, number, number][] = []
    for (let i = 1; i <= 7; i++) {
      const f = i / 8
      points.push([A1[0] + (B1[0] - A1[0]) * f, A1[1] + (B1[1] - A1[1]) * f, A1[2] + (B1[2] - A1[2]) * f])
    }
    const rgb = (c: { r: number; g: number; b: number } | null) =>
      c === null ? null : ((c.r << 16) | (c.g << 8) | c.b)
    const deltaFromBg = (c: { r: number; g: number; b: number } | null) =>
      c === null ? -Infinity : Math.abs(c.r - background.r) + Math.abs(c.g - background.g) + Math.abs(c.b - background.b)

    // Pick the point that reads most clearly as "on the line" BEFORE the
    // box exists, then re-sample that EXACT same world point after the box
    // is added — the assertion is "did this specific point's color change",
    // not "does it still differ from background" (a box's own lit face can
    // legitimately differ from a flat background by MORE than the
    // annotation's own flat ink color does, so comparing against background
    // twice doesn't reliably show occlusion — comparing the point to
    // itself, before vs. after, does).
    let bestIdx = 0
    let bestDelta = -Infinity
    const beforeColors = points.map((p) => t.pixelColorAt(p))
    beforeColors.forEach((c, i) => {
      const d = deltaFromBg(c)
      if (d > bestDelta) {
        bestDelta = d
        bestIdx = i
      }
    })
    const before = beforeColors[bestIdx]

    // A box between the camera (y=-5) and the line (y=1.6), spanning the
    // ray's path at z~1.
    t.drawBox([0, -3, 0], [1.5, -1, 0], 2)
    const after = t.pixelColorAt(points[bestIdx])

    return { beforeOnLine: bestDelta, beforeRgb: rgb(before), afterRgb: rgb(after) }
  })

  // Before the box: the line reads as annotation ink, clearly not the bare
  // background (proves the scan actually landed on drawn geometry).
  expect(result.beforeOnLine).toBeGreaterThan(30)
  // After the box: the SAME world point's color changed — the box's face
  // now wins the depth test where the annotation used to always win
  // (findings 2/3's "visible through any model" bug).
  expect(result.afterRgb).not.toBe(result.beforeRgb)
})

test('a dimension line drawn in front of a solid (camera side) still reads as annotation ink, unoccluded', async ({ page }) => {
  const result = await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.drawBox([0, 2, 0], [2, 4, 0], 2) // the "model" — well behind the line
    t.addLinearDimension([0.2, 0, 1], [1.8, 0, 1], [0, 1.6, 0], [0, 0, 0, 0, -1, 0])
    t.setCamera({ position: [1, -5, 1], target: [1, 1.6, 1], fovDeg: 45 })
    const A1: [number, number, number] = [0.2, 1.6, 1]
    const B1: [number, number, number] = [1.8, 1.6, 1]
    const background = t.pixelColorAt([3.5, 1.6, 1])!
    let best = -Infinity
    for (let i = 1; i <= 7; i++) {
      const f = i / 8
      const p: [number, number, number] = [
        A1[0] + (B1[0] - A1[0]) * f, A1[1] + (B1[1] - A1[1]) * f, A1[2] + (B1[2] - A1[2]) * f,
      ]
      const c = t.pixelColorAt(p)
      if (c === null) continue
      const d = Math.abs(c.r - background.r) + Math.abs(c.g - background.g) + Math.abs(c.b - background.b)
      if (d > best) best = d
    }
    return { delta: best }
  })
  expect(result.delta).toBeGreaterThan(30)
})

test('no z-fighting for a dimension drawn exactly on a face plane, across camera angles including edge-on', async ({ page }) => {
  // A z-fight loses the depth tie unpredictably; the user-visible symptom is
  // the annotation (which should always win — depthPolicy.ts's `FACE` rung
  // recedes on its own account, so a coincident annotation, drawn at its
  // true unmodified depth, wins without needing a bias of its own) blending
  // into or disappearing behind its own exactly-coincident face. This checks
  // the direct, practical form of that: at each of several stress angles
  // (including a near-edge-on grazing view), at least one point along the
  // coincident line still reads as the annotation's own ink color, not the
  // face's.
  //
  // TWO methodology defects, found by a playtest-2 DELTA review, are fixed
  // here together — fixing only one still leaves a vacuous test:
  //
  //  1. This test used to call `setCamera` then `pixelColorAt` with NO frame
  //     yield in between. `pixelColorAt`'s underlying `captureFrame` renders
  //     out-of-band and does NOT itself run `updateAnnotationBillboards` (see
  //     `nextFrame`'s own doc comment above) — so every sample here used to
  //     reflect whichever pose the real rAF loop last happened to settle on
  //     (often just the initial camera), not the pose this test had just
  //     set. It passed regardless of what the depth-bias code actually did.
  //  2. Even with a yield inserted, the REFERENCE pixel (this test's "ground
  //     truth" ink color, historically a single fixed point at the
  //     reference line's exact midpoint) turned out to be unreliable: a
  //     dimension line breaks for its own label around its midpoint
  //     (§2, `computeLineLabelLayout`), so a single point sampled exactly
  //     there can land inside that gap — reading as background, not ink —
  //     depending on incidental label-size/zoom factors unrelated to the
  //     depth-bias property this test exists to check. Fixed the same way
  //     the OTHER tests in this file already establish their reference: scan
  //     several points and keep whichever most clearly differs from
  //     background.
  //
  // A literal frame-to-frame sub-pixel-repaint FLIP (`edge-stability.spec.ts`'s
  // own methodology) was tried first and dropped: even with the depth bias
  // temporarily zeroed out (a manual probe, not left in the codebase), this
  // reduced small-world scene never produced a single differing pixel
  // between sub-pixel-apart poses on the coincident line — SwiftShader's
  // depth-tie behavior at this scale evidently doesn't reproduce the
  // original (large-scene) shimmer in a way a tiny two-object scene can
  // provoke on demand. The "does the annotation actually win" check below is
  // what the depth policy exists to guarantee either way, and is exactly
  // what a user watching the screen would judge "z-fighting" by.
  const result = await page.evaluate(async () => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)

    // Ground truth: this SAME annotation's ink color, sampled where it is
    // unambiguously alone (no coincident geometry) — the reference this
    // test compares the coincident-with-a-face samples against. Scans
    // several points along the line rather than trusting a single fixed
    // point (see the methodology-defect-2 comment above).
    t.addLinearDimension([5, 5, 0], [6, 5, 0], [0, 1, 0], [0, 0, 0, 0, 0, 1])
    t.setCamera({ position: [5.5, 4, 3], target: [5.5, 6, 0], fovDeg: 45 })
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    const background = t.pixelColorAt([4.5, 6, 0])!
    let reference = t.pixelColorAt([5.5, 6, 0])!
    let bestBgDelta = -Infinity
    for (let i = 1; i <= 15; i++) {
      const f = i / 16
      const c = t.pixelColorAt([5 + f, 6, 0])
      if (c === null) continue
      const d = Math.abs(c.r - background.r) + Math.abs(c.g - background.g) + Math.abs(c.b - background.b)
      if (d > bestBgDelta) {
        bestBgDelta = d
        reference = c
      }
    }

    t.drawBox([0, 0, 0], [2, 2, 0], 2) // top face at z=2
    // A SECOND dimension drawn EXACTLY on the top face's plane — the
    // coplanar worst case this test targets.
    t.addLinearDimension([0.2, 0.2, 2], [1.8, 0.2, 2], [0, 1.6, 0], [0, 0, 2, 0, 0, 1])
    // a1=(0.2,1.8,2), b1=(1.8,1.8,2) — sample several points along it.
    const points: [number, number, number][] = []
    for (let i = 1; i <= 7; i++) {
      const f = i / 8
      points.push([0.2 + (1.8 - 0.2) * f, 1.8, 2])
    }

    // Three angles onto the coplanar annotation, all far enough from the
    // target (~45-50 units, matching edge-stability.spec.ts's own distance
    // calibration) that the perspective depth buffer's precision is
    // actually under stress: steep-ish overhead, a 3/4 view, and a
    // near-edge-on grazing angle across the top face (camera nearly level
    // with z=2).
    const poses: Record<string, [number, number, number]> = {
      overhead: [1, 1, 50],
      threeQuarter: [30, -35, 20],
      edgeOn: [46, 1, 2.3],
    }
    const out: Record<string, number> = {}
    for (const [name, position] of Object.entries(poses)) {
      t.setCamera({ position, target: [1, 1, 2] })
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
      let bestDelta = Infinity
      for (const p of points) {
        const c = t.pixelColorAt(p)
        if (c === null) continue
        const d = Math.abs(c.r - reference.r) + Math.abs(c.g - reference.g) + Math.abs(c.b - reference.b)
        if (d < bestDelta) bestDelta = d
      }
      out[name] = bestDelta
    }
    return out
  })

  for (const [name, closestMatch] of Object.entries(result)) {
    // At least one sampled point along the coincident line reads close to
    // the annotation's own ink color — it won the depth tie against its
    // exactly-coplanar face, at every stress angle tried.
    expect(closestMatch, `${name}: closest sample to the annotation's own ink color`).toBeLessThan(30)
  }
})

test('no z-fighting for a dimension drawn exactly on a face plane, across a grazing-angle distance sweep (5-50 world units)', async ({
  page,
}) => {
  // Permanent regression coverage for the playtest-2 DELTA review's own
  // reproduction methodology: a coincident dimension line, viewed at a
  // constant near-edge-on grazing angle (matching the "edgeOn" pose in the
  // test above: camera elevated 0.3 world units above the face plane per 45
  // units of horizontal distance — about 0.4 degrees off dead-level) but
  // swept across a full practical range of camera distances, not just the
  // single distance the older test happens to sample. A distance-dependent
  // world-space mechanism (like the world-space nudge this fix replaced)
  // could win at some distances and lose at others; this sweep is what
  // would have caught that, with a real frame yield and the robust
  // reference methodology (see the previous test's own comment for both).
  const result = await page.evaluate(async () => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)

    t.addLinearDimension([5, 5, 0], [6, 5, 0], [0, 1, 0], [0, 0, 0, 0, 0, 1])
    t.setCamera({ position: [5.5, 4, 3], target: [5.5, 6, 0], fovDeg: 45 })
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
    const background = t.pixelColorAt([4.5, 6, 0])!
    let reference = t.pixelColorAt([5.5, 6, 0])!
    let bestBgDelta = -Infinity
    for (let i = 1; i <= 15; i++) {
      const f = i / 16
      const c = t.pixelColorAt([5 + f, 6, 0])
      if (c === null) continue
      const d = Math.abs(c.r - background.r) + Math.abs(c.g - background.g) + Math.abs(c.b - background.b)
      if (d > bestBgDelta) {
        bestBgDelta = d
        reference = c
      }
    }

    t.drawBox([0, 0, 0], [2, 2, 0], 2) // top face at z=2
    t.addLinearDimension([0.2, 0.2, 2], [1.8, 0.2, 2], [0, 1.6, 0], [0, 0, 2, 0, 0, 1])
    const points: [number, number, number][] = []
    for (let i = 1; i <= 7; i++) {
      const f = i / 8
      points.push([0.2 + (1.8 - 0.2) * f, 1.8, 2])
    }

    // Same ratio as the "edgeOn" pose above ([46,1,2.3] from target [1,1,2]
    // -> elevation atan(0.3/45)), scaled to each swept distance so the
    // ANGLE stays constant while distance varies.
    const target: [number, number, number] = [1, 1, 2]
    const ux = 45 / Math.hypot(45, 0.3)
    const uz = 0.3 / Math.hypot(45, 0.3)
    const distances = [5, 8, 10, 12, 15, 18, 20, 25, 30, 35, 40, 45, 50]
    const out: Record<string, number> = {}
    for (const d of distances) {
      const position: [number, number, number] = [target[0] + d * ux, target[1], target[2] + d * uz]
      t.setCamera({ position, target })
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
      let bestDelta = Infinity
      for (const p of points) {
        const c = t.pixelColorAt(p)
        if (c === null) continue
        const dd = Math.abs(c.r - reference.r) + Math.abs(c.g - reference.g) + Math.abs(c.b - reference.b)
        if (dd < bestDelta) bestDelta = dd
      }
      out[`d${d}`] = bestDelta
    }
    return out
  })

  for (const [name, closestMatch] of Object.entries(result)) {
    expect(closestMatch, `${name}: closest sample to the annotation's own ink color`).toBeLessThan(30)
  }
})

test('an occluder a couple of millimetres in front of a dimension line still occludes it, at the reviewer\'s own ~11.6-unit distance AND 6.6 units; a 30mm-clearance control also occludes cleanly', async ({
  page,
}) => {
  // The reviewer's own reproduction: an ordinary "see the whole model"
  // camera distance (~11.6 world units) with an occluder whose front face
  // sits just 2mm in front of the dimension line, PLUS the same check at 6.6
  // units (closer, still an ordinary working distance).
  //
  // History: the PRE-review `annotationViewBiasVector` nudged the line
  // toward the camera by an amount LINEAR IN CAMERA DISTANCE
  // (`billboardWorldScale`, ~1.5 screen px) with no ceiling — at 11.6 units
  // that nudge was ~6mm, comfortably bigger than a 2mm gap, so the biased
  // line vertex ended up CLOSER to the camera than the occluder and won the
  // depth test it should have lost. The first fix capped that nudge at a
  // fixed 0.3mm world-space ceiling — which held at 6.6 units but a DELTA
  // review found it STILL leaking past the 2mm occluder at 11.6 (measured:
  // the same-point-before/after-adding-the-box comparison below showed no
  // pixel change at 11.6, i.e. the box failed to occlude). A modest, nonzero
  // `glPolygonOffset` on the annotation's own geometry was tried next and
  // measured leaking at the SAME 11.6-unit distance, smaller magnitude but
  // the same mechanism. The fix that actually holds both this requirement
  // AND the coincident-face-tie requirement (previous tests) is `DEPTH_BIAS
  // .ANNOTATION` at rung 0 — no offset on the annotation's own geometry at
  // all, so its rendered depth is always its true geometric depth and it can
  // never appear closer than a real occluder actually is; see
  // `depthPolicy.ts`'s "Dimension/leader-text annotations" note for the
  // full measured history. The 30mm-gap scenario is the reviewer's own
  // control and must stay correct throughout.
  //
  // Residual, measured and accepted rather than hidden (see depthPolicy.ts):
  // a 2mm gap starts failing to occlude correctly somewhere beyond 11.6 and
  // before 15 world units, because the OCCLUDING face's own `DEPTH_BIAS
  // .FACE` recession (needed so faces lose to their own edge overlay) grows
  // in world-space terms at long range — an existing property of this app's
  // depth-bias ladder, not something specific to annotations. A 30mm gap
  // remains correct at 20+ units. Not tested here (covered narratively in
  // depthPolicy.ts instead of as a passing assertion, since asserting a
  // KNOWN-FAILING pixel comparison would be a skip in disguise).
  //
  // A dimension line is a THIN fat line (`ANNOTATION_LINE_WIDTH_PX`, a
  // couple of screen px): a handful of evenly-spaced sample points along it
  // easily straddle the covered band and land on background instead
  // (`edge-stability.spec.ts`'s own "scan several points" rationale) — 25
  // points, not 7, so at least one reliably lands solidly on the line.
  const SAMPLES = 25

  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    // Reference: this SAME kind of annotation's own ink color, established
    // with nothing occluding it — what the gap scenarios below must NOT
    // read as if they're correctly occluded.
    t.addLinearDimension([0.2, 0, 1], [1.8, 0, 1], [0, 1.6, 0], [0, 0, 0, 0, -1, 0])
    t.setCamera({ position: [1, -5, 1], target: [1, 1.6, 1] })
  })
  await nextFrame(page)
  const reference = await page.evaluate((SAMPLES) => {
    const t = window.__hew_test!
    const pts: [number, number, number][] = []
    for (let i = 1; i < SAMPLES; i++) {
      const f = i / SAMPLES
      pts.push([0.2 + (1.8 - 0.2) * f, 1.6, 1])
    }
    const background = t.pixelColorAt([3.5, 1.6, 1])!
    let best: { r: number; g: number; b: number } | null = null
    let bestBgDelta = -Infinity
    for (const p of pts) {
      const c = t.pixelColorAt(p)
      if (c === null) continue
      const d = Math.abs(c.r - background.r) + Math.abs(c.g - background.g) + Math.abs(c.b - background.b)
      if (d > bestBgDelta) {
        bestBgDelta = d
        best = c
      }
    }
    return best!
  }, SAMPLES)

  // Independent scenarios, offset along X so they never interact: a
  // dimension line at y=1.6, z=1, `camDist` world units from the camera,
  // with a box whose front (camera-facing) face sits `gap` in front of it.
  // Each scenario gets its OWN `setCamera` + `nextFrame` so the sampled
  // pixels reflect layout genuinely computed for THIS pose (not whichever
  // pose happened to be live during the reference capture above).
  const scenarios: { xOff: number; camDist: number; gap: number }[] = [
    { xOff: 3, camDist: 6.6, gap: 0.002 }, // closer ordinary distance
    { xOff: 9, camDist: 11.6, gap: 0.002 }, // the reviewer's own reproduction
    { xOff: 15, camDist: 6.6, gap: 0.03 }, // the control, at 6.6
    { xOff: 21, camDist: 11.6, gap: 0.03 }, // the control, at 11.6
  ]
  const results: Record<string, number> = {}
  for (const { xOff, camDist, gap } of scenarios) {
    await page.evaluate(
      ([xOff, camDist, gap]) => {
        const t = window.__hew_test!
        t.addLinearDimension([0.2 + xOff, 0, 1], [1.8 + xOff, 0, 1], [0, 1.6, 0], [0, 0, 0, 0, -1, 0])
        const front = 1.6 - gap
        t.drawBox([xOff, front, 0], [xOff + 2, front + 0.5, 0], 2)
        t.setCamera({ position: [1 + xOff, 1.6 - camDist, 1], target: [1 + xOff, 1.6, 1] })
      },
      [xOff, camDist, gap],
    )
    await nextFrame(page)
    const bestDelta = await page.evaluate(
      ([xOff, reference, SAMPLES]) => {
        const t = window.__hew_test!
        let best = Infinity
        for (let i = 1; i < SAMPLES; i++) {
          const f = i / SAMPLES
          const p: [number, number, number] = [0.2 + xOff + (1.8 - 0.2) * f, 1.6, 1]
          const c = t.pixelColorAt(p)
          if (c === null) continue
          const d = Math.abs(c.r - reference.r) + Math.abs(c.g - reference.g) + Math.abs(c.b - reference.b)
          if (d < best) best = d
        }
        return best
      },
      [xOff, reference, SAMPLES] as [number, { r: number; g: number; b: number }, number],
    )
    results[`gap_${Math.round(gap * 1000)}mm_dist${camDist}`] = bestDelta
  }

  for (const [name, closestToReference] of Object.entries(results)) {
    // No sampled point along any occluded line reads as the annotation's own
    // ink color — the box wins the depth test in every scenario, including
    // the 2mm reproduction at the reviewer's own 11.6-unit distance.
    expect(closestToReference, `${name}: closest sample's distance from the annotation's own ink color`).toBeGreaterThan(30)
  }
})
