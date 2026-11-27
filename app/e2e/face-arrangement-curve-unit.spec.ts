/**
 * Stage 4b of the face arrangement, driven with REAL pointer and key events
 * on the front (y = 0) wall of a box:
 *
 * (a) A circle drawn across a rectangle's side is cut into two runs, yet
 *     stays one drawn curve: a click on it selects the run under the
 *     cursor, a double-click selects the whole circle (one "Circle" in the
 *     outliner), and Delete gives the rectangle back in one undo step.
 * (b) The Line tool's chain from the wall's left edge across the rectangle
 *     to the wall's right edge is planned in the kernel: it splits the wall
 *     and the rectangle, one undo step.
 * (c) The Arc tool's open arc from the wall's left edge over the
 *     rectangle's sides to the wall's right edge goes through the same
 *     planner: it splits every face it crosses, keeps its circle (one
 *     "Arc" curve unit of three runs), one undo step.
 */
import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

// The box spans x 0 → 22 mm; the rectangle x 6.3 → 15.7 mm, z 2.2 → 6.0 mm.
const W = 0.022
const X0 = 0.0063
const X1 = 0.0157
const Z0 = 0.0022
const Z1 = 0.006
const ZM = 0.0041
// The crossing circle: centered on the rectangle's right side, 1 mm.
const RX = X1 + 0.001

async function setup(page: Page): Promise<string> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const object = await page.evaluate(
    ({ X0, X1, RX, Z0, Z1, ZM, W }) => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [W, 0.0275, 0], 0.007)
      h.setCamera({ position: [W / 2, -0.025, ZM], target: [W / 2, 0, ZM], up: [0, 0, 1], fovDeg: 45 })
      for (const x of [X0, X1, RX]) h.addGuideLine(x, 0, ZM, 0, 0, 1)
      for (const z of [Z0, Z1, ZM]) h.addGuideLine(W / 2, 0, z, 1, 0, 0)
      return id
    },
    { X0, X1, RX, Z0, Z1, ZM, W },
  )
  await page.waitForTimeout(200)
  return object
}

async function pagePoint(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

/** Hover in from a few pixels off (so the snap engine sees motion). */
async function hover(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x - 4, p.y + 3)
  await page.waitForTimeout(80)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(150)
  return p
}

async function click(page: Page, w: V3): Promise<void> {
  await hover(page, w)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(250)
}

async function imprints(page: Page, object: string) {
  return page.evaluate((o) => window.__hew_test!.getImprints(o), object)
}

async function faceAt(page: Page, x: number, z: number): Promise<string | null> {
  return page.evaluate(
    ({ x, z }) => {
      const hit = window.__hew_test!.pickFace([x, -0.01, z], [0, 1, 0])
      return hit === null ? null : `${hit.object}:${hit.face}`
    },
    { x, z },
  )
}

async function expectDistinctFaces(page: Page, points: [number, number][]): Promise<void> {
  const faces = await Promise.all(points.map(([x, z]) => faceAt(page, x, z)))
  for (const f of faces) expect(f).not.toBeNull()
  expect(new Set(faces).size).toBe(points.length)
}

async function expectPlainRectangle(page: Page, object: string): Promise<void> {
  const list = await imprints(page, object)
  expect(list.length).toBe(1)
  expect(list[0].kind).toBe('sub_face')
  expect(list[0].name).toBe('Rectangle')
}

async function selection(page: Page) {
  return page.evaluate(() => window.__hew_test!.getSelection())
}

async function drawRectangle(page: Page, object: string): Promise<void> {
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await click(page, [X0, 0, Z0])
  await click(page, [X1, 0, Z1])
  await expectPlainRectangle(page, object)
}

