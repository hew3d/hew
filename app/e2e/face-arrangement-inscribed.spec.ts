/**
 * A circle drawn inside a rectangle imprint so that it touches the
 * rectangle's sides at isolated points splits the rectangle the way the same
 * shapes split a ground sketch (the face-arrangement planner,
 * `Object::plan_face_arrangement`): the disc plus three leftovers, every arc
 * keeping its circle. Driven with REAL pointer and key events on the front
 * (y = 0) wall of a box: the rectangle is 9.4 × 3.8 mm, and the circle of
 * radius 1.9 mm is centered 1.9 mm in from its left end, so it is tangent to
 * the left, top, and bottom sides at three of its own vertices. Before the
 * planner this was refused LoopNotStrictlyInside.
 *
 * Two ways in: the rim clicked on the rectangle's left-side midpoint, and a
 * radius typed after a shorter rim click (the retype window resizes the
 * circle in place along the clicked direction).
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
// A shorter rim, straight left of the center, for the typed-radius path.
const XR = 0.0072

async function setup(page: Page): Promise<string> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const object = await page.evaluate(
    ({ X0, X1, XC, XR, Z0, Z1, ZC }) => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [0.022, 0.0275, 0], 0.007)
      // Looking straight at the y = 0 wall from the front, close in.
      h.setCamera({ position: [0.011, -0.025, ZC], target: [0.011, 0, ZC], up: [0, 0, 1], fovDeg: 45 })
      // Guides whose intersections are every point the gestures click.
      for (const x of [X0, X1, XC, XR]) h.addGuideLine(x, 0, ZC, 0, 0, 1)
      for (const z of [Z0, Z1, ZC]) h.addGuideLine(0.011, 0, z, 1, 0, 0)
      return id
    },
    { X0, X1, XC, XR, Z0, Z1, ZC },
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

async function drawRectangle(page: Page, object: string): Promise<void> {
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await click(page, [X0, 0, Z0])
  await click(page, [X1, 0, Z1])
  const list = await imprints(page, object)
  expect(list.length).toBe(1)
  expect(list[0].kind).toBe('sub_face')
  const xs = list[0].loop.map((p) => p[0])
  const zs = list[0].loop.map((p) => p[2])
  expect(Math.min(...xs)).toBeCloseTo(X0, 6)
  expect(Math.max(...xs)).toBeCloseTo(X1, 6)
  expect(Math.min(...zs)).toBeCloseTo(Z0, 6)
  expect(Math.max(...zs)).toBeCloseTo(Z1, 6)
}

/** The disc + three leftovers, as the ground gives for the same shapes. */
async function expectArranged(page: Page, object: string): Promise<void> {
  await expect(page.getByText(/would run off the face/)).toHaveCount(0)
  // No sub-face any more: three arc runs between the disc and each leftover,
  // every facet claiming the drawn circle, and each leftover's run of
  // rectangle edges along the wall (selectable and deletable on its own).
  const list = await imprints(page, object)
  expect(list.map((f) => f.kind)).toEqual(Array(6).fill('chord'))
  const arcs = list.filter((f) => f.curves.some((c) => c !== null))
  expect(arcs.length).toBe(3)
  expect(list.filter((f) => f.curves.every((c) => c === null)).length).toBe(3)
  for (const f of arcs) {
    expect(f.curves.length).toBeGreaterThan(0)
    for (const c of f.curves) {
      expect(c).not.toBeNull()
      expect(c!.radius).toBeCloseTo(R, 9)
      expect(c!.center[0]).toBeCloseTo(XC, 9)
      expect(c!.center[2]).toBeCloseTo(ZC, 9)
    }
  }
  // Five distinct coplanar faces on the wall: disc, top-left and
  // bottom-left corner leftovers, the right leftover, and the wall itself.
  const d = 0.0002
  const faces = await Promise.all([
    faceAt(page, XC, ZC),
    faceAt(page, X0 + d, Z1 - d),
    faceAt(page, X0 + d, Z0 + d),
    faceAt(page, X1 - 0.001, ZC),
    faceAt(page, 0.002, 0.001),
  ])
  for (const f of faces) expect(f).not.toBeNull()
  expect(new Set(faces).size).toBe(5)
}

async function expectPlainRectangle(page: Page, object: string): Promise<void> {
  const list = await imprints(page, object)
  expect(list.length).toBe(1)
  expect(list[0].kind).toBe('sub_face')
  expect(list[0].name).toBe('Rectangle')
}

test('a circle tangent to three sides of a rectangle on a wall, rim on the left midpoint, splits it into four faces', async ({
  page,
}, testInfo) => {
  const object = await setup(page)
  await drawRectangle(page, object)
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('1-rectangle.png') })

  await page.getByRole('radio', { name: 'Circle' }).click()
  await click(page, [XC, 0, ZC])
  await click(page, [X0, 0, ZC]) // the rectangle's left-side midpoint
  await expectArranged(page, object)
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('2-arranged.png') })

  // One undo entry: Ctrl+Z restores the plain rectangle, redo the arrangement.
  await page.keyboard.press('Escape')
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  await expectPlainRectangle(page, object)
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(250)
  await expectArranged(page, object)
})

test('a typed 1.9mm radius after a shorter rim click makes the same four faces', async ({ page }, testInfo) => {
  const object = await setup(page)
  await drawRectangle(page, object)

  await page.getByRole('radio', { name: 'Circle' }).click()
  await click(page, [XC, 0, ZC])
  await click(page, [XR, 0, ZC]) // 1 mm straight left: a small, clear circle
  let list = await imprints(page, object)
  expect(list.map((f) => f.kind).sort()).toEqual(['sub_face', 'sub_face'])
  await page.keyboard.type('1.9mm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  await expectArranged(page, object)
  await page.locator('canvas').first().screenshot({ path: testInfo.outputPath('3-typed.png') })

  await page.keyboard.press('Escape')
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(250)
  list = await imprints(page, object)
  // The retype replaced the small circle in place, so one undo goes back to
  // the plain rectangle.
  expect(list.map((f) => f.kind)).toEqual(['sub_face'])
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForTimeout(250)
  await expectArranged(page, object)
})
