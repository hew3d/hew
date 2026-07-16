import { test, expect } from '@playwright/test'

/**
 * Edge-render stability under sub-pixel camera motion.
 *
 * Regression coverage for the post-orbit edge shimmer: after the user releases
 * an orbit, OrbitControls' damping tail repaints the scene for a second or two
 * at camera deltas far below one pixel. Each of those repaints must produce
 * (essentially) the same image — historically it did not, because the edge
 * overlay (`LineSegments`) is exactly coplanar with the faces it outlines, and
 * without `polygonOffset` on the face materials the depth test resolved the
 * edge/face tie by floating-point rounding noise. That noise is a function of
 * the camera matrices, so every tail frame re-rolled it and thousands of
 * high-contrast pixels flipped per frame along edges (worst on large models,
 * where perspective depth leaves millimetres per depth-buffer quantum at
 * building-scale distances).
 *
 * The probe renders one large-world scene at two poses a ~0.09 mm camera
 * rotation apart (deeply sub-pixel at 45 m) and counts differing pixels
 * in-page (`__hew_test.frameStability`). Chromium only: the functional
 * chromium project pins SwiftShader software GL, which makes the counts
 * bit-stable per machine — but SwiftShader JITs per architecture, so counts
 * can differ across machines (the same variance class the visual goldens'
 * -linux/-darwin split exists for); WebKit renders on the host GPU, where
 * rasterization noise is not ours to assert on at all.
 *
 * What each bound means:
 *  - `hard` (a pixel flipping by > 60/255) is the shimmer signature itself —
 *    a dark edge line trading places with the face fill behind it. Measured
 *    1 with the fix on two machines, 63 with the fix removed; the bound is
 *    tight because the signal is.
 *  - `differing` (> 8/255) additionally counts per-architecture AA rounding:
 *    with the fix it measured 55 on one machine and 82 on another (each
 *    bit-stable across repeated runs), 954 with the defect. Its bound is a
 *    labeled sanity ceiling that only catches order-of-magnitude
 *    regressions, not a precise expectation.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pixel counts are only stable on pinned SwiftShader')

test('idle repaints at sub-pixel camera deltas do not flip edge pixels', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  const result = await page.evaluate(() => {
    const t = window.__hew_test!
    // Large-world scene: a spread grid of boxes (a third of them rotated so
    // their edges sit oblique to the axes), viewed from ~45 m — far enough
    // that the perspective depth buffer quantizes to whole millimetres and
    // coplanar edge/face fragments genuinely tie.
    const N = 7
    const S = 5
    const half = ((N - 1) * S) / 2
    for (let i = 0; i < N; i++) {
      for (let j = 0; j < N; j++) {
        const x = i * S - half
        const y = j * S - half
        const w = 3
        const h = 1 + (3 * ((i * 7 + j * 3) % 5)) / 5
        t.drawBox([x - w / 2, y - w / 2, 0], [x + w / 2, y + w / 2, 0], h)
      }
    }
    t.getObjectIds().forEach((id, idx) => {
      if (idx % 3 === 0) t.rotateObject(id, 17 + (idx % 50))
    })
    // Model pixels only: the procedural grid and the axes legitimately
    // re-shade with any camera change and would pollute the count.
    t.setGridVisible(false)
    t.setAxesVisible(false)

    const base = { position: [30, -32, 22] as [number, number, number], target: [0, 0, 1] as [number, number, number] }
    const eps = 2e-6 // rad about Z — ~0.09 mm of camera travel at 45 m
    const rot = {
      position: [
        base.position[0] * Math.cos(eps) - base.position[1] * Math.sin(eps),
        base.position[0] * Math.sin(eps) + base.position[1] * Math.cos(eps),
        base.position[2],
      ] as [number, number, number],
      target: base.target,
    }
    return t.frameStability(base, rot)
  })

  // Sanity: the probe actually rendered something at a real size.
  expect(result.width).toBeGreaterThan(100)
  expect(result.height).toBeGreaterThan(100)
  // Tight bound on the defect signal: hard flips measured 1 with the fix on
  // two machines (bit-stable per machine) and 63 with the fix removed.
  expect(result.hard).toBeLessThanOrEqual(8)
  // Sanity ceiling only: `differing` includes per-architecture AA rounding
  // (55 and 82 with the fix on two machines; 954 with the defect), so this
  // bound exists to catch order-of-magnitude regressions, nothing finer.
  expect(result.differing).toBeLessThanOrEqual(500)
})
