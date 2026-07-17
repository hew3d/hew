import { test, expect } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
} from './helpers/projectWorldToScreen'

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

/**
 * Line-vs-line sibling of the probe above: coincident LINEWORK instead of
 * edge-vs-face. Two defects, two coincidence classes:
 *
 *  - A 1 m cube on the origin shares its vertical edge with the +Z axis and
 *    its bottom edges with +X/+Y (axes visible, grid hidden). The axes are
 *    150 m fat lines (`Line2`), and any half extending behind the camera (or
 *    projecting far off-screen) used to wobble a few tenths of a pixel per
 *    repaint — `LineMaterial`'s float32 near-plane trim and its sloppy
 *    interpolation across extreme quads (see `clampOriginAxes` in
 *    `Viewport.tsx`). Worst where an axis overlays high-contrast coincident
 *    linework: measured 609 hard flips at the far pose (and 20 near) before
 *    the float64 frustum clip + depth-bias ladder, 0 after.
 *
 *  - A ground sketch retracing the cube's footprint puts fat sketch lines
 *    exactly over the cube's native bottom edge lines (axes hidden). Native
 *    `GL_LINES` can't be polygon-offset, so the tie is settled by biasing
 *    the fat sketch lines one ladder rung in front (depthPolicy.ts).
 *    Measured 0 hard flips with the ladder.
 *
 * Same SwiftShader-only caveat as above: counts are bit-stable per machine,
 * not across machines.
 */
test('idle repaints do not flip pixels on coincident linework (axes, sketch lines)', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  const results = await page.evaluate(() => {
    const t = window.__hew_test!
    const poses = {
      // Cube fills the view; +Z shares the near vertical cube edge on screen.
      near: { position: [2.6, -2.2, 1.4] as [number, number, number], target: [0.3, 0.3, 0.5] as [number, number, number] },
      // ~34 m out: depth quanta are coarse AND two axis halves extend behind
      // the camera — the regime that exercised both instability mechanisms.
      far: { position: [26, -22, 14] as [number, number, number], target: [0.3, 0.3, 0.5] as [number, number, number] },
    }
    const eps = 2e-6 // rad about Z — deeply sub-pixel at both distances
    const pair = (base: { position: [number, number, number]; target: [number, number, number] }) => {
      const rot = {
        position: [
          base.position[0] * Math.cos(eps) - base.position[1] * Math.sin(eps),
          base.position[0] * Math.sin(eps) + base.position[1] * Math.cos(eps),
          base.position[2],
        ] as [number, number, number],
        target: base.target,
      }
      return t.frameStability(base, rot)
    }

    // Scene 1 — axes over coincident cube edges. Grid off (its shader
    // legitimately re-shades with the camera uniform); axes ON: they are the
    // subject here, unlike the model-only probe above.
    t.drawBox([0, 0, 0], [1, 1, 0], 1)
    t.setGridVisible(false)
    t.setAxesVisible(true)
    const axesNear = pair(poses.near)
    const axesFar = pair(poses.far)

    // Scene 2 — fat sketch lines retracing the cube's native bottom edges
    // (plus the region fill). Axes off to isolate the sketch-vs-edge pair.
    t.drawRectangle([0, 0, 0], [1, 1, 0])
    t.setAxesVisible(false)
    const sketchNear = pair(poses.near)
    const sketchFar = pair(poses.far)

    return { axesNear, axesFar, sketchNear, sketchFar }
  })

  for (const [name, r] of Object.entries(results)) {
    // Sanity: each probe actually rendered at a real size.
    expect(r.width, name).toBeGreaterThan(100)
    expect(r.height, name).toBeGreaterThan(100)
    // Tight bound on the defect signal (all four measured 0 with the fix;
    // the axes cases measured 20-609 without it).
    expect(r.hard, name).toBeLessThanOrEqual(8)
    // Sanity ceiling for AA rounding drift across machines (measured 0-2
    // with the fix; 43-859 with the defect).
    expect(r.differing, name).toBeLessThanOrEqual(300)
  }
})

