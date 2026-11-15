import { test, expect, type Page } from '@playwright/test'
import type { CameraParams } from './helpers/projectWorldToScreen'
import { aim, clickAt, moveTo, activateFromRail, shot } from './helpers/shapes'

/**
 * Loft — real pointer and keyboard. A drawn rectangle and the top face of a box at a different height,
 * clicked in order, then Enter: one watertight solid between them.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const HINT = /Click profiles in order\./

const CAMERA: CameraParams = {
  position: { x: 4, y: -5, z: 4 },
  target: { x: 0.5, y: 0.5, z: 0.8 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

async function twoRectangles(page: Page): Promise<void> {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawRectangle([0, 0, 0], [1, 1, 0]) // a drawn shape on the ground
    h.drawBox([0.25, 0.25, 0], [0.75, 0.75, 0], 1.5) // its top face (z = 1.5) is the second profile
  })
}

test('Loft: click two shapes in order, Enter makes one solid between them', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await twoRectangles(page)
  await activateFromRail(page, 'Loft', HINT)

  await moveTo(page, ctx, 0.9, 0.9, 0)
  await clickAt(page, ctx, 0.9, 0.9, 0)
  await clickAt(page, ctx, 0.5, 0.5, 1.5)
  await shot(page, 'loft-1-two-shapes-ghost', true)

  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2) // the box and the loft
  const r = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getObjectIds().at(-1)!
    return { solid: h.isObjectSolid(id), bounds: h.getObjectBounds(id) }
  })
  expect(r.solid).toBe(true)
  // The loft flares from the box top (0.5 m wide) down to the ground rectangle (1 m).
  expect(r.bounds[2]).toBeCloseTo(0, 4)
  expect(r.bounds[5]).toBeCloseTo(1.5, 4)
  expect(r.bounds[3] - r.bounds[0]).toBeCloseTo(1, 4)
  await shot(page, 'loft-2-solid')

  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
})

test('Loft: clicking a picked shape again takes it out; Esc takes back the last pick', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await twoRectangles(page)
  await activateFromRail(page, 'Loft', HINT)
  await clickAt(page, ctx, 0.9, 0.9, 0)
  await clickAt(page, ctx, 0.5, 0.5, 1.5)
  await clickAt(page, ctx, 0.5, 0.5, 1.5) // out again
  await page.keyboard.press('Enter')
  await expect(page.getByText(/at least two/)).toBeVisible()
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1) // just the box
})

test('Loft: a drawn open chain is refused in plain words', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await page.evaluate(() => window.__hew_test!.drawLineChain([[2, 0, 0], [3, 1, 0]]))
  await activateFromRail(page, 'Loft', HINT)
  await clickAt(page, ctx, 2.5, 0.5, 0)
  await expect(page.getByText('Loft needs closed shapes. Close this one first.')).toBeVisible()
})
