import { test, expect, type Page } from '@playwright/test'

/**
 * Named points on the view-inferred plane and under a Shift pin
 * (docs/design/wireframe-faces-and-solids.md R18, Kurt's playtest find):
 * a 1 m Rectangle on the ground, the camera low to one side so the picker
 * infers the green (or red) plane, and a hover on a corner of the ground
 * Rectangle that is off that plane. The chip reads "Endpoint" with no
 * "projected", the plane patch passes through the corner, and the click
 * starts a vertical Rectangle AT that corner on the inferred orientation.
 * No arrow key is pressed anywhere. Four walls on the ground Rectangle
 * make the fifth face of a box: the top closes by itself and the faces
 * become one solid Object.
 *
 * Real pointer, wheel-free, key events only for typed sizes and Shift; the
 * harness pins the camera and reads kernel state back.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pixel-accurate picking is only stable on pinned SwiftShader')

type V3 = [number, number, number]

const TARGET: V3 = [0.5, 0.5, 0.3]
/** Low and to the left of the Rectangle, looking along +Y: green inferred. */
const LOW_LEFT: V3 = [-1.5, -6, 1]
/** Low and to the right, looking along -X: red inferred. */
const LOW_RIGHT: V3 = [7, -1.5, 1]
/** Low and to the left, looking along +X: red inferred. */
const LOW_LEFT_X: V3 = [-6, -1.5, 1]
/** An ordinary three-quarter view from above: blue inferred. */
const HIGH: V3 = [3.5, -3, 2.5]

const CHIP =
  /^(Endpoint|Midpoint|Intersection|Quadrant|Tangent|Center|On Edge|On Face|On Guide|On Axis|On Plane|Ground|From Point)/

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
  })
  await camera(page, HIGH)
})

async function camera(page: Page, position: V3): Promise<void> {
  await page.evaluate(
    ({ position, target }) => window.__hew_test!.setCamera({ position, target, up: [0, 0, 1], fovDeg: 45 }),
    { position, target: TARGET },
  )
  await page.waitForTimeout(250)
}

async function toPage(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const p = await page.evaluate((world) => window.__hew_test!.worldToScreen(world as V3), w)
  return { x: canvas.x + p.x, y: canvas.y + p.y }
}

/** Hover a world point the way a hand arrives (a few moves, so the snap settles). */
async function hover(page: Page, w: V3): Promise<void> {
  const p = await toPage(page, w)
  await page.mouse.move(p.x + 5, p.y + 4)
  await page.mouse.move(p.x, p.y, { steps: 4 })
  await page.waitForTimeout(100)
}

async function press(page: Page): Promise<void> {
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(100)
}

/** The inference chip's label under the cursor, or '' when none shows. */
async function chip(page: Page): Promise<string> {
  const texts = await page.locator('div:has(> span)').allTextContents()
  const hit = texts.map((t) => t.trim()).find((t) => CHIP.test(t))
  return hit === undefined ? '' : (CHIP.exec(hit) ?? [''])[0]
}

/** Hover `corner` until the chip reads "Endpoint" (the idle snap settles over a few frames). */
async function hoverCorner(page: Page, corner: V3): Promise<void> {
  const p = await toPage(page, corner)
  await expect
    .poll(
      async () => {
        await page.mouse.move(p.x + 1, p.y)
        await page.mouse.move(p.x, p.y)
        await page.waitForTimeout(60)
        return chip(page)
      },
      { timeout: 10_000, message: `the chip never read Endpoint over ${JSON.stringify(corner)}` },
    )
    .toBe('Endpoint')
}

/** R18's hover contract at a corner: "Endpoint", no "projected", the patch
 *  through the corner on the inferred orientation (`axis`: 0 red, 1 green). */
async function expectOffered(page: Page, corner: V3, axis: 0 | 1): Promise<void> {
  await hoverCorner(page, corner)
  expect(await page.locator('text=projected').count(), 'the corner is that point, not its foot on a plane').toBe(0)
  const cue = await page.evaluate(() => window.__hew_test!.getDrawPlaneCue())
  expect(cue, 'a patch shows').not.toBeNull()
  expect(Math.abs(cue!.normal[axis])).toBeCloseTo(1, 9)
  for (let i = 0; i < 3; i++) expect(cue!.through[i]).toBeCloseTo(corner[i], 4)
}

/** A 1 m Rectangle on the ground (exact corners from the harness, as
 *  wireframe-solid.spec.ts does), then the Rectangle tool for the walls. */
async function groundRectangle(page: Page): Promise<void> {
  await page.evaluate(() => window.__hew_test!.drawRectangle([0, 0, 0], [1, 1, 0]))
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)
}

/** Start a wall on `corner` (already hovered and offered) and size it 1 m x
 *  1 m toward `toward` by typing. */
async function stand(page: Page, toward: V3): Promise<void> {
  await press(page)
  await hover(page, toward)
  await page.keyboard.type('1,1')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
}

function allSegments(page: Page): Promise<number[][]> {
  return page.evaluate(() => {
    const t = window.__hew_test!
    return t.getSketchIds().flatMap((id) => t.getSketchSegmentsExact(id))
  })
}

