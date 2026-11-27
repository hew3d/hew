import { test, expect } from '@playwright/test'
import { clickWorld, moveToWorld, pagePoint, setup, shot, volume } from './helpers/offset'
import { WALL_SKIP, isProductionShape } from './helpers/preview'

/**
 * Eraser, driven with real pointer events, in both builds: a plain click
 * erases a drawn line, a drag sweeps several as one undo step, and a plain
 * click on a solid's edge only explains. The crease half (Ctrl-click, which
 * only means something with Smooth) is in eraser-crease.spec.ts; here the
 * production-shape build checks that the modifier does nothing at all (the
 * preview build skips that one test with the wall's reason).
 */

const CAMERA = { position: [3, -3.4, 2.4] as [number, number, number], target: [0.5, 0.5, 0.4] as [number, number, number] }

test('Eraser: a click erases a drawn line; a drag across three erases all of them in one undo step', async ({ page }) => {
  await setup(page, { position: [3.5, -3, 6], target: [3.5, 3.5, 0] })
  const lines: string[] = []
  for (const y of [3, 4, 5]) {
    const c = await page.evaluate((y) => window.__hew_test!.drawLineChain([[3, y, 0], [4, y, 0]]), y)
    lines.push(c.sketch)
  }
  const edgeCount = async (): Promise<number> => {
    let n = 0
    for (const s of lines) {
      try {
        n += (await page.evaluate((s) => window.__hew_test!.getSketchEdgeIds(s), s)).length
      } catch {
        // an emptied sketch is gone
      }
    }
    return n
  }
  expect(await edgeCount()).toBe(3)

  await page.keyboard.press('e')
  await page.waitForTimeout(100)
  await clickWorld(page, [3.5, 3, 0])
  await page.waitForTimeout(200)
  expect(await edgeCount()).toBe(2)
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  expect(await edgeCount()).toBe(3)

  // Drag from below the first line to above the last, across x = 3.5.
  const from = await pagePoint(page, [3.5, 2.5, 0])
  const to = await pagePoint(page, [3.5, 5.5, 0])
  await page.mouse.move(from.x, from.y)
  await page.waitForTimeout(60)
  await page.mouse.down()
  const steps = 24
  for (let i = 1; i <= steps; i++) {
    await page.mouse.move(from.x + ((to.x - from.x) * i) / steps, from.y + ((to.y - from.y) * i) / steps)
  }
  await shot(page, 'eraser-sweep')
  await page.mouse.up()
  await page.waitForTimeout(250)
  expect(await edgeCount()).toBe(0)
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  expect(await edgeCount()).toBe(3)
})

test('Eraser: Escape during a drag changes nothing', async ({ page }) => {
  await setup(page, { position: [3.5, -3, 6], target: [3.5, 3.5, 0] })
  const c = await page.evaluate(() => window.__hew_test!.drawLineChain([[3, 3, 0], [4, 3, 0]]))
  await page.keyboard.press('e')
  const from = await pagePoint(page, [3.5, 2.5, 0])
  const to = await pagePoint(page, [3.5, 3.5, 0])
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  for (let i = 1; i <= 12; i++) await page.mouse.move(from.x + ((to.x - from.x) * i) / 12, from.y + ((to.y - from.y) * i) / 12)
  await page.keyboard.press('Escape')
  await page.mouse.up()
  await page.waitForTimeout(200)
  expect((await page.evaluate((s) => window.__hew_test!.getSketchEdgeIds(s), c.sketch)).length).toBe(1)
})

test('Eraser: a plain click on a solid edge erases nothing and says why', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  const before = await volume(page, box)
  await page.keyboard.press('e')
  await page.waitForTimeout(100)
  await moveToWorld(page, [0.5, 0, 0])
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)

  await expect(page.getByText(/Edges of a solid can't be erased/).first()).toBeVisible()
  expect(await volume(page, box)).toBeCloseTo(before, 6)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), box)).toBe(0)
})

test('Eraser: Ctrl-click on a solid edge does nothing crease-related, and no hint mentions creasing', async ({ page }) => {
  // The gated-off half: only the production-shape build has the crease modifier off.
  // The preview build creases on Ctrl-click; eraser-crease.spec.ts drives that.
  test.skip(!isProductionShape, `${WALL_SKIP}the crease modifier is off only in the production-shape build (HEW_PREVIEW=0); creasing is driven in eraser-crease.spec.ts`)
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  const before = await volume(page, box)
  await page.keyboard.press('e')
  await page.waitForTimeout(100)
  // The modifier is ignored: the tool reads as if Ctrl were not held.
  await page.keyboard.down('Control')
  await moveToWorld(page, [1, 0, 0.5])
  await page.waitForTimeout(150)
  await expect(page.getByText(/Edges of a solid can't be erased\.$/).first()).toBeVisible()
  await page.mouse.down()
  await page.mouse.up()
  await page.keyboard.up('Control')
  await page.waitForTimeout(250)
  expect(await volume(page, box)).toBeCloseTo(before, 9)
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), box)).toBe(0)
  await expect(page.getByText(/crease|smooth/i)).toHaveCount(0)
})
