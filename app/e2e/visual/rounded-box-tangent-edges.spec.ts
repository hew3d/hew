import { test, expect } from '@playwright/test'
import { settleFrame } from '../helpers/render'
import { requiresPreview } from '../helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('fillet')

/**
 * Visual goldens for a filleted box: the tangent seams where each rounded
 * strip meets a flat face stay out of sight by default and show as thin
 * lines with View > Tangent Edges. Runs only in the pinned `visual` project;
 * the PNGs are rasterized by CI's SwiftShader (the "Regen Visual Goldens"
 * workflow), never locally — see e2e/visual/README.md.
 */

declare global {
  interface Window {
    __hew_test?: import('../../src/test/harness').HewTestHarness
  }
}

const CAMERA = { position: [4.2, -3.4, 3], target: [0.6, 0.6, 0.3], up: [0, 0, 1], fovDeg: 45 } as const

test('a filleted box hides its tangent edges, then shows them (goldens)', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1.2, 1.2, 0], 0.6)
    // Every sharp edge of the top face takes a 0.15 round.
    h.filletEdges(
      box,
      [
        [0.6, 0, 0.6],
        [1.2, 0.6, 0.6],
        [0.6, 1.2, 0.6],
        [0, 0.6, 0.6],
      ],
      '{"kind":"round","radius":0.15,"segments":8}',
    )
    h.setCamera(cam)
  }, CAMERA)
  await settleFrame(page)

  const canvas = page.locator('canvas').first()
  await expect(canvas).toBeVisible()
  await expect(canvas).toHaveScreenshot('rounded-box-tangent-edges-hidden.png')

  const menuItem = () => page.getByTestId('menu-bar').getByText(/^✓?Tangent Edges$/)
  await page.getByTestId('menu-bar').getByRole('button', { name: 'View' }).click()
  await menuItem().click()
  await settleFrame(page)
  await expect(canvas).toHaveScreenshot('rounded-box-tangent-edges-shown.png')
})
