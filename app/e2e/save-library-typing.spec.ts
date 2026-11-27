import { test, expect, type Page } from '@playwright/test'

/**
 * GitHub #36 through the REAL viewport key handler: digits typed into the
 * Save to Library name field must land in the field and must not retype the
 * rectangle that was just drawn (the Rectangle tool is still active and
 * captures digits for its typed-size buffer). The unit test in
 * Viewport.test.tsx pins the predicate; this pins the three gates in
 * Viewport.tsx that call it.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

async function pagePoint(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function click(page: Page, w: V3): Promise<void> {
  const at = await pagePoint(page, w)
  await page.mouse.move(at.x - 3, at.y + 2)
  await page.waitForTimeout(60)
  await page.mouse.move(at.x, at.y)
  await page.waitForTimeout(120)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

test('digits typed into the Save to Library name never retype the last rectangle', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [1, 1, 0], 0.5)
    h.setCamera({ position: [0.6, -1.2, 3.2], target: [0.7, 0.5, 0.5], up: [0, 1, 0], fovDeg: 45 })
  })
  await page.waitForTimeout(250)

  // Real pointer events: a rectangle on the box's top, Rectangle tool left active.
  await page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Rectangle' }).click()
  await click(page, [0.25, 0.25, 0.5])
  await click(page, [0.75, 0.75, 0.5])
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  const sketchBounds = () =>
    page.evaluate(() => {
      const h = window.__hew_test!
      return { hash: h.getModelStateHash(), sketches: h.getSketchIds().length }
    })
  const before = await sketchBounds()

  // Select the box so the dock's Save to Library verb is available, then open it.
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.selectNodes([{ kind: 'object', id: h.getObjectIds()[0] }])
  })
  await page.getByRole('button', { name: 'Save to Library' }).click()
  const prompt = page.getByRole('dialog', { name: 'Save to Library' })
  await expect(prompt).toBeVisible()
  const name = prompt.getByRole('textbox')
  await name.click()
  await page.keyboard.press('ControlOrMeta+A') // replace the prefilled default name
  await page.keyboard.type('box 2')
  await expect(name).toHaveValue('box 2')

  // The rectangle (and the whole model) is untouched by the typed digit.
  expect(await sketchBounds()).toEqual(before)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  await page.keyboard.press('Escape')
  await expect(prompt).not.toBeVisible()
})