/**
 * Near-pole orbit clamp. With world-up +Z, a camera within ~1e-4 rad of an
 * exact ±Z pole is ill-conditioned: screen roll tracks the azimuth of the
 * camera's tiny lateral offset, so sub-µm position jitter re-rolls the whole
 * frame on every damping-tail repaint — severe full-viewport shimmer. Only
 * free orbit could reach that regime (the Top/Bottom standard views bake a
 * POLE_TILT = 1e-3 rad margin into their eye); OrbitControls' polar-angle
 * clamp now floors free orbit at the same constant, atan(POLE_TILT).
 *
 * Pinned here: (1) a real middle-drag orbit slammed toward the pole settles
 * at the floor, never below it; (2) programmatic poses below the floor clamp
 * up to it (every camera write funnels through controls.update); (3) the
 * clamped pose repaints stably under the standard sub-pixel delta with the
 * axes on.
 */
test('free orbit cannot reach the pole, and the clamped pose repaints stably', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  const POLE_TILT = 0.001 // mirrors Viewport.tsx (shared by views + clamp)
  const FLOOR = Math.atan(POLE_TILT)

  await page.evaluate(() => {
    const t = window.__hew_test!
    t.drawBox([0, 0, 0], [1, 1, 0], 1)
    t.setGridVisible(false)
    t.setAxesVisible(true)
    t.setCamera({ position: [4, -3, 3], target: [0, 0, 0] })
  })

  // (1) Real orbit input: a huge middle-button drag downward rotates the
  // camera up toward the +Z pole (far more travel than the clamp allows).
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const cx = box.x + box.width / 2
  const cy = box.y + box.height / 2
  await page.mouse.move(cx, cy)
  await page.mouse.down({ button: 'middle' })
  for (let i = 1; i <= 10; i++) {
    await page.mouse.move(cx, cy + i * 60)
  }
  await page.mouse.up({ button: 'middle' })
  // Let the damping tail settle before reading the pose back.
  await page.waitForTimeout(600)

  const tiltOf = (p: number[], t: number[]) => {
    const dx = p[0] - t[0]
    const dy = p[1] - t[1]
    const dz = p[2] - t[2]
    return Math.acos(dz / Math.hypot(dx, dy, dz))
  }
  const dragged = await page.evaluate(() => window.__hew_test!.getCamera())
  const draggedTilt = tiltOf(dragged.position, dragged.target)
  // Lower bound: the clamp held. Upper bound: the drag really drove the
  // camera to the pole (~7 rad of requested polar travel from a 1.03 rad
  // start) and only the clamp stopped it — without it this pose would be at
  // the makeSafe floor (1e-6), and a drag that failed to register would
  // still be at 1.03 rad.
  expect(draggedTilt).toBeGreaterThanOrEqual(FLOOR * 0.99)
  expect(draggedTilt).toBeLessThanOrEqual(0.05)

  // (2) Programmatic pose 1000× inside the floor clamps up to it exactly.
  const settled = await page.evaluate(() => {
    const t = window.__hew_test!
    const D = 6
    const tilt = 1e-6
    t.setCamera({
      position: [D * Math.sin(tilt) * Math.cos(0.3), D * Math.sin(tilt) * Math.sin(0.3), D * Math.cos(tilt)],
      target: [0, 0, 0],
    })
    return t.getCamera()
  })
  const settledTilt = tiltOf(settled.position, settled.target)
  expect(settledTilt).toBeGreaterThanOrEqual(FLOOR * 0.99)
  expect(settledTilt).toBeLessThanOrEqual(FLOOR * 1.5)

  // (3) The clamped pose itself repaints stably (axes on — the worst case).
  const result = await page.evaluate(() => {
    const t = window.__hew_test!
    const D = 6
    const tilt = 1e-6 // clamps to the floor inside setCamera
    const eps = 2e-6
    const at = (az: number) => ({
      position: [
        D * Math.sin(tilt) * Math.cos(az),
        D * Math.sin(tilt) * Math.sin(az),
        D * Math.cos(tilt),
      ] as [number, number, number],
      target: [0, 0, 0] as [number, number, number],
    })
    return t.frameStability(at(0.3), at(0.3 + eps))
  })
  expect(result.width).toBeGreaterThan(100)
  expect(result.hard).toBeLessThanOrEqual(8)
  expect(result.differing).toBeLessThanOrEqual(300)
})

