import { test, expect } from '@playwright/test'
import { settleFrame } from '../helpers/render'

/**
 *  — visual-regression golden specs beyond the single box.
 *
 * Runs ONLY in the pinned `visual` Playwright project (800×600, DPR=1,
 * SwiftShader software GL) — see playwright.config.ts and e2e/visual/README.md.
 * These specs are intentionally pixel-sparse: logic regressions belong one
 * layer down in the harness specs; goldens catch shading, material, and camera
 * drift that logic can't see.
 *
 * THREE SCENES:
 *
 *   1. multi-object.png — two cubes at different positions under a fixed camera.
 *      Proves the renderer draws multiple independent solids (no Z-fighting,
 *      no missing geometry, correct edge outlines).
 *
 *   2. materials.png — two boxes painted with different solid-color materials
 *      (one with alpha < 1 for transparency). Proves the material pipeline:
 *      face-color assignment, transparency blending, and per-object base material.
 *
 *   3. guides.png — a single box with two guide lines and a guide point overlaid.
 *      Proves guide rendering: dashed lines (LineDashedMaterial), point cross
 *      markers, and the purple on-guide cue layer.
 *
 * GOLDEN MANAGEMENT:
 *   Goldens are committed as `multi.spec.ts-snapshots/<name>-visual-linux.png`.
 *   DO NOT commit PNGs generated on a macOS dev machine — they will not match
 *   what the pinned Linux CI runner (SwiftShader) rasterizes. Run on the pinned
 *   runner:
 *     pnpm --dir app exec playwright test --project=visual --update-snapshots
 *   See e2e/visual/README.md for the golden contract.
 */

declare global {
  interface Window {
    __hew_test?: import('../../src/test/harness').HewTestHarness
  }
}

// Fixed camera framing: a medium-distance angle that shows the XY ground plane
// and gives depth to multiple objects. Shared across all three scenes so goldens
// compare fairly.
const CAMERA = { position: [10, 8, 10], target: [1.5, 1.5, 1], up: [0, 0, 1], fovDeg: 45 } as const

// ---------------------------------------------------------------------------
// 1. Multi-object scene
// ---------------------------------------------------------------------------

test('multi-object scene (golden)', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setCamera(cam)

    // Two boxes at different positions and heights.
    h.drawBox([0, 0, 0], [2, 2, 0], 1) // short box
    h.drawBox([3, 0, 0], [5, 2, 0], 3) // tall box
  }, CAMERA)

  await settleFrame(page)

  const canvas = page.locator('canvas').first()
  await expect(canvas).toBeVisible()
  await expect(canvas).toHaveScreenshot('multi-object.png', {
    mask: [page.getByTestId('floating-panel')],
  })
})

// ---------------------------------------------------------------------------
// 2. Materials / transparency
// ---------------------------------------------------------------------------

test('materials and transparency (golden)', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setCamera(cam)

    // Draw two boxes.
    const boxA = h.drawBox([0, 0, 0], [2, 2, 0], 2)
    const boxB = h.drawBox([3, 0, 0], [5, 2, 0], 1.5)

    // Paint each with a distinct base material via the harness methods
    // (add_material + set_object_material under the hood):
    //   - boxA: opaque warm orange — exercises solid face-color assignment.
    //   - boxB: translucent blue (alpha < 255) — exercises the transparency
    //     blend path, so the ground grid + boxA show through it.
    const orange = h.addMaterial('orange', 220, 110, 40, 255)
    const blueGlass = h.addMaterial('blue-glass', 60, 120, 220, 110)
    h.paintObject(boxA, orange)
    h.paintObject(boxB, blueGlass)

    return { boxA, boxB }
  }, CAMERA)

  await settleFrame(page)

  const canvas = page.locator('canvas').first()
  await expect(canvas).toBeVisible()
  await expect(canvas).toHaveScreenshot('materials.png', {
    mask: [page.getByTestId('floating-panel')],
  })
})

// ---------------------------------------------------------------------------
// 3. Guide overlay
// ---------------------------------------------------------------------------

test('guide lines and points overlay (golden)', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setCamera(cam)

    // One box as a reference solid.
    h.drawBox([0, 0, 0], [2, 2, 0], 1)

    // Two guide lines: one along X at y=3, one along Y at x=3.
    h.addGuideLine(0, 3, 0, 1, 0, 0)
    h.addGuideLine(3, 0, 0, 0, 1, 0)

    // One guide point at (3, 3, 0).
    h.addGuidePoint(3, 3, 0)
  }, CAMERA)

  await settleFrame(page)

  const canvas = page.locator('canvas').first()
  await expect(canvas).toBeVisible()
  await expect(canvas).toHaveScreenshot('guides.png', {
    mask: [page.getByTestId('floating-panel')],
  })
})
