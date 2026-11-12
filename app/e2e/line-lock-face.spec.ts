import { test, expect } from '@playwright/test'

/**
 * Playtest item 7: a Line chain anchored on the midpoint of an edge shared
 * by two faces (a box's top/west edge), then locked to red. The pick at
 * that midpoint may adopt the WEST face, whose plane is normal to the
 * lock — every candidate would then collapse onto the anchor and the
 * rubber band would ride the lock line's ray-nearest point, jumping with
 * parallax. The lock must re-adopt the top face, so hovering the top
 * face's south edge projects each edge point perpendicularly onto the lock
 * (length = x), and hovering the north edge's midpoint reports Midpoint,
 * projected — with its dotted tie drawn from the real midpoint.
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

test('a red lock from a shared-edge midpoint draws along the top face and projects its edge points', async ({
  page,
}) => {
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.drawBox([0, 0, 0], [1, 1, 0], 0.5)
    // An ISO view from the south-west, looking down at the box.
    t.setCamera({ position: [-2.2, -2.6, 2.4], target: [0.5, 0.5, 0.25], up: [0, 0, 1], fovDeg: 45 })
  })
  await page.evaluate(
    () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))),
  )
  const canvas = (await page.locator('canvas').first().boundingBox())!
  const toPage = async (w: [number, number, number]) => {
    const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w as [number, number, number]), w)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  const readoutMeters = async (): Promise<number> => {
    const t = (await page.locator('text=/^Value$|^Length$/').locator('xpath=..').allTextContents())[0] ?? ''
    const m = /([\d.]+)\s*m/.exec(t)
    return m ? Number(m[1]) : NaN
  }
  const chip = async () =>
    (await page.locator('text=/^(Endpoint|Midpoint|On Edge|On Face|On Axis|Intersection)/').allTextContents()).join('|')

  await page.getByRole('radio', { name: 'Line' }).click()
  const start = await toPage([0, 0.5, 0.5])
  await page.mouse.move(start.x, start.y)
  await expect.poll(chip).toContain('Midpoint')
  await page.mouse.down()
  await page.mouse.up()
  // The chain is on the box's top face, not the ground: nothing "projected".
  const q = await toPage([0.5, 0.5, 0.5])
  await page.mouse.move(q.x, q.y)
  await expect.poll(chip).toContain('On Face')
  expect(await page.locator('text=projected').count()).toBe(0)
  await page.keyboard.press('ArrowRight')

  // Each point of the top face's south edge projects onto the lock at its own x.
  for (const x of [0.3, 0.5, 0.8]) {
    const p = await toPage([x, 0, 0.5])
    await page.mouse.move(p.x, p.y)
    await expect.poll(readoutMeters).toBeCloseTo(x, 2)
    expect(await chip()).not.toContain('On Axis')
  }

  // The north edge's midpoint: a projected Midpoint, 0.5 m along the lock.
  const nm = await toPage([0.5, 1, 0.5])
  await page.mouse.move(nm.x, nm.y)
  await expect.poll(chip).toContain('Midpoint')
  expect(await readoutMeters()).toBeCloseTo(0.5, 2)

  // Commit there: the segment lies on the top face (z = 0.5), 0.5 m long.
  await page.mouse.down()
  await page.mouse.up()
  await page.keyboard.press('Escape')
  const err = await page.evaluate(() => window.__hew_test!.getLastError())
  expect(err).toBeNull()
})

test('without a lock, the second point decides which face a shared-edge anchor meant', async ({ page }) => {
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
  const click = async (p: { x: number; y: number }) => {
    await page.mouse.move(p.x, p.y)
    await page.mouse.down()
    await page.mouse.up()
  }
  const before = await page.evaluate(() => ({
    w: window.__hew_test!.pickFace([0.25, 0.5, 5], [0, 0, -1])?.face,
    e: window.__hew_test!.pickFace([0.75, 0.5, 5], [0, 0, -1])?.face,
  }))
  expect(before.w).toEqual(before.e) // one top face to start with

  await page.getByRole('radio', { name: 'Line' }).click()
  await click(await toPage([0, 0.5, 0.5])) // west edge midpoint — shared by the west and top faces
  await click(await toPage([1, 0.5, 0.5])) // east edge midpoint: across the TOP face
  await page.keyboard.press('Escape')
  const after = await page.evaluate(() => ({
    err: window.__hew_test!.getLastError(),
    w: window.__hew_test!.pickFace([0.25, 0.5, 5], [0, 0, -1])?.face,
    e: window.__hew_test!.pickFace([0.75, 0.5, 5], [0, 0, -1])?.face,
    n: window.__hew_test!.pickFace([0.5, 0.75, 5], [0, 0, -1])?.face,
    s: window.__hew_test!.pickFace([0.5, 0.25, 5], [0, 0, -1])?.face,
  }))
  expect(after.err).toBeNull()
  // The top face was split along the line: north and south halves differ.
  expect(after.n).not.toEqual(after.s)
})
