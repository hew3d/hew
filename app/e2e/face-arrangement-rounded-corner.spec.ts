/**
 * Rounding a rectangle's corners by deleting lines on a face, as on the
 * ground: on the front (y = 0) wall of a box, a 9.4 × 3.8 mm rectangle and a
 * circle of radius 1.9 mm inscribed at its left end (tangent to the left,
 * top, and bottom sides) leave the disc plus three leftovers. Clicking one of
 * a corner leftover's rectangle edges with the Select tool selects the run
 * of rectangle edges between that leftover and the wall, and Delete merges
 * the leftover into the wall (one draft erasing those edges), so
 * that corner of the rectangle becomes the circle's arc. Both left corners,
 * then the arc between the disc and the right leftover, give one imprint with
 * two rounded corners. Driven with REAL pointer and key events; Ctrl+Z and
 * Ctrl+Shift+Z walk the three deletes back and forth.
 */
import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

// The rectangle (front wall, y = 0): x 6.3 → 15.7 mm, z 2.2 → 6.0 mm.
const X0 = 0.0063
const X1 = 0.0157
const Z0 = 0.0022
const Z1 = 0.006
// The circle: center 1.9 mm in from the left side, at mid height.
const XC = 0.0082
const ZC = 0.0041
const R = 0.0019

async function setup(page: Page): Promise<string> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const object = await page.evaluate(
    ({ X0, X1, XC, Z0, Z1, ZC }) => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [0.022, 0.0275, 0], 0.007)
      // Looking straight at the left end of the rectangle, close in.
      h.setCamera({ position: [0.0095, -0.016, ZC], target: [0.0095, 0, ZC], up: [0, 0, 1], fovDeg: 45 })
      // Guides whose intersections are every point the drawing clicks. They
      // stay: the guides at X0, Z0, and Z1 lie exactly on the rectangle's
      // left, bottom, and top sides, so the corner clicks below land on a
      // guide over the edge, as they do in a real session.
      for (const x of [X0, X1, XC]) h.addGuideLine(x, 0, ZC, 0, 0, 1)
      for (const z of [Z0, Z1, ZC]) h.addGuideLine(0.011, 0, z, 1, 0, 0)
      return id
    },
    { X0, X1, XC, Z0, Z1, ZC },
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
async function faceAt(page: Page, x: number, z: number): Promise<string | null> {
  return page.evaluate(
    ({ x, z }) => {
      const hit = window.__hew_test!.pickFace([x, -0.01, z], [0, 1, 0])
      return hit === null ? null : `${hit.object}:${hit.face}`
    },
    { x, z },
  )
}

const D = 0.0002
// A point inside each region: wall, top-left and bottom-left corner
// leftovers, disc, right leftover.
const WALL: [number, number] = [0.002, 0.001]
const TL: [number, number] = [X0 + D, Z1 - D]
const BL: [number, number] = [X0 + D, Z0 + D]
const DISC: [number, number] = [XC, ZC]
const RIGHT: [number, number] = [X1 - 0.001, ZC]

async function faces(page: Page): Promise<(string | null)[]> {
  return Promise.all([WALL, TL, BL, DISC, RIGHT].map(([x, z]) => faceAt(page, x, z)))
}

/** The circle's facets keep its claim (per-edge curves of every arc run). */
function expectCircle(c: { center: V3; radius: number } | null): void {
  expect(c).not.toBeNull()
  expect(c!.radius).toBeCloseTo(R, 9)
  expect(c!.center[0]).toBeCloseTo(XC, 9)
  expect(c!.center[2]).toBeCloseTo(ZC, 9)
}

/** Disc + three leftovers: five faces, six chord runs (three arcs, and the
 *  three leftovers' runs along the rectangle). */
async function expectArranged(page: Page, object: string): Promise<void> {
  const list = await imprints(page, object)
  expect(list.map((f) => f.kind)).toEqual(Array(6).fill('chord'))
  expect(list.filter((f) => f.curves.every((c) => c === null)).length).toBe(3)
  const f = await faces(page)
  for (const x of f) expect(x).not.toBeNull()
  expect(new Set(f).size).toBe(5)
}

/** Top-left rounded: its leftover is now the wall. */
async function expectTopLeftRounded(page: Page): Promise<void> {
  const [wall, tl, bl, disc, right] = await faces(page)
  expect(tl).toBe(wall)
  expect(new Set([wall, bl, disc, right]).size).toBe(4)
}