/**
 * PREVIEW and REGION_FILL deliberately share ladder rung -2 (depthPolicy.ts):
 * an active rubber-band drawn coplanar over an existing region fill —
 * re-drawing across a closed region, an entirely ordinary gesture — is a
 * same-bias depth encounter. It is measured stable (0 hard / 0 differing at
 * both a 5 m and a 45 m pose): the fill never writes depth and only blends a
 * low-alpha tint, and the same-bias test resolves consistently across
 * sub-pixel repaints. This spec pins that pairing; if the rungs are ever
 * split or the fill starts writing depth, it documents the contract the
 * ladder must keep honoring.
 *
 * The gesture is driven through real input (keyboard shortcut + mouse on the
 * canvas), then held open while frameStability moves only the camera —
 * pointer events don't fire during the captures, so the preview geometry
 * stays put in world space.
 */
test('an active rubber-band held over a region fill repaints stably (shared -2 rung)', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  const CAMERA: CameraParams = {
    position: { x: 4, y: -3, z: 3 },
    target: { x: 0.5, y: 0.5, z: 0 },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.01,
    far: 100,
  }

  await page.evaluate((cam) => {
    const t = window.__hew_test!
    t.drawRectangle([0, 0, 0], [1, 1, 0]) // committed region + translucent fill
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.setCamera({
      position: [cam.position.x, cam.position.y, cam.position.z],
      target: [cam.target.x, cam.target.y, cam.target.z],
      up: [cam.up.x, cam.up.y, cam.up.z],
      fovDeg: cam.fovDeg,
    })
  }, CAMERA)

  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  const vp = buildViewProjection(CAMERA, rect.width / rect.height)
  const px = (x: number, y: number, z: number) => {
    const p = worldToPagePixel({ x, y, z }, vp, rect)
    if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
    return p
  }

  // Rectangle tool: anchor inside the fill, rubber-band held open across its
  // interior (clear of the committed boundary lines, so the fill itself is
  // the coincident partner).
  await page.keyboard.press('r')
  const a = px(0.1, 0.1, 0)
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.up()
  const b = px(0.9, 0.6, 0)
  await page.mouse.move(b.x, b.y)

  // Sanity: the gesture is really open (nothing committed yet).
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)

  const results = await page.evaluate((cam) => {
    const t = window.__hew_test!
    const eps = 2e-6
    const pair = (base: { position: [number, number, number]; target: [number, number, number] }) => {
      const rot = {
        position: [
          base.position[0] * Math.cos(eps) - base.position[1] * Math.sin(eps),
          base.position[0] * Math.sin(eps) + base.position[1] * Math.cos(eps),
          base.position[2],
        ] as [number, number, number],
        target: base.target,
      }
      return t.frameStability(base, rot)
    }
    const near: [number, number, number] = [cam.position.x, cam.position.y, cam.position.z]
    const far: [number, number, number] = [near[0] * 9, near[1] * 9, near[2] * 9]
    const target: [number, number, number] = [cam.target.x, cam.target.y, cam.target.z]
    return {
      near: pair({ position: near, target }),
      far: pair({ position: far, target }),
    }
  }, CAMERA)

  for (const [name, r] of Object.entries(results)) {
    expect(r.width, name).toBeGreaterThan(100)
    // Measured 0 hard / 0 differing at both poses; same bounds as the rest
    // of this file so cross-machine AA drift has headroom.
    expect(r.hard, name).toBeLessThanOrEqual(8)
    expect(r.differing, name).toBeLessThanOrEqual(300)
  }
})
