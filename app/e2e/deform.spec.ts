import { test, expect, type Page } from '@playwright/test'
import type { CameraParams } from './helpers/projectWorldToScreen'
import { activateFromToolsMenu } from './helpers/offset'
import { aim, clickAt, moveTo, sweep, activateFromMenu, shot, type Ctx } from './helpers/shapes'

/**
 * Deform — Twist, Taper and Bend, driven with REAL pointer and keyboard
 * input on a 0.1 x 0.1 x 1 m bar. The bar is built through the harness and
 * selected; the tool's gizmo, face click, drag and commit are real events.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 1.2, y: -2.2, z: 1.6 },
  target: { x: 0, y: 0, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

async function scene(page: Page): Promise<{ ctx: Ctx; id: string }> {
  const ctx = await aim(page, CAMERA)
  const id = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([-0.05, -0.05, 0], [0.05, 0.05, 0], 1)
    h.selectObjects([id])
    return id
  })
  return { ctx, id }
}

const extentOfTop = (page: Page, id: string) =>
  page.evaluate((id) => {
    const top = window.__hew_test!.getObjectVertices(id).filter((v) => v[2] > 0.99)
    return {
      count: top.length,
      maxX: Math.max(...top.map((v) => Math.abs(v[0]))),
      maxY: Math.max(...top.map((v) => Math.abs(v[1]))),
    }
  }, id)

test('Twist: click the top of the box, sweep the pointer, click; Undo restores the bar exactly', async ({ page }) => {
  const { ctx, id } = await scene(page)
  await activateFromMenu(page, 'Twist', /Click the end of the box to twist/)
  await shot(page, 'deform-1-gizmo')

  await moveTo(page, ctx, 0.04, 0, 1)
  await shot(page, 'deform-2-hover-face')
  await clickAt(page, ctx, 0.04, 0, 1)
  await expect(page.getByText(/Move to twist, click to finish/)).toBeVisible()

  // A quarter of the way round counterclockwise: about 45 degrees.
  await sweep(page, ctx, [[0.0377, 0.013, 1], [0.0283, 0.0283, 1]])
  await shot(page, 'deform-3-twist-ghost')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(300)

  const top = await extentOfTop(page, id)
  // An untwisted square of half-side 0.05 reaches 0.05 along each axis; turned
  // about 45 degrees its corners sit near 0.0707.
  expect(top.maxX).toBeGreaterThan(0.058)
  expect(await page.evaluate((id) => window.__hew_test!.isObjectSolid(id), id)).toBe(true)
  await shot(page, 'deform-4-twisted')

  await page.keyboard.press('Control+z')
  await page.waitForTimeout(300)
  const b = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
  expect(b[0]).toBeCloseTo(-0.05, 5)
  expect(b[3]).toBeCloseTo(0.05, 5)
})

test('Twist: a typed 45 commits exactly 45 degrees; retype redoes it', async ({ page }) => {
  const { ctx, id } = await scene(page)
  await activateFromMenu(page, 'Twist', /Click the end of the box to twist/)
  await clickAt(page, ctx, 0.04, 0, 1)
  await page.keyboard.type('45')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  let top = await extentOfTop(page, id)
  expect(top.maxX).toBeCloseTo(0.05 * Math.SQRT2, 3)

  await page.keyboard.type('90')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  top = await extentOfTop(page, id)
  expect(top.maxX).toBeCloseTo(0.05, 3) // a quarter turn maps the square onto itself
})

test('Taper: the pointer sets the end size; a typed percentage is exact', async ({ page }) => {
  const { ctx, id } = await scene(page)
  await activateFromToolsMenu(page, 'Taper')
  await expect(page.getByText(/Click the end of the box to taper/)).toBeVisible()
  await clickAt(page, ctx, 0.04, 0, 1)
  await page.keyboard.type('50%')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  const top = await extentOfTop(page, id)
  expect(top.maxX).toBeCloseTo(0.025, 3)
  await shot(page, 'deform-5-taper')
})

test('Bend: sideways drag bends the bar toward that side', async ({ page }) => {
  const { ctx, id } = await scene(page)
  await activateFromToolsMenu(page, 'Bend')
  await expect(page.getByText(/Click the end of the box to bend/)).toBeVisible()
  await clickAt(page, ctx, 0.04, 0, 1)
  await sweep(page, ctx, [[0.2, 0, 1], [0.4, 0, 1]])
  await shot(page, 'deform-6-bend-ghost')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(300)
  const b = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
  // The tip moved sideways and came down: the bar is no longer 1 m tall.
  expect(b[5]).toBeLessThan(0.97)
  expect(Math.max(Math.abs(b[0]), Math.abs(b[3]), Math.abs(b[1]), Math.abs(b[4]))).toBeGreaterThan(0.1)
})