async function expectBothRounded(page: Page): Promise<void> {
  const [wall, tl, bl, disc, right] = await faces(page)
  expect(tl).toBe(wall)
  expect(bl).toBe(wall)
  expect(new Set([wall, disc, right]).size).toBe(3)
}

/** One imprint: the rectangle with two rounded corners. */
async function expectRoundedShape(page: Page, object: string): Promise<void> {
  const list = await imprints(page, object)
  expect(list.length).toBe(1)
  expect(list[0].kind).toBe('sub_face')
  expect(list[0].name).toBe('Arc shape')
  // Twelve facets of the left half-circle, then three straight sides.
  expect(list[0].loop.length).toBe(15)
  const arcs = list[0].curves.filter((c) => c !== null)
  expect(arcs.length).toBe(12)
  for (const c of arcs) expectCircle(c)
  const [wall, tl, bl, disc, right] = await faces(page)
  expect(tl).toBe(wall)
  expect(bl).toBe(wall)
  expect(disc).toBe(right)
  expect(disc).not.toBe(wall)
}

/** Select tool: click `at`, then (`alsoAt`) Shift-click the corner's other
 *  side. Expect exactly those selected — plain rectangle LINES (a corner is
 *  two of them: tangent point, corner, tangent point), or (`arc`) one RUN of
 *  circle facets — then Delete. */
async function selectAndDelete(page: Page, at: V3, arc: boolean, alsoAt?: V3): Promise<void> {
  await page.getByRole('radio', { name: 'Select' }).click()
  await click(page, at)
  if (alsoAt !== undefined) {
    await page.keyboard.down('Shift')
    await click(page, alsoAt)
    await page.keyboard.up('Shift')
  }
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel.length).toBe(alsoAt === undefined ? 1 : 2)
  const units = await page.evaluate(() => {
    const h = window.__hew_test!
    // The front wall: y = 0, outward normal -Y.
    return h.getPlaneUnits(h.ctxOfPlane([0, 0, 0], [0, -1, 0])!).units
  })
  for (const n of sel) {
    if (arc) {
      expect(n.kind).toBe('draft-curve')
      const run = units.flatMap((u) => u.curves).find((r) => r.key === n.key)
      expect(run).toBeDefined()
      expectCircle({ center: run!.center as V3, radius: run!.radius })
    } else {
      expect(n.kind).toBe('draft-edge')
      const e = units.flatMap((u) => u.edges).find((x) => x.key === n.key)
      expect(e).toBeDefined()
      expect(e!.curve).toBeNull()
    }
  }
  await page.keyboard.press('Delete')
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
}

test('deleting the corner leftovers’ rectangle edges rounds the corners, and undo/redo walk it exactly', async ({
  page,
}, testInfo) => {
  const object = await setup(page)
  const shot = (name: string) => page.locator('canvas').first().screenshot({ path: testInfo.outputPath(name) })

  // The rectangle, then the inscribed circle (rim on the left-side midpoint).
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await click(page, [X0, 0, Z0])
  await click(page, [X1, 0, Z1])
  await page.getByRole('radio', { name: 'Circle' }).click()
  await click(page, [XC, 0, ZC])
  await click(page, [X0, 0, ZC])
  await page.keyboard.press('Escape')
  await expectArranged(page, object)
  await shot('1-arranged.png')

  // The top-left leftover's top edge, half a millimetre in from the corner:
  // its run along the rectangle (top and left segments) selects as one.
  await selectAndDelete(page, [X0 + 0.0005, 0, Z1], false, [X0, 0, Z1 - 0.0005])
  await expectTopLeftRounded(page)
  await shot('2-top-left-rounded.png')

  // The bottom-left leftover's bottom edge.
  await selectAndDelete(page, [X0 + 0.0005, 0, Z0], false, [X0, 0, Z0 + 0.0005])
  await expectBothRounded(page)
  await shot('3-both-rounded.png')

  // The arc between the disc and the right leftover, at its rightmost point.
  await selectAndDelete(page, [XC + R, 0, ZC], true)
  await expectRoundedShape(page, object)
  await shot('4-rounded-shape.png')

  // Undo walks the three deletes back; redo walks them forward again.
  await page.keyboard.press('Escape')
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  await expectBothRounded(page)
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  await expectTopLeftRounded(page)
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  await expectArranged(page, object)
  await shot('5-undone.png')
  for (let i = 0; i < 3; i++) {
    await page.keyboard.press('ControlOrMeta+Shift+Z')
    await page.waitForTimeout(250)
  }
  await expectRoundedShape(page, object)
  await shot('6-redone.png')
})
