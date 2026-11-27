/**
 * A shape drawn on a face that CROSSES another shape there splits both
 * sides of the crossing the way the same shapes split a ground sketch (the
 * face-arrangement planner's hole-anchored pieces): the part inside the
 * rectangle cuts the rectangle, the part outside cuts the wall from the
 * rectangle's edge back to it, one undo entry. Driven with REAL pointer and
 * key events on the front (y = 0) wall of a box with the real Rectangle and
 * Circle tools. Before stage 3 both draws were refused LoopNotStrictlyInside.
 *
 * Two cases: a circle centered on the rectangle's right side, and the
 * maintainer's case — a rectangle typed 3.7897 mm tall instead of 3.8 mm,
 * then the R = 1.9 mm circle at its left end, which overshoots the top by
 * 10.3 µm and leaves a sliver there, as on the ground.
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
// The maintainer's short rectangle top: 2.2 + 3.7897 mm.
const Z1_SHORT = 0.0059897
// The inscribed circle: center 1.9 mm in from the left and bottom sides.
const XC = 0.0082
const ZC = 0.0041
const R = 0.0019
// The crossing circle: centered on the right side at mid height, 1 mm.
const ZM = 0.0041
const RX = X1 + 0.001

async function setup(page: Page): Promise<string> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const object = await page.evaluate(
    ({ X0, X1, XC, RX, Z0, Z1, ZC }) => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [0.022, 0.0275, 0], 0.007)
      // Looking straight at the y = 0 wall from the front, close in.
      h.setCamera({ position: [0.011, -0.025, ZC], target: [0.011, 0, ZC], up: [0, 0, 1], fovDeg: 45 })
      // Guides whose intersections are every point the gestures click.
      for (const x of [X0, X1, XC, RX]) h.addGuideLine(x, 0, ZC, 0, 0, 1)
      for (const z of [Z0, Z1, ZC]) h.addGuideLine(0.011, 0, z, 1, 0, 0)
      return id
    },
    { X0, X1, XC, RX, Z0, Z1, ZC },
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

/** `points` hit distinct faces of the wall, none missing. */
async function expectDistinctFaces(page: Page, points: [number, number][]): Promise<void> {
  const faces = await Promise.all(points.map(([x, z]) => faceAt(page, x, z)))
  for (const f of faces) expect(f).not.toBeNull()
  expect(new Set(faces).size).toBe(points.length)
}

async function expectNoRefusal(page: Page): Promise<void> {
  await expect(page.getByText(/would run off the face/)).toHaveCount(0)
}

async function expectPlainRectangle(page: Page, object: string): Promise<void> {
  const list = await imprints(page, object)
  expect(list.length).toBe(1)
  expect(list[0].kind).toBe('sub_face')
  expect(list[0].name).toBe('Rectangle')
}

async function undoRedo(page: Page, object: string, arranged: () => Promise<void>): Promise<void> {
  // One undo entry: Ctrl+Z restores the plain rectangle, redo the arrangement.
  await page.keyboard.press('Escape')
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  await expectPlainRectangle(page, object)
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(250)
  await arranged()
}

test('a circle centered on a rectangle side cuts both the rectangle and the wall', async ({ page }, testInfo) => {
  const object = await setup(page)
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await click(page, [X0, 0, Z0])
  await click(page, [X1, 0, Z1])
  await expectPlainRectangle(page, object)
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('1-rectangle.png') })

  await page.getByRole('radio', { name: 'Circle' }).click()
  await click(page, [X1, 0, ZM]) // on the rectangle's right side
  await click(page, [RX, 0, ZM]) // 1 mm to the right, on the wall
  const arranged = async () => {
    await expectNoRefusal(page)
    // No whole imprint is left: every line between two regions is a chord.
    const list = await imprints(page, object)
    expect(list.length).toBeGreaterThan(0)
    expect(list.every((f) => f.kind === 'chord')).toBe(true)
    // The rectangle's leftover, the half-disc inside it, the half-disc
    // outside it, and the wall: four distinct faces.
    const d = 0.0003
    await expectDistinctFaces(page, [
      [X1 - d, ZM],
      [X1 + d, ZM],
      [X0 + 0.001, ZM],
      [0.002, 0.001],
    ])
  }
  await arranged()
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('2-crossing.png') })
  await undoRedo(page, object, arranged)
})

test('an R1.9 circle at the end of a rectangle typed 3.7897 mm tall leaves a sliver, as on the ground', async ({
  page,
}, testInfo) => {
  const object = await setup(page)
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await click(page, [X0, 0, Z0])
  await page.keyboard.type('9.4mm,3.7897mm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(250)
  await expectPlainRectangle(page, object)
  const zs = (await imprints(page, object))[0].loop.map((p) => p[2])
  expect(Math.max(...zs)).toBeCloseTo(Z1_SHORT, 9)
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('3-short-rectangle.png') })

  await page.getByRole('radio', { name: 'Circle' }).click()
  await click(page, [XC, 0, ZC])
  // On the left side, 1.9 mm up — where the side's midpoint snap (5.15 µm
  // lower on this short rectangle) wins, as it does under a user's click:
  // the radius is 1.900007 mm, still tangent at the bottom and 10.3 µm over
  // the top.
  await click(page, [X0, 0, ZC])
  const arranged = async () => {
    await expectNoRefusal(page)
    const list = await imprints(page, object)
    expect(list.every((f) => f.kind === 'chord')).toBe(true)
    // Arc runs keep the drawn circle.
    for (const f of list) {
      for (const c of f.curves) {
        if (c === null) continue
        expect(c.radius).toBeCloseTo(R, 7)
        expect(c.center[0]).toBeCloseTo(XC, 9)
        expect(c.center[2]).toBeCloseTo(ZC, 9)
      }
    }
    // The disc, the 10.3 µm sliver above the rectangle's top, the three
    // leftovers, and the wall: six distinct faces.
    const d = 0.0002
    await expectDistinctFaces(page, [
      [XC, ZC],
      [XC, Z1_SHORT + 0.000005],
      [X0 + d, Z1_SHORT - d],
      [X0 + d, Z0 + d],
      [X1 - 0.001, ZC],
      [0.002, 0.001],
    ])
  }
  await arranged()
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('4-overshoot.png') })
  await undoRedo(page, object, arranged)
})
