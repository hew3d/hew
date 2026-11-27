import { test, expect, type Page } from '@playwright/test'

/**
 * OW-14: with Push/Pull armed and idle, the face a click would move is
 * tinted and outlined under the cursor (`FaceCueLayer`, fed by the tool's
 * `hoverFace()`), orange for a push or pull and blue for a new object.
 * Real mouse moves and a real Control tap throughout; the lit target is read
 * through the test harness (`getFaceCue`), and a screenshot is attached.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pointer-driven spec, chromium only')

type V3 = [number, number, number]

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

async function toPage(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const canvas = (await page.locator('canvas').first().boundingBox())!
  const p = await page.evaluate((world) => window.__hew_test!.worldToScreen(world as V3), w)
  return { x: canvas.x + p.x, y: canvas.y + p.y }
}

async function hoverAt(page: Page, p: { x: number; y: number }): Promise<void> {
  await page.mouse.move(p.x + 4, p.y + 4)
  await page.mouse.move(p.x, p.y, { steps: 3 })
  await page.waitForTimeout(120)
}

const cue = (page: Page) => page.evaluate(() => window.__hew_test!.getFaceCue())

test('hovering a box lights the face under the cursor, follows it to another face, clears off the model, and recolors on the Ctrl tap', async ({ page }, info) => {
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.drawBox([0, 0, 0], [1, 1, 0], 1)
    // A three-quarter view: the top (+Z) and the front (-Y) faces both show.
    t.setCamera({ position: [3, -4, 3], target: [0.5, 0.5, 0.5], up: [0, 0, 1], fovDeg: 45 })
  })
  await page.waitForTimeout(400)
  await page.getByRole('radio', { name: 'Push/Pull' }).click()

  expect(await cue(page)).toBeNull()

  // The top face.
  await hoverAt(page, await toPage(page, [0.5, 0.5, 1]))
  const top = await cue(page)
  expect(top).not.toBeNull()
  expect(top).toMatchObject({ kind: 'face', color: '--preview-pushpull', tinted: true, outlined: true })
  const topPick = await page.evaluate(() => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]))
  expect(top!.id).toBe(String(topPick!.face))
  await info.attach('cue-top-face', { body: await page.screenshot(), contentType: 'image/png' })

  // The front face: a different face of the same object.
  await hoverAt(page, await toPage(page, [0.5, 0, 0.5]))
  const front = await cue(page)
  expect(front).toMatchObject({ kind: 'face', owner: top!.owner, color: '--preview-pushpull' })
  expect(front!.id).not.toBe(top!.id)
  await info.attach('cue-side-face', { body: await page.screenshot(), contentType: 'image/png' })

  // Tap Control: the same face, now blue (a click would make a new object).
  await page.keyboard.press('Control')
  await page.waitForTimeout(100)
  const asNew = await cue(page)
  expect(asNew).toMatchObject({ kind: 'face', owner: front!.owner, id: front!.id, color: '--preview-extrude-new' })
  await info.attach('cue-extrude-as-new', { body: await page.screenshot(), contentType: 'image/png' })
  await page.keyboard.press('Control')
  await page.waitForTimeout(100)
  expect((await cue(page))!.color).toBe('--preview-pushpull')

  // Off the model: nothing lit.
  await hoverAt(page, await toPage(page, [-2.5, 0.5, 0]))
  expect(await cue(page)).toBeNull()

  // Back on, then off the canvas: cleared.
  await hoverAt(page, await toPage(page, [0.5, 0.5, 1]))
  expect(await cue(page)).not.toBeNull()
  await page.mouse.move(2, 2)
  await page.waitForTimeout(120)
  expect(await cue(page)).toBeNull()
})

test('pressing starts the pull and takes the cue; Escape and a tool switch leave none', async ({ page }) => {
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.drawBox([0, 0, 0], [1, 1, 0], 1)
    t.setCamera({ position: [3, -4, 3], target: [0.5, 0.5, 0.5], up: [0, 0, 1], fovDeg: 45 })
  })
  await page.waitForTimeout(400)
  await page.getByRole('radio', { name: 'Push/Pull' }).click()

  const top = await toPage(page, [0.5, 0.5, 1])
  await hoverAt(page, top)
  expect(await cue(page)).not.toBeNull()
  await page.mouse.down()
  await page.mouse.up()
  // The first click began a pull: the ghost, not the cue.
  expect(await cue(page)).toBeNull()
  // Escape cancels the pull and the still pointer does not re-light the face.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(100)
  expect(await cue(page)).toBeNull()

  // Idle again over the face: lit again; Escape while it is lit drops it.
  await hoverAt(page, await toPage(page, [0.4, 0.4, 1]))
  expect(await cue(page)).not.toBeNull()
  await page.keyboard.press('Escape')
  await page.waitForTimeout(100)
  expect(await cue(page)).toBeNull()

  // Lit once more; switching tools drops it.
  await hoverAt(page, await toPage(page, [0.45, 0.45, 1]))
  expect(await cue(page)).not.toBeNull()
  await page.getByRole('radio', { name: 'Select' }).click()
  expect(await cue(page)).toBeNull()
})

test('a face-less hover of a free sketch region lights it blue', async ({ page }) => {
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.setCamera({ position: [0.5, -3, 3], target: [0.5, 0.5, 0], up: [0, 0, 1], fovDeg: 45 })
  })
  await page.waitForTimeout(400)
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  const a = await toPage(page, [0, 0, 0])
  const b = await toPage(page, [1, 1, 0])
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.up()
  await page.mouse.move(b.x, b.y, { steps: 4 })
  await page.mouse.down()
  await page.mouse.up()
  await page.getByRole('radio', { name: 'Push/Pull' }).click()
  await hoverAt(page, await toPage(page, [0.5, 0.5, 0]))
  expect(await cue(page)).toMatchObject({ kind: 'region', color: '--preview-extrude-new', tinted: true, outlined: true })
})
