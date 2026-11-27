import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Union, Subtract and Intersect over more than two solids, end to end with
 * REAL pointer input: the solids are picked with clicks and Shift-clicks in
 * the viewport and the command comes from the Object menu. Ground truth is
 * the harness's object list and bounds (logical, not pixel).
 *
 * Every box below sits on the ground with its front face at y = 0 or lower,
 * facing the camera, so each one is clickable on its own front face.
 */

const CAMERA = {
  position: [3, -12, 7] as [number, number, number],
  target: [3, 0.5, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 50,
}

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

/** Click the first point, then Shift-click each of the rest. */
async function pickInOrder(page: Page, points: [number, number, number][]): Promise<void> {
  const [first, ...rest] = points
  const p0 = await pagePoint(page, first)
  await page.mouse.click(p0.x, p0.y)
  for (const w of rest) {
    const p = await pagePoint(page, w)
    await page.keyboard.down('Shift')
    await page.mouse.click(p.x, p.y)
    await page.keyboard.up('Shift')
  }
}

async function objectMenu(page: Page, item: 'Union' | 'Subtract' | 'Intersect'): Promise<void> {
  const bar = page.getByTestId('menu-bar')
  await bar.getByRole('button', { name: 'Object', exact: true }).click()
  await bar.getByText(item, { exact: true }).click()
}

const objectIds = (page: Page) => page.evaluate(() => window.__hew_test!.getObjectIds())
const selection = (page: Page) =>
  page.evaluate(() => window.__hew_test!.getSelection().map((s) => ({ kind: s.kind, id: s.id })))

test('three overlapping solids union into one object, as one undo step', async ({ page }) => {
  await setup(page)
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    return [
      h.drawBox([0, 0, 0], [2, 1, 0], 1),
      h.drawBox([1.5, -0.5, 0], [3.5, 1, 0], 1),
      h.drawBox([3, -1, 0], [5, 1, 0], 1),
    ]
  })
  await page.waitForTimeout(100)

  await pickInOrder(page, [
    [0.5, 0, 0.5],
    [2.5, -0.5, 0.5],
    [4.5, -1, 0.5],
  ])
  expect((await selection(page)).map((s) => s.id)).toEqual(ids)

  await objectMenu(page, 'Union')
  await expect.poll(() => objectIds(page)).toHaveLength(1)
  const [result] = await objectIds(page)
  expect(await selection(page)).toEqual([{ kind: 'object', id: result }])
  const b = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), result)
  expect(b[0]).toBeCloseTo(0, 6)
  expect(b[1]).toBeCloseTo(-1, 6)
  expect(b[3]).toBeCloseTo(5, 6)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  // One undo brings all three back; one redo combines them again.
  await page.evaluate(() => window.__hew_test!.undo())
  await expect.poll(() => objectIds(page)).toHaveLength(3)
  await page.evaluate(() => window.__hew_test!.redo())
  await expect.poll(() => objectIds(page)).toHaveLength(1)
})

test('Subtract keeps the first solid picked and cuts every other one out of it', async ({ page }) => {
  await setup(page)
  const { bar } = await page.evaluate(() => {
    const h = window.__hew_test!
    const bar = h.drawBox([0, 0, 0], [6, 1, 0], 1)
    // Two cutters that pass through the bar, front faces in front of it.
    h.drawBox([1, -1, 0], [1.5, 2, 0], 2)
    h.drawBox([4, -1, 0], [4.5, 2, 0], 2)
    return { bar }
  })
  await page.waitForTimeout(100)

  // The bar first (between the cutters), then the two cutters.
  await pickInOrder(page, [
    [3, 0, 0.5],
    [1.25, -1, 1.5],
    [4.25, -1, 1.5],
  ])
  expect((await selection(page))[0]).toEqual({ kind: 'object', id: bar })

  await objectMenu(page, 'Subtract')
  // The bar is cut into three pieces, which arrive together in one group.
  await expect.poll(() => objectIds(page)).toHaveLength(3)
  const sel = await selection(page)
  expect(sel).toHaveLength(1)
  expect(sel[0].kind).toBe('group')
  for (const id of await objectIds(page)) {
    const b = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
    // Every piece is part of the bar: nothing of the cutters is left.
    expect(b[1]).toBeCloseTo(0, 6)
    expect(b[5]).toBeCloseTo(1, 6)
  }
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  await page.evaluate(() => window.__hew_test!.undo())
  await expect.poll(() => objectIds(page)).toHaveLength(3)
  expect(await objectIds(page)).toContain(bar)
})

test('a Subtract whose cutters all miss is refused and changes nothing', async ({ page }) => {
  await setup(page)
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    return [
      h.drawBox([0, 0, 0], [2, 1, 0], 1),
      h.drawBox([3, 0, 0], [4, 1, 0], 1),
      h.drawBox([5, 0, 0], [6, 1, 0], 1),
    ]
  })
  await page.waitForTimeout(100)

  await pickInOrder(page, [
    [1, 0, 0.5],
    [3.5, 0, 0.5],
    [5.5, 0, 0.5],
  ])
  await objectMenu(page, 'Subtract')
  await expect(page.getByText(/Nothing was cut away/).first()).toBeVisible()
  expect((await objectIds(page)).sort()).toEqual([...ids].sort())
})
