import { test, expect, type Page } from '@playwright/test'

/**
 * #36: a click back on a Line chain's last point finishes the chain, at any
 * pace, on the ground and on a face — no "same point" toast. A double-click
 * whose two presses are too far apart for the Viewport's own click count
 * (but not for the browser's) delivers both presses AND a `dblclick`; the
 * second press finishes the chain and the `dblclick` must not fall through
 * to entering the object under the cursor. Real Iso view, real mouse input.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'the long-gap double-click is a Chromium path (pointerdown detail = 0)')

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

type V3 = [number, number, number]

async function setup(page: Page) {
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.drawBox([0, 0, 0], [1, 1, 0], 0.5)
    // A small ground rectangle past the box, so Zoom Extents frames the
    // ground beside it where the ground chain is drawn.
    t.drawRectangle([2, 0, 0], [2.2, 0.2, 0])
  })
  await page.getByRole('button', { name: 'Camera' }).click()
  await page.getByText('Standard Views', { exact: true }).hover()
  await page.getByText('Iso', { exact: true }).first().click()
  await page.getByRole('button', { name: 'Camera' }).click()
  await page.getByText('Zoom Extents', { exact: true }).click()
  await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))))
  const canvas = (await page.locator('canvas').first().boundingBox())!
  const toPage = async (w: V3) => {
    const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w as V3), w)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  /** Hover onto `w` in steps (so the snap settles), then click. */
  const clickAt = async (w: V3) => {
    const s = await toPage(w)
    await page.mouse.move(s.x, s.y, { steps: 4 })
    await page.waitForTimeout(80)
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(80)
  }
  const wander = async (w: V3) => {
    const s = await toPage(w)
    await page.mouse.move(s.x, s.y, { steps: 4 })
    await page.waitForTimeout(80)
  }
  return { toPage, clickAt, wander }
}

const midChainHint = (page: Page) => page.getByText(/Click the next point|resize the segment you just drew/)
const idleHint = (page: Page) => page.getByText('Click to start a line', { exact: false })
const sameSpotToast = (page: Page) => page.getByText(/landed on the last point|same as the last one/)
/** The Outliner's "editing" chip: present once a double-click entered the box. */
const editing = (page: Page) => page.getByLabel('Tray', { exact: true }).getByText('editing', { exact: true })
/** Every segment drawn in any sketch. */
const segmentCount = (page: Page) => page.evaluate(() => {
  const t = window.__hew_test!
  return t.getSketchIds().reduce((n, id) => n + t.getSketchSegmentsExact(id).length, 0)
})

test('a slow click back on the last point finishes a ground chain and a face chain, with no toast', async ({ page }) => {
  test.setTimeout(60_000)
  const { clickAt, wander } = await setup(page)
  await page.getByRole('radio', { name: 'Line' }).click()

  // Ground: two points beside the box, then back onto the second.
  const segmentsBefore = await segmentCount(page)
  await clickAt([1.4, 0.1, 0])
  await clickAt([1.4, 0.9, 0])
  await expect(midChainHint(page)).toBeVisible()
  // The segment really landed on the ground.
  expect(await segmentCount(page)).toBe(segmentsBefore + 1)
  await wander([1.7, 0.5, 0])
  await page.waitForTimeout(600)
  await clickAt([1.4, 0.9, 0])
  await expect(idleHint(page)).toBeVisible()
  await expect(sameSpotToast(page)).toHaveCount(0)
  expect(await segmentCount(page)).toBe(segmentsBefore + 1)

  // Face: two points on the box's top, then back onto the second.
  await clickAt([0.2, 0.2, 0.5])
  await clickAt([0.7, 0.3, 0.5])
  await expect(midChainHint(page)).toBeVisible()
  await wander([0.5, 0.7, 0.5])
  await page.waitForTimeout(600)
  await clickAt([0.7, 0.3, 0.5])
  await expect(idleHint(page)).toBeVisible()
  await expect(sameSpotToast(page)).toHaveCount(0)
  await expect(editing(page)).toHaveCount(0)
})

test('a long-gap double-click on the last point finishes the chain and does not enter the object', async ({ page }) => {
  test.setTimeout(60_000)
  const { toPage, clickAt } = await setup(page)

  // Control: a double-click on the box that no tool claims (Select's) DOES
  // enter it — so the check below can see a fall-through.
  await page.getByRole('radio', { name: 'Select' }).click()
  const c = await toPage([0.5, 0.5, 0.5])
  await page.mouse.move(c.x, c.y, { steps: 4 })
  await page.mouse.dblclick(c.x, c.y)
  await expect(editing(page)).toHaveCount(1)
  await page.keyboard.press('Escape')
  await expect(editing(page)).toHaveCount(0)

  await page.getByRole('radio', { name: 'Line' }).click()

  await clickAt([0.2, 0.2, 0.5])
  const last = await toPage([0.7, 0.3, 0.5])
  await page.mouse.move(last.x, last.y, { steps: 4 })
  await page.waitForTimeout(80)
  // First press of the double-click places the point…
  await page.mouse.down({ clickCount: 1 })
  await page.mouse.up({ clickCount: 1 })
  // …and the second lands 450 ms later: past the Viewport's 400 ms click
  // window, inside the browser's own, so it reaches the tool and a
  // `dblclick` follows it.
  await page.waitForTimeout(450)
  await page.mouse.down({ clickCount: 2 })
  await page.mouse.up({ clickCount: 2 })
  await page.waitForTimeout(150)
  await expect(idleHint(page)).toBeVisible()
  await expect(sameSpotToast(page)).toHaveCount(0)
  await expect(editing(page)).toHaveCount(0)
})
