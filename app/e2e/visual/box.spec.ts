import { test, expect } from '@playwright/test'
import { settleFrame } from '../helpers/render'

/**
 *  — visual-regression golden for the viewport render.
 *
 * Runs only in the pinned `visual` project (fixed 800×600 viewport, DPR=1,
 * SwiftShader software GL — see playwright.config.ts). It drives the
 * harness to build one deterministic solid under a fixed camera, then asserts
 * the canvas matches a committed golden PNG (tolerance in the global
 * `expect.toHaveScreenshot`). This is the *render* regression net: it catches
 * shading/tessellation/camera drift that the logic-only harness specs can't see.
 *
 * Goldens are GPU/runner-specific and authoritative on the pinned CI runner.
 * Refresh with `pnpm --dir app e2e:visual:update` (read e2e/visual/README.md
 * first — do it on the pinned runner, not a dev GPU).
 */

declare global {
  interface Window {
    __hew_test?: import('../../src/test/harness').HewTestHarness
  }
}

// The same box + camera as the web smoke, so the golden frames a known solid.
const CAMERA = { position: [8, 6, 8], target: [1, 1, 1], up: [0, 0, 1], fovDeg: 45 } as const

test('viewport renders the canonical push/pulled box (golden)', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })

  await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setCamera(cam)
    const box = h.drawBox([0, 0, 0], [2, 2, 0], 2)
    const pick = h.pickFace([1, 1, 10], [0, 0, -1]) // top face, ray down
    h.pushPull(pick!.object, pick!.face, 1)
    return box
  }, CAMERA)

  await settleFrame(page)

  const canvas = page.locator('canvas').first()
  await expect(canvas).toBeVisible()
  // Mask the floating info panels: they overlap the canvas crop but their text
  // (object names, counts) is incidental to a *render* golden — masking keeps
  // the assertion about shading/tessellation/camera, not panel copy.
  await expect(canvas).toHaveScreenshot('box.png', {
    mask: [page.getByTestId('floating-panel')],
  })
})
