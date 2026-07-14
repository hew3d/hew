import { test, expect } from '@playwright/test'

/**
 * Materials pane wiring — the cross-layer behavior a component test can't reach
 * (App → Viewport → ToolController → PaintTool).
 *
 * Contract: picking a material in the palette makes it current AND activates
 * the Paint tool, so the next click paints with it (Ctrl/Cmd-click fills the
 * whole object). This replaced the old "Fill selected object" button, which
 * must no longer exist.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('selecting a material activates the Paint tool; no Fill button remains', async ({ page }) => {
  // Reveal the Materials tray if collapsed.
  const defaultSwatch = page.getByTitle('Default (unpainted)')
  if (!(await defaultSwatch.isVisible().catch(() => false))) {
    await page.getByText('Materials', { exact: true }).first().click()
  }
  await expect(defaultSwatch).toBeVisible()

  // The removed button must not be present.
  await expect(page.getByRole('button', { name: /fill selected object/i })).toHaveCount(0)

  // Picking a swatch activates Paint — proven by the Paint tool's own status
  // hint (only shown when PaintTool is the active tool) appearing.
  await defaultSwatch.click()
  await expect(page.getByText(/Click a face to paint it with the current material/i)).toBeVisible()
})
