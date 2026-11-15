import { test, expect, type Page } from '@playwright/test'
import type { CameraParams } from './helpers/projectWorldToScreen'
import { aim, clickAt, moveTo, shot, type Ctx } from './helpers/shapes'

/**
 * Follow Me twist and end size (true-curves.md §5.2), typed during the drag
 * with REAL pointer and keyboard input: press on the profile, type `45deg`
 * and Enter (the twist is set, the drag goes on), release to sweep the full
 * path. The end cap is the start square turned 45 degrees. Retyping `90deg`
 * after the commit redoes the sweep; one Undo removes it.
 *
 * Path: a straight 3 m ground segment (0,0,0)->(3,0,0); profile: a 0.2 m
 * square at its near end (plane x = 0), the same scene as
 * follow-me-partial-sweep.spec.ts.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 1.5, y: -3.6, z: 2.4 },
  target: { x: 1.5, y: 0, z: 0.1 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.01,
  far: 1000,
}

async function setup(page: Page): Promise<Ctx> {
  const ctx = await aim(page, CAMERA)
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawLineChain([[0, 0, 0], [3, 0, 0]])
    const r = h.drawRectangle([-0.1, -0.1, 0], [0.1, 0.1, 0])
    h.rotateSketch(r.sketch, 90, [0, 1, 0], [0, 0, 0])
  })
  await page.keyboard.press(' ')
  await clickAt(page, ctx, 1.5, 0, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
  return ctx
}

/** Vertices at the far end (x = 3): half-extents across the path. */
const endCap = (page: Page, id: string) =>
  page.evaluate((id) => {
    const v = window.__hew_test!.getObjectVertices(id).filter((p) => p[0] > 2.99)
    return {
      count: v.length,
      maxY: Math.max(...v.map((p) => Math.abs(p[1]))),
      maxZ: Math.max(...v.map((p) => Math.abs(p[2]))),
    }
  }, id)

test('Follow Me: a typed twist during the drag turns the end cap; retype redoes it; Undo removes it', async ({ page }) => {
  const ctx = await setup(page)
  await moveTo(page, ctx, 0, 0, 0)
  await page.mouse.down() // arm the drag on the profile
  await page.keyboard.type('45deg')
  await page.keyboard.press('Enter')
  await expect(page.getByText(/Twist 45°/)).toBeVisible()
  await shot(page, 'follow-me-twist-1-option-set')
  await page.mouse.up() // release at the press point: the full sweep
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  const id = await page.evaluate(() => window.__hew_test!.getObjectIds()[0])
  expect(await page.evaluate((id) => window.__hew_test!.isObjectSolid(id), id)).toBe(true)
  let cap = await endCap(page, id)
  expect(cap.maxY).toBeCloseTo(0.1 * Math.SQRT2, 3) // the square turned 45 degrees
  await shot(page, 'follow-me-twist-2-twisted-bar')

  // Retype: 90 degrees maps the square onto itself.
  await page.keyboard.type('90deg')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  cap = await endCap(page, ids[0])
  expect(cap.maxY).toBeCloseTo(0.1, 3)

  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 0)
})

test('Follow Me: a typed end size scales the far end', async ({ page }) => {
  const ctx = await setup(page)
  await moveTo(page, ctx, 0, 0, 0)
  await page.mouse.down()
  await page.keyboard.type('50%')
  await page.keyboard.press('Enter')
  await page.mouse.up()
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  const id = await page.evaluate(() => window.__hew_test!.getObjectIds()[0])
  const cap = await endCap(page, id)
  expect(cap.maxY).toBeCloseTo(0.05, 3)
  await shot(page, 'follow-me-twist-3-end-size')
})
