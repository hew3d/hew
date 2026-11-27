/**
 * A shape drawn on a face that joins two SEPARATE outlines there — a circle
 * overlapping two rectangles drawn on the wall — splits everything it
 * crosses the way the same shapes split a ground sketch: each rectangle
 * loses a lens, the part of the circle between them becomes its own face,
 * and the two rectangles' outlines join into one. The two arcs between the
 * rectangles each join the two rectangles' holes in the wall (bridges), so
 * the wall is re-traced with both at once; until stage 4 the draw was
 * refused. Then the middle piece's top arc is selected and deleted: the
 * piece meets the wall along two separate runs (its top and bottom arcs),
 * so the merge dissolves both and the joined outline splits back into two.
 * Ctrl+Z and Ctrl+Shift+Z walk it exactly.
 *
 * Driven with REAL pointer and key events on the front (y = 0) wall of a
 * box, with the real Rectangle, Circle, and Select tools.
 */
import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

// Rectangle A: x 3 → 8 mm, B: x 12 → 17 mm, both z 2 → 5.5 mm.
const AX0 = 0.003
const AX1 = 0.008
const BX0 = 0.012
const BX1 = 0.017
const Z0 = 0.002
const Z1 = 0.0055
// The circle: centered in the gap at mid height, rim 2.5 mm straight up —
// it crosses A's right side and B's left side.
const XC = 0.01
const ZC = 0.00375
const R = 0.0025

// Sample points (x, z): the wall, A's rest, A's lens, the middle piece, B's
// lens, B's rest.
const WALL: [number, number] = [0.0015, 0.001]
const A_REST: [number, number] = [0.004, ZC]
const A_LENS: [number, number] = [0.0078, ZC]
const MIDDLE: [number, number] = [XC, ZC]
const B_LENS: [number, number] = [0.0122, ZC]
const B_REST: [number, number] = [0.016, ZC]

async function setup(page: Page): Promise<string> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const object = await page.evaluate(
    ({ AX0, AX1, BX0, BX1, XC, Z0, Z1, ZC, R }) => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [0.022, 0.0275, 0], 0.007)
      // Looking straight at the y = 0 wall from the front, close in.
      h.setCamera({ position: [0.011, -0.025, ZC], target: [0.011, 0, ZC], up: [0, 0, 1], fovDeg: 45 })
      // Guides whose intersections are every point the gestures click.
      for (const x of [AX0, AX1, BX0, BX1, XC]) h.addGuideLine(x, 0, ZC, 0, 0, 1)
      for (const z of [Z0, Z1, ZC, ZC + R]) h.addGuideLine(0.011, 0, z, 1, 0, 0)
      return id
    },
    { AX0, AX1, BX0, BX1, XC, Z0, Z1, ZC, R },
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

/** Hover in from a few pixels off (so the snap engine sees motion), then click. */
async function click(page: Page, w: V3): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x - 4, p.y + 3)
  await page.waitForTimeout(80)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
}

async function imprints(page: Page, object: string) {
  return page.evaluate((o) => window.__hew_test!.getImprints(o), object)
}

/** The face a ray straight into the wall at (x, z) hits, as `object:face`. */
async function faceAt(page: Page, [x, z]: [number, number]): Promise<string | null> {
  return page.evaluate(
    ({ x, z }) => {
      const hit = window.__hew_test!.pickFace([x, -0.01, z], [0, 1, 0])
      return hit === null ? null : `${hit.object}:${hit.face}`
    },
    { x, z },
  )
}

async function faces(page: Page): Promise<(string | null)[]> {
  return Promise.all([WALL, A_REST, A_LENS, MIDDLE, B_LENS, B_REST].map((p) => faceAt(page, p)))
}

async function expectNoRefusal(page: Page): Promise<void> {
  await expect(page.getByText(/would run off the face/)).toHaveCount(0)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
}

/** Two plain rectangle imprints. */
async function expectTwoRectangles(page: Page, object: string): Promise<void> {
  const list = await imprints(page, object)
  expect(list.map((f) => f.kind)).toEqual(['sub_face', 'sub_face'])
  expect(list.map((f) => f.name)).toEqual(['Rectangle', 'Rectangle'])
}

/** The circle across both: six distinct faces, every line a chord run. */
async function expectBridged(page: Page, object: string): Promise<void> {
  const list = await imprints(page, object)
  expect(list.length).toBeGreaterThan(0)
  expect(list.every((f) => f.kind === 'chord')).toBe(true)
  const f = await faces(page)
  for (const x of f) expect(x).not.toBeNull()
  expect(new Set(f).size).toBe(6)
}

/** The middle piece merged into the wall: five faces, the middle one the wall. */
async function expectMiddleMerged(page: Page): Promise<void> {
  const [wall, aRest, aLens, middle, bLens, bRest] = await faces(page)
  expect(middle).toBe(wall)
  expect(new Set([wall, aRest, aLens, bLens, bRest]).size).toBe(5)
}

test('a circle over two rectangles on a wall splits them all, and deleting its middle arc splits the outline again', async ({
  page,
}, testInfo) => {
  const object = await setup(page)
  const shot = (name: string) => page.locator('canvas').first().screenshot({ path: testInfo.outputPath(name) })

  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await click(page, [AX0, 0, Z0])
  await click(page, [AX1, 0, Z1])
  await click(page, [BX0, 0, Z0])
  await click(page, [BX1, 0, Z1])
  await page.keyboard.press('Escape')
  await expectTwoRectangles(page, object)
  await shot('1-two-rectangles.png')

  // The circle: center in the gap, rim 2.5 mm straight up on the wall.
  await page.getByRole('radio', { name: 'Circle' }).click()
  await click(page, [XC, 0, ZC])
  await click(page, [XC, 0, ZC + R])
  await page.keyboard.press('Escape')
  await expectNoRefusal(page)
  await expectBridged(page, object)
  await shot('2-circle-over-both.png')

  // Select the middle piece's top arc by clicking the middle of one of its
  // facets, then Delete: the merge dissolves both arcs it shares with the
  // wall and the joined outline splits back into the two rectangles'.
  // (Its end facets, cut where the circle crosses the rectangles' sides,
  // no longer claim the circle; click a whole facet.)
  const eps = 1e-9
  const top = (await imprints(page, object)).find(
    (f) =>
      f.kind === 'chord' &&
      f.curves.some((c) => c !== null) &&
      f.loop.every((p) => p[0] > AX1 - eps && p[0] < BX0 + eps && p[2] > ZC),
  )
  expect(top).toBeDefined()
  const k = top!.curves.findIndex((c) => c !== null)
  const [p, q] = [top!.loop[k], top!.loop[k + 1]]
  await page.getByRole('radio', { name: 'Select' }).click()
  await click(page, [(p[0] + q[0]) / 2, 0, (p[2] + q[2]) / 2])
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel.length).toBe(1)
  expect(sel[0].kind).toBe('draft-curve')
  await page.keyboard.press('Delete')
  await page.waitForTimeout(250)
  await expectNoRefusal(page)
  await expectMiddleMerged(page)
  await shot('3-middle-merged.png')

  // Undo walks the delete and the circle back; redo walks them forward.
  await page.keyboard.press('Escape')
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  await expectBridged(page, object)
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  await expectTwoRectangles(page, object)
  await shot('4-undone.png')
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(250)
  await expectBridged(page, object)
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(250)
  await expectMiddleMerged(page)
  await shot('5-redone.png')
})
