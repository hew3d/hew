/**
 * Unified drafting, the modify tools with REAL pointer events: the plane is
 * the drafting context, so Offset, Edit Vertex, and Push/Pull act on drawing
 * on a solid's face exactly as they do on the ground.
 *
 *  - Offset a rectangle on a box top outward past the edge: the loop splits
 *    into a face part and a free part.
 *  - Edit Vertex drags a vertex of a drawing on a face (a solid's own corner
 *    is not a drawing vertex).
 *  - Push/Pull the free part of a straddling rectangle (a new object) and its
 *    face part (the box grows), in either order.
 */
import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

async function pagePoint(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function hover(page: Page, w: V3): Promise<void> {
  const at = await pagePoint(page, w)
  await page.mouse.move(at.x - 3, at.y + 2)
  await page.waitForTimeout(60)
  await page.mouse.move(at.x, at.y)
  await page.waitForTimeout(120)
}

async function click(page: Page, w: V3): Promise<void> {
  await hover(page, w)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

async function typeAndEnter(page: Page, text: string): Promise<void> {
  await page.keyboard.type(text)
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
}

async function pickTool(page: Page, name: string): Promise<void> {
  const radio = page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name })
  if ((await radio.count()) > 0) {
    await radio.click()
    return
  }
  // Palette-only tools (Edit Vertex) live in the Tools menu.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Tools' }).click()
  await page.getByText(name, { exact: true }).last().click()
}

const lastError = (page: Page) => page.evaluate(() => window.__hew_test!.getLastError())
const objectCount = (page: Page) => page.evaluate(() => window.__hew_test!.getObjectCount())
const stateHash = (page: Page) => page.evaluate(() => window.__hew_test!.getModelStateHash())
const bounds = (page: Page, o: string) => page.evaluate((o) => window.__hew_test!.getObjectBounds(o), o)

async function freeRegions(page: Page): Promise<Array<{ sketch: string; region: string }>> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    const out: Array<{ sketch: string; region: string }> = []
    for (const id of h.getSketchIds()) for (const r of h.getSketchRegionIds(id)) out.push({ sketch: id, region: r })
    return out
  })
}

async function drawRectOnTop(page: Page, a: V3, b: V3): Promise<void> {
  await pickTool(page, 'Rectangle')
  await click(page, a)
  await click(page, b)
  expect(await lastError(page)).toBeNull()
}

let box: string

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [1, 1, 0], 0.5)
    // Looking down onto the top face (z = 0.5), tilted enough that a lift reads.
    h.setCamera({ position: [0.6, -1.2, 3.2], target: [0.7, 0.5, 0.5], up: [0, 1, 0], fovDeg: 45 })
  })
  box = (await page.evaluate(() => window.__hew_test!.getObjectIds()))[0]
  await page.waitForTimeout(250)
})

test('Offset outward past the top edge splits into a face part and a free part', async ({ page }) => {
  await drawRectOnTop(page, [0.5, 0.25, 0.5], [0.9, 0.75, 0.5])
  const hashBefore = await stateHash(page)
  const freeBefore = (await freeRegions(page)).length

  await pickTool(page, 'Offset')
  await click(page, [0.7, 0.5, 0.5]) // inside the rectangle
  await hover(page, [1.1, 0.5, 0.5]) // 0.2 m outside its right edge — past the box edge
  await click(page, [1.1, 0.5, 0.5])
  expect(await lastError(page)).toBeNull()
  expect(await stateHash(page)).not.toBe(hashBefore)

  // Face part: the ring between the rectangle and the loop is a face of the box,
  // distinct from the plain top beyond the loop.
  const ring = await page.evaluate(() => window.__hew_test!.pickFace([0.95, 0.5, 4], [0, 0, -1]))
  const plain = await page.evaluate(() => window.__hew_test!.pickFace([0.1, 0.1, 4], [0, 0, -1]))
  expect(ring).not.toBeNull()
  expect(plain).not.toBeNull()
  expect(ring!.face).not.toBe(plain!.face)

  // Free part: the strip past the edge is a region of its own, extrudable.
  const free = await freeRegions(page)
  expect(free.length).toBeGreaterThan(freeBefore)
  const before = await objectCount(page)
  await page.evaluate(([s, r]) => window.__hew_test!.extrudeRegion(s, r, 0.3), [free[0].sketch, free[0].region])
  expect(await objectCount(page)).toBe(before + 1)
  // The solid itself did not move.
  const b = await bounds(page, box)
  expect(b[3]).toBeCloseTo(1, 6)

  // The offset is one undo step.
  await page.evaluate(() => window.__hew_test!.undo()) // the extrusion
  await page.evaluate(() => window.__hew_test!.undo()) // the offset
  expect(await stateHash(page)).toBe(hashBefore)
})

