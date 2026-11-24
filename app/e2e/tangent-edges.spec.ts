import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * View ▸ Tangent Edges: a rounded rectangle extrusion has tangent joints where
 * each flat wall meets a corner arc. Those joints are neither hard edges nor
 * facet seams, so the viewport draws nothing there until View ▸ Tangent Edges
 * is on; then thin (half-strength) lines appear. Driven with REAL menu clicks;
 * the harness only builds the model and reads back the renderer's line state.
 */

const CAMERA = {
  position: [3.2, -3.4, 2.6] as [number, number, number],
  target: [0.6, 0.4, 0.3] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

test('View > Tangent Edges shows and hides the wall-to-corner lines', async ({ page }, testInfo) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })

  // A 1.2 x 0.8 rounded rectangle (corner radius 0.2), extruded 0.6.
  await page.evaluate((cam) => {
    const h = window.__hew_test!
    const { sketch, region } = h.drawRoundedRectangle([0, 0, 0], [1.2, 0.8, 0], 0.2)
    h.extrudeRegion(sketch, region, 0.6)
    h.setCamera(cam)
  }, CAMERA)
  await page.waitForTimeout(300)

  // The four wall-to-corner joints x 2 (top rim and bottom rim are separate
  // edges; the vertical ones are the tangent joints): eight tangent segments
  // at least, and the line exists but is hidden by default.
  const before = await page.evaluate(() => window.__hew_test!.getTangentEdgeInfo())
  expect(before.lines).toBeGreaterThan(0)
  expect(before.segments).toBeGreaterThanOrEqual(8)
  expect(before.visible).toBe(0)
  await page.screenshot({ path: testInfo.outputPath('tangent-edges-off.png') })

  // The label shares its span with the check mark, so match either state.
  const menuItem = () => page.getByTestId('menu-bar').getByText(/^✓?Tangent Edges$/)
  await page.getByTestId('menu-bar').getByRole('button', { name: 'View' }).click()
  await expect(menuItem()).not.toContainText('✓')
  await menuItem().click()
  await page.waitForTimeout(200)

  const on = await page.evaluate(() => window.__hew_test!.getTangentEdgeInfo())
  expect(on.visible).toBe(on.lines)
  expect(on.segments).toBe(before.segments)
  await page.screenshot({ path: testInfo.outputPath('tangent-edges-on.png') })

  // The menu check follows, and a second click turns the lines off again.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'View' }).click()
  await expect(menuItem()).toContainText('✓')
  await menuItem().click()
  await page.waitForTimeout(200)
  const off = await page.evaluate(() => window.__hew_test!.getTangentEdgeInfo())
  expect(off.visible).toBe(0)
  await page.screenshot({ path: testInfo.outputPath('tangent-edges-off-again.png') })
})