/** Whether some drawn edge runs exactly from `a` to `b` (either way). */
function hasEdge(segments: number[][], a: V3, b: V3): boolean {
  const eq = (p: number[], q: V3) => p[0] === q[0] && p[1] === q[1] && p[2] === q[2]
  return segments.some((s) => (eq(s.slice(0, 3), a) && eq(s.slice(3, 6), b)) || (eq(s.slice(0, 3), b) && eq(s.slice(3, 6), a)))
}

test("Kurt's eight steps: four walls from the ground Rectangle's corners close a solid, no arrow key", async ({ page }) => {
  test.setTimeout(120_000)
  await groundRectangle(page)

  // (2)-(4) Low and to the left (green): the FAR corner (0, 1, 0).
  await camera(page, LOW_LEFT)
  await expectOffered(page, [0, 1, 0], 1)
  await stand(page, [1, 1, 1])
  let segments = await allSegments(page)
  expect(hasEdge(segments, [0, 1, 0], [0, 1, 1]), 'the wall stands AT the far corner, on the green plane').toBe(true)
  expect(hasEdge(segments, [0, 1, 1], [1, 1, 1])).toBe(true)

  // (5)-(8) Low and to the right (red): the NEAR corner (1, 0, 0).
  await camera(page, LOW_RIGHT)
  await expectOffered(page, [1, 0, 0], 0)
  await stand(page, [1, 1, 1])
  segments = await allSegments(page)
  expect(hasEdge(segments, [1, 0, 0], [1, 0, 1]), 'the wall stands AT the near corner, on the red plane').toBe(true)
  expect(hasEdge(segments, [1, 0, 1], [1, 1, 1])).toBe(true)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)

  // The remaining walls likewise: green at the origin corner, red from the left.
  await camera(page, LOW_LEFT)
  await expectOffered(page, [0, 0, 0], 1)
  await stand(page, [1, 0, 1])
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount()), 'four faces are not yet a closed shape').toBe(0)

  await camera(page, LOW_LEFT_X)
  await expectOffered(page, [0, 0, 0], 0)
  await stand(page, [0, 1, 1])

  // The fifth face closes the box: the top fills itself and the solid forms.
  await expect(page.getByText('The closed shape is now a solid.')).toBeVisible()
  const result = await page.evaluate(() => {
    const t = window.__hew_test!
    const id = t.getObjectIds()[0]
    return { error: t.getLastError(), count: t.getObjectCount(), solid: id === undefined ? null : t.isObjectSolid(id), bounds: id === undefined ? null : t.getObjectBounds(id) }
  })
  expect(result.error).toBeNull()
  expect(result.count).toBe(1)
  expect(result.solid).toBe(true)
  const b = result.bounds!
  const want = [0, 0, 0, 1, 1, 1]
  for (let i = 0; i < 6; i++) expect(b[i]).toBeCloseTo(want[i], 5)
})

test('with Shift pinned on the inferred plane the far corner is still offered and the wall stands at it', async ({ page }) => {
  test.setTimeout(90_000)
  await groundRectangle(page)

  // Pin the green patch from empty space, then hover the far corner.
  await camera(page, LOW_LEFT)
  await hover(page, [3, 0, 2])
  await page.keyboard.down('Shift')
  await page.keyboard.up('Shift')
  await expect(page.getByText(/Pinned to the hovered plane/).first()).toBeVisible()
  await expectOffered(page, [0, 1, 0], 1)
  await stand(page, [1, 1, 1])
  let segments = await allSegments(page)
  expect(hasEdge(segments, [0, 1, 0], [0, 1, 1]), 'pinned green: the wall stands at the far corner').toBe(true)

  // The same from the right with red: release the pin, orbit, pin again.
  await page.keyboard.down('Shift')
  await page.keyboard.up('Shift')
  await camera(page, LOW_RIGHT)
  await hover(page, [0, -3, 2])
  await page.keyboard.down('Shift')
  await page.keyboard.up('Shift')
  await expect(page.getByText(/Pinned to the hovered plane/).first()).toBeVisible()
  await expectOffered(page, [1, 0, 0], 0)
  await stand(page, [1, 1, 1])
  segments = await allSegments(page)
  expect(hasEdge(segments, [1, 0, 0], [1, 0, 1]), 'pinned red: the wall stands at the near corner').toBe(true)
})

test('an ordinary view keeps the ground at a corner of the ground Rectangle', async ({ page }) => {
  test.setTimeout(60_000)
  await groundRectangle(page)
  await hoverCorner(page, [1, 1, 0])
  expect(await page.locator('text=projected').count()).toBe(0)
  expect(await page.evaluate(() => window.__hew_test!.getDrawPlaneCue()), 'the ground has no patch').toBeNull()
  await press(page)
  await hover(page, [2, 2, 0])
  await page.keyboard.type('1,1')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  const segments = await allSegments(page)
  expect(hasEdge(segments, [1, 1, 0], [2, 1, 0]), 'a ground Rectangle from the corner').toBe(true)
})
