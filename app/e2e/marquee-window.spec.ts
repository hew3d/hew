import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Marquee selection direction semantics, driven with REAL pointer events:
 *  - left→right is a WINDOW (blue, solid, "Window" tag): an object is picked
 *    when it is fully enclosed or at least 80% covered;
 *  - right→left is a CROSSING (green, dashed, "Crossing" tag): anything the
 *    box touches;
 *  - a browser that delivers a button-up pointermove ahead of the pointerup
 *    must not drop the drag.
 */

const CAMERA = {
  position: [2, -9, 7] as [number, number, number],
  target: [2, 0.5, 0.3] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 50,
}

async function setup(page: Page): Promise<{ a: string; hull: { minX: number; maxX: number; minY: number; maxY: number } }> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  const a = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.waitForTimeout(150)
  const box = (await page.locator('canvas').first().boundingBox())!
  const xs: number[] = []
  const ys: number[] = []
  for (const x of [0, 1]) for (const y of [0, 1]) for (const z of [0, 1]) {
    const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w as [number, number, number]), [x, y, z])
    xs.push(box.x + p.x)
    ys.push(box.y + p.y)
  }
  return { a, hull: { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) } }
}

async function drag(page: Page, x0: number, y0: number, x1: number, y1: number, beforeUp?: () => Promise<void>): Promise<void> {
  await page.mouse.move(x0, y0)
  await page.mouse.down()
  await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 5 })
  await page.mouse.move(x1, y1, { steps: 5 })
  if (beforeUp) await beforeUp()
  await page.mouse.up()
}

async function clearSel(page: Page): Promise<void> {
  const box = (await page.locator('canvas').first().boundingBox())!
  // Top-right, below the view chips: the floating selection toolbar can reach
  // the bottom corners on narrower-font renderers and swallow the click.
  await page.mouse.click(box.x + box.width - 40, box.y + 120)
}

const sel = (page: Page) => page.evaluate(() => window.__hew_test!.getSelection().map((s) => s.id))

test('window drag: enclosing selects; covering 80%+ selects; covering a sliver does not', async ({ page }) => {
  const { a, hull } = await setup(page)
  const m = 30
  const w = hull.maxX - hull.minX
  const h = hull.maxY - hull.minY

  await drag(page, hull.minX - m, hull.minY - m, hull.maxX + m, hull.maxY + m)
  expect(await sel(page)).toEqual([a])

  await clearSel(page)
  expect(await sel(page)).toEqual([])

  // ~72% of the bounding box (left 85% x bottom 85%): short of 80%, not picked.
  await drag(page, hull.minX - m, hull.minY - m, hull.minX + w * 0.85, hull.minY + h * 0.85)
  expect(await sel(page)).toEqual([])

  // ~90% of the bounding box (95% x 95%), starting in empty air: picked.
  await drag(page, hull.minX - m, hull.minY - m, hull.minX + w * 0.95, hull.minY + h * 0.95)
  expect(await sel(page)).toEqual([a])

  await clearSel(page)
  expect(await sel(page)).toEqual([])

  // ~12% of the bounding box: not enough.
  await drag(page, hull.minX - m, hull.minY - m, hull.minX + w * 0.35, hull.minY + h * 0.35)
  expect(await sel(page)).toEqual([])
})

test('overlay and status hint tell window from crossing', async ({ page }) => {
  const { hull } = await setup(page)
  const overlay = page.getByTestId('viewport-marquee-overlay')
  const cx = (hull.minX + hull.maxX) / 2
  await page.mouse.move(hull.minX - 40, hull.minY - 40)
  await page.mouse.down()
  await page.mouse.move(cx, hull.minY + 20, { steps: 6 })
  await expect(overlay).toContainText('Window')
  await expect(page.getByText(/Window selection:/)).toBeVisible()
  expect(await overlay.evaluate((e) => getComputedStyle(e).borderTopStyle)).toBe('solid')
  await page.mouse.move(hull.minX - 80, hull.minY + 20, { steps: 6 })
  await expect(overlay).toContainText('Crossing')
  await expect(page.getByText(/Crossing selection:/)).toBeVisible()
  expect(await overlay.evaluate((e) => getComputedStyle(e).borderTopStyle)).toBe('dashed')
  await page.mouse.up()
})

test('a button-up move ahead of the pointerup still completes the marquee', async ({ page }) => {
  const { a, hull } = await setup(page)
  const m = 30
  const x1 = hull.maxX + m
  const y1 = hull.maxY + m
  await drag(page, hull.minX - m, hull.minY - m, x1, y1, async () => {
    await page.evaluate(([x, y]) => {
      const c = document.querySelector('canvas')!
      c.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, pointerId: 1, pointerType: 'mouse', buttons: 0, button: -1, clientX: x, clientY: y }))
    }, [x1, y1])
  })
  expect(await sel(page)).toEqual([a])
})
