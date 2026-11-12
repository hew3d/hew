import { test, expect } from '@playwright/test'

/**
 * Playtest III: a Line chain started on a top-face EDGE point — a corner,
 * a midpoint, anywhere along the edge — must draw on the face, not fall
 * into ground mode with every later point "projected" to z = 0. The pick
 * ray through a boundary point misses both faces that meet there, so the
 * face pick now probes a hair to each side. Real Iso view, real clicks.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pixel sampling is only stable on pinned SwiftShader')

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('a first click on any top-edge point keeps the chain on the top face', async ({ page }) => {
  test.setTimeout(120_000)
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.drawBox([0, 0, 0], [1, 1, 0], 0.5)
  })
  await page.getByRole('button', { name: 'Camera' }).click()
  await page.getByText('Standard Views', { exact: true }).hover()
  await page.getByText('Iso', { exact: true }).first().click()
  await page.getByRole('button', { name: 'Camera' }).click()
  await page.getByText('Zoom Extents', { exact: true }).click()
  await page.evaluate(
    () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))),
  )
  const canvas = (await page.locator('canvas').first().boundingBox())!
  const toPage = async (w: [number, number, number]) => {
    const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w as [number, number, number]), w)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  const projected = () => page.locator('text=projected').count()

  const edges: [[number, number, number], [number, number, number]][] = [
    [[0, 0, 0.5], [0, 1, 0.5]],
    [[0, 0, 0.5], [1, 0, 0.5]],
    [[0, 1, 0.5], [1, 1, 0.5]],
    [[1, 0, 0.5], [1, 1, 0.5]],
  ]
  const failures: string[] = []
  for (const [a, b] of edges) {
    for (const t of [0, 0.25, 0.5, 0.75, 1]) {
      const p1: [number, number, number] = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, 0.5]
      await page.getByRole('radio', { name: 'Line' }).click()
      const s = await toPage(p1)
      await page.mouse.move(s.x, s.y)
      await page.mouse.down()
      await page.mouse.up()
      const q = await toPage([0.5, 0.5, 0.5])
      await page.mouse.move(q.x, q.y)
      await page.waitForTimeout(40)
      if ((await projected()) > 0) failures.push(JSON.stringify(p1))
      await page.keyboard.press('Escape')
      await page.keyboard.press('Escape')
    }
  }
  expect(failures, 'first clicks that fell into ground mode').toEqual([])
})
