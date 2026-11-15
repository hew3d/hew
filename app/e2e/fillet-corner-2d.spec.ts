import { test, expect, type Page } from '@playwright/test'
import { settleFrame } from './helpers/render'
import { requiresPreview } from './helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('fillet')

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Rounding a drawn corner with the Fillet tool: draw a rectangle, Shift+F,
 * click a corner, type 0.1, Enter, then extrude it with Push/Pull. The wall
 * the new arc stands on must come out smooth. Real pointer and keyboard
 * events throughout.
 *
 * Set HEW_E2E_SHOTS to a directory to keep the screenshot there.
 */

const CAMERA = {
  position: [3, -4, 4] as [number, number, number],
  target: [1, 0.5, 0] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function clickWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y, { steps: 3 })
  await page.mouse.down()
  await page.mouse.up()
}

test('Fillet rounds a drawn corner, and the extruded wall follows the arc', async ({ page }, info) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setLengthUnit('m')
    h.drawRectangle([0, 0, 0], [2, 1, 0])
    h.setCamera(cam)
  }, CAMERA)
  await page.waitForTimeout(200)

  await page.keyboard.press('Shift+F')
  const corner = await pagePoint(page, [2, 1, 0])
  await page.mouse.move(corner.x, corner.y, { steps: 3 })
  await expect(page.getByText('Click to round this corner', { exact: false })).toBeVisible()
  await page.mouse.down()
  await page.mouse.up()
  await page.keyboard.type('0.1')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)

  // Push/Pull the region up 0.5.
  await page.keyboard.press('p')
  await clickWorld(page, [1, 0.5, 0])
  await page.keyboard.type('0.5')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(250)

  const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  const b = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids[0])
  expect(b[3] - b[0]).toBeCloseTo(2, 5)
  expect(b[4] - b[1]).toBeCloseTo(1, 5)
  expect(b[5] - b[2]).toBeCloseTo(0.5, 5)
  expect(await page.evaluate((id) => window.__hew_test!.isObjectSolid(id), ids[0])).toBe(true)

  await settleFrame(page)
  const dir = process.env.HEW_E2E_SHOTS
  await page.screenshot({ path: dir ? `${dir}/corner-2d-wall.png` : info.outputPath('corner-2d-wall.png') })
})

test('Over a drawn corner the Edge chip offers only Round and Chamfer', async ({ page }, info) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setLengthUnit('m')
    h.drawRectangle([0, 0, 0], [2, 1, 0])
    h.setCamera(cam)
  }, CAMERA)
  await page.waitForTimeout(200)
  const dir = process.env.HEW_E2E_SHOTS
  const shot = (name: string) => page.screenshot({ path: dir ? `${dir}/${name}` : info.outputPath(name) })

  await page.keyboard.press('Shift+F')
  const chip = page.getByTestId('fillet-profile-chip').getByRole('button', { name: /^Edge/ })
  await chip.click()
  await page.getByRole('menuitemradio', { name: 'Cove', exact: true }).click()
  await expect(chip).toContainText('Cove')

  const away = await pagePoint(page, [1, -1.5, 0])
  const corner = await pagePoint(page, [2, 1, 0])
  await page.mouse.move(corner.x, corner.y, { steps: 3 })
  await expect(page.getByText('Click to round this corner', { exact: false })).toBeVisible()
  await expect(chip).toContainText('Round')
  await chip.click()
  await expect(page.getByRole('menuitemradio', { name: 'Cove', exact: true })).toBeDisabled()
  await expect(page.getByRole('menuitemradio', { name: 'Bullnose', exact: true })).toBeDisabled()
  await expect(page.getByRole('menuitemradio', { name: 'Cove and bead', exact: false })).toBeDisabled()
  await expect(page.getByRole('menuitemradio', { name: 'Chamfer', exact: true })).toBeEnabled()
  await settleFrame(page)
  await shot('corner-chip-round-only.png')
  await chip.click()
  await expect(page.getByRole('menu')).toHaveCount(0)

  await page.mouse.move(away.x, away.y, { steps: 3 })
  await expect(chip).toContainText('Cove')

  // Typing over the corner rounds it with Cove still picked.
  await page.mouse.move(corner.x, corner.y, { steps: 3 })
  await page.keyboard.type('0.1')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  await expect(page.getByText('Drawn corners can be rounded or chamfered.')).toHaveCount(0)
  await expect(page.getByText('Type a radius to adjust', { exact: false })).toBeVisible()
  await page.mouse.move(away.x, away.y, { steps: 3 })
  await expect(chip).toContainText('Cove')
  await settleFrame(page)
  await shot('corner-chip-cove-back.png')
})