test('a circle cut apart by a rectangle selects whole on a double-click and deletes back to the rectangle', async ({
  page,
}, testInfo) => {
  const object = await setup(page)
  await drawRectangle(page, object)
  await page.getByRole('radio', { name: 'Circle' }).click()
  await click(page, [X1, 0, ZM])
  await click(page, [RX, 0, ZM])
  const cutApart = async () => {
    const units = await page.evaluate((o) => window.__hew_test!.getCurveRuns(o), object)
    expect(units.length).toBe(1)
    expect(units[0].name).toBe('Circle')
    expect(units[0].runs.length).toBe(2)
    // The outliner lists the circle once, not its two runs.
    // The outliner lists the drawing once, as one connected shape — not a row
    // per run.
    const labels = await page.evaluate((o) => window.__hew_test!.getOutlinerShapeLabels(o), object)
    expect(labels.length).toBe(1)
  }
  await cutApart()
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('1-circle-across-rectangle.png') })

  // One click on the circle's rim outside the rectangle: just that run.
  await page.getByRole('radio', { name: 'Select' }).click()
  const rim: V3 = [RX, 0, ZM]
  await click(page, rim)
  let sel = await selection(page)
  expect(sel.length).toBe(1)
  expect(sel[0].kind).toBe('draft-curve')
  const runKey = sel[0].key

  // A double-click there: the whole circle.
  const p = await hover(page, rim)
  await page.mouse.dblclick(p.x, p.y)
  await page.waitForTimeout(300)
  sel = await selection(page)
  expect(sel.length).toBe(1)
  expect(sel[0].kind).toBe('draft-curve')
  expect(sel[0].key).not.toBe(runKey) // widened: the whole circle's curve key, not the run's
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('2-whole-circle-selected.png') })

  // Delete: the rectangle is back, one undo step.
  await page.keyboard.press('Delete')
  await page.waitForTimeout(300)
  await expectPlainRectangle(page, object)
  expect(await page.evaluate((o) => window.__hew_test!.getCurveRuns(o), object)).toEqual([])
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('3-rectangle-back.png') })

  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(300)
  await cutApart()
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(300)
  await expectPlainRectangle(page, object)
})

test('a Line tool chain from wall edge to wall edge across a rectangle splits both, one undo step', async ({
  page,
}, testInfo) => {
  const object = await setup(page)
  await drawRectangle(page, object)
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('4-rectangle.png') })

  await page.getByRole('radio', { name: 'Line' }).click()
  await click(page, [0, 0, ZM]) // the wall's left edge
  await click(page, [W, 0, ZM]) // the wall's right edge
  await page.keyboard.press('Escape') // ends the chain: the kernel plans and commits it
  await page.waitForTimeout(300)
  const split = async () => {
    await expect(page.getByText(/would run off the face|must start and end on the face/)).toHaveCount(0)
    const d = 0.0005
    await expectDistinctFaces(page, [
      [X0 + 0.001, ZM + d], // the rectangle, above the line
      [X0 + 0.001, ZM - d], // the rectangle, below it
      [0.002, ZM + d], // the wall, above
      [0.002, ZM - d], // the wall, below
    ])
    expect(await faceAt(page, 0.002, ZM + d)).toBe(await faceAt(page, W - 0.002, Z1 + 0.0005))
  }
  await split()
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('5-line-across.png') })

  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(300)
  await expectPlainRectangle(page, object)
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(300)
  await split()
})

test('an Arc tool arc from wall edge to wall edge over a rectangle splits both and stays one arc, one undo step', async ({
  page,
}, testInfo) => {
  const object = await setup(page)
  // Guides through the arc's two ends on the wall's left and right edges
  // (1 mm up) and its apex. (The bottom edge also lies on the ground, where
  // a click starts a ground arc instead.)
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.addGuideLine(0.011, 0, 0.001, 1, 0, 0)
    h.addGuideLine(0.011, 0, 0.004, 0, 0, 1)
    h.addGuideLine(0.011, 0, 0.004, 1, 0, 0)
  })
  await page.waitForTimeout(150)
  await drawRectangle(page, object)

  await page.getByRole('radio', { name: 'Arc' }).click()
  await click(page, [0, 0, 0.001]) // start, on the wall's left edge
  await click(page, [W, 0, 0.001]) // end, on its right edge
  await click(page, [0.011, 0, 0.004]) // the bulge, up over the rectangle's sides
  await page.waitForTimeout(300)
  const split = async () => {
    await expect(page.getByText(/doesn't split the face|must start and end on the face/)).toHaveCount(0)
    const units = await page.evaluate((o) => window.__hew_test!.getCurveRuns(o), object)
    expect(units.length).toBe(1)
    expect(units[0].name).toBe('Arc')
    expect(units[0].runs.length).toBe(3)
    await expectDistinctFaces(page, [
      [0.011, 0.003], // the rectangle, under the arc
      [0.011, 0.0055], // the rectangle, over it
      [0.004, 0.0005], // the wall, under the arc
      [0.004, 0.0055], // the wall, over it
    ])
  }
  await split()
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('6-arc-across.png') })

  await page.keyboard.press('Escape')
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(300)
  await expectPlainRectangle(page, object)
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(300)
  await split()
})