test('Edit Vertex drags a drawn vertex on a face; a solid corner is not pickable', async ({ page }) => {
  await drawRectOnTop(page, [0.25, 0.25, 0.5], [0.75, 0.75, 0.5])
  const probe = [0.78, 0.8, 4] as V3
  const faceAt = (p: V3) => page.evaluate((p) => window.__hew_test!.pickFace(p, [0, 0, -1]), p)
  const outerBefore = await faceAt([0.1, 0.1, 4])
  const probeBefore = await faceAt(probe)
  expect(probeBefore!.face).toBe(outerBefore!.face) // outside the rectangle for now

  await pickTool(page, 'Edit Vertex')

  // The box's own corner picks nothing: the next click is a no-op.
  const hash0 = await stateHash(page)
  await click(page, [0, 0, 0.5])
  await click(page, [0.1, 0.05, 0.5])
  expect(await stateHash(page)).toBe(hash0)

  // The rectangle's corner moves, taking its two edges along.
  await click(page, [0.75, 0.75, 0.5])
  await hover(page, [0.85, 0.9, 0.5])
  await click(page, [0.85, 0.9, 0.5])
  expect(await lastError(page)).toBeNull()
  expect(await stateHash(page)).not.toBe(hash0)
  const probeAfter = await faceAt(probe)
  const outerAfter = await faceAt([0.1, 0.1, 4])
  expect(probeAfter!.face).not.toBe(outerAfter!.face) // now inside the drawn shape

  await page.evaluate(() => window.__hew_test!.undo())
  expect(await stateHash(page)).toBe(hash0)
})

/** A rectangle half over the top's right edge: face part (0.5..1) and free part (1..1.5). */
async function straddle(page: Page): Promise<void> {
  await drawRectOnTop(page, [0.5, 0.25, 0.5], [1.5, 0.75, 0.5])
}

async function pushFreePart(page: Page, distance: string): Promise<void> {
  await pickTool(page, 'Push/Pull')
  await click(page, [1.25, 0.5, 0.5])
  await typeAndEnter(page, distance)
  expect(await lastError(page)).toBeNull()
}

async function pushFacePart(page: Page, distance: string): Promise<void> {
  await pickTool(page, 'Push/Pull')
  await click(page, [0.75, 0.5, 0.5])
  await typeAndEnter(page, distance)
  expect(await lastError(page)).toBeNull()
}

async function newObjectBounds(page: Page): Promise<[number, number, number, number, number, number]> {
  const id = (await page.evaluate(() => window.__hew_test!.getObjectIds())).find((o) => o !== box)
  expect(id).toBeDefined()
  return bounds(page, id!)
}

test('Push/Pull: the free part first (a new object), then the face part (the box grows)', async ({ page }) => {
  await straddle(page)
  await pushFreePart(page, '0.3')
  expect(await objectCount(page)).toBe(2)
  const free = await newObjectBounds(page)
  expect(free[0]).toBeCloseTo(1, 5)
  expect(free[3]).toBeCloseTo(1.5, 5)
  expect(free[5] - free[2]).toBeCloseTo(0.3, 5)

  await pushFacePart(page, '0.2')
  expect(await objectCount(page)).toBe(2)
  const b = await bounds(page, box)
  expect(b[5]).toBeCloseTo(0.7, 5) // the face part rose; the box stays one object
  expect(b[3]).toBeCloseTo(1, 5)
})

test('Push/Pull: the face part first (the box grows), then the free part (the same new object)', async ({ page }) => {
  await straddle(page)
  await pushFacePart(page, '0.2')
  expect(await objectCount(page)).toBe(1)
  expect((await bounds(page, box))[5]).toBeCloseTo(0.7, 5)

  await pushFreePart(page, '0.3')
  expect(await objectCount(page)).toBe(2)
  const free = await newObjectBounds(page)
  expect(free[0]).toBeCloseTo(1, 5)
  expect(free[3]).toBeCloseTo(1.5, 5)
  expect(free[5] - free[2]).toBeCloseTo(0.3, 5)
})
