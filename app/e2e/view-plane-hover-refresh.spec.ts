import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * The idle plane cue is hover (docs/design/view-plane-inference.md §5), so
 * it follows the hover when no pointer move does:
 *
 * - The pointer leaving the canvas takes the patch and the "Facing" line
 *   with it; coming back brings them back. A held lock keeps its plane but
 *   its patch goes too.
 * - A document change under a parked pointer (Delete, undo) re-resolves
 *   the hover there, so the tool's plane shows what a click would now do
 *   instead of a sketch that is gone.
 *
 * Fixture `sketch-planes.hew` (see sketch-fill-adopt.spec.ts): three
 * sketches through the origin; the red one (normal +X, x = 0) is a
 * rectangle y 0.4172..1.4532, z 0.2304..0.5124. Real pointer and key events
 * throughout; the patch is read from the cue layer as drawn.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pointer-driven spec, chromium only')

type V3 = [number, number, number]

const FIXTURE = resolve(process.cwd(), 'e2e/fixtures/sketch-planes.hew')
const RED_CENTER: V3 = [0, 0.935, 0.371]

async function setup(page: Page, fixture: boolean): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  if (fixture) {
    await page.evaluate((arr) => window.__hew_test!.load(arr), Array.from(readFileSync(FIXTURE)))
    await page.waitForTimeout(200)
  }
}

async function aim(page: Page, position: V3, target: V3): Promise<void> {
  await page.evaluate(
    (cam) => window.__hew_test!.setCamera({ ...cam, up: [0, 0, 1], fovDeg: 45 }),
    { position, target },
  )
  await page.waitForTimeout(400) // past the camera-settle replay
}

async function canvasBox(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  return canvas
}

async function toPage(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const canvas = await canvasBox(page)
  const p = await page.evaluate((world) => window.__hew_test!.worldToScreen(world as V3), w)
  return { x: canvas.x + p.x, y: canvas.y + p.y }
}

/** Hover a page point the way a hand arrives (a few moves). */
async function hoverAt(page: Page, x: number, y: number): Promise<void> {
  await page.mouse.move(x + 5, y + 5)
  await page.mouse.move(x, y, { steps: 3 })
  await page.waitForTimeout(100)
}

/** Move the pointer off the canvas, onto the page around it, in one jump:
 *  no move on the canvas on the way out re-hovers anything. */
async function leaveCanvas(page: Page): Promise<void> {
  const canvas = await canvasBox(page)
  const x = canvas.x + canvas.width / 2
  const y = canvas.y > 4 ? canvas.y - 3 : canvas.y + canvas.height + 3
  await page.mouse.move(x, y)
  await page.waitForTimeout(100)
}

const cue = (page: Page) => page.evaluate(() => window.__hew_test!.getDrawPlaneCue())
const facing = (page: Page) => page.getByText(/Facing the/).first()

const planeState = (page: Page) => page.evaluate(() => window.__hew_test!.getDrawPlaneState())

/** The tool's plane (its normal along `axis`) and what the pointer is over. */
function expectPlane(s: { normal: V3; hover: string | null } | null, axis: number, hover: string | null): void {
  expect(s).not.toBeNull()
  expect(Math.abs(s!.normal[axis])).toBeCloseTo(1, 6)
  expect(s!.hover).toBe(hover)
}

function expectNormalAlong(c: { normal: V3 } | null, axis: number): void {
  expect(c).not.toBeNull()
  expect(Math.abs(c!.normal[axis])).toBeCloseTo(1, 6)
}

test('leaving the canvas takes the inferred plane\'s patch and Facing line; coming back restores them', async ({ page }) => {
  await setup(page, false)
  // Low and level from the Front side: empty space infers the green plane.
  await aim(page, [0.3, -8, 0.6], [0.3, 0, 0.6])
  await page.keyboard.press('r')
  await page.waitForTimeout(100)
  const canvas = await canvasBox(page)
  const px = canvas.x + canvas.width * 0.6
  const py = canvas.y + canvas.height * 0.4
  await hoverAt(page, px, py)
  expectNormalAlong(await cue(page), 1)
  await expect(facing(page)).toBeVisible()

  await leaveCanvas(page)
  expect(await cue(page)).toBeNull()
  await expect(facing(page)).toHaveCount(0)

  await hoverAt(page, px, py)
  expectNormalAlong(await cue(page), 1)
  await expect(facing(page)).toBeVisible()
})

test('leaving the canvas clears the hover; an arrow lock keeps its plane but its patch goes with the pointer', async ({ page }) => {
  await setup(page, true)
  await aim(page, [3.5, 0.9, 2.0], [0, 0.935, 0.37])
  await page.keyboard.press('r')
  await page.waitForTimeout(100)
  const at = await toPage(page, RED_CENTER)
  // Over a sketch the plane is the sketch's, and the cue is none: the die's
  // pip and the chip name it instead of a patch.
  await hoverAt(page, at.x, at.y)
  expectPlane(await planeState(page), 0, 'sketch')
  expect(await cue(page)).toBeNull()
  await leaveCanvas(page)
  expect((await planeState(page))?.hover).toBeNull()
  expect(await cue(page)).toBeNull()

  // A lock is a choice, not hover: the plane stays locked off the canvas,
  // and its held patch shows while the pointer is on it.
  await hoverAt(page, at.x, at.y)
  await page.keyboard.press('ArrowLeft') // green lock
  await hoverAt(page, at.x, at.y)
  const locked = await cue(page)
  expectNormalAlong(locked, 1)
  expect(locked!.held).toBe(true)
  await leaveCanvas(page)
  const away = await planeState(page)
  expect(away?.hold).toBe('locked')
  expect(Math.abs(away!.normal[1])).toBeCloseTo(1, 6)
  expect(away?.hover).toBeNull()
  expect(await cue(page)).toBeNull()
  await expect(page.getByText(/Locked to the green plane/).first()).toBeVisible()
})

test('Delete and undo under a parked pointer re-resolve the hover to what a click would now do', async ({ page }) => {
  await setup(page, true)
  // From the +X side, about 25° up: over empty space the ground decides
  // (no patch).
  await aim(page, [3.5, 0.9, 2.0], [0, 0.935, 0.37])
  const red = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getSketchIds().find((id) => Math.abs(h.getSketchPlane(id)![3]) > 0.999)!
  })
  await page.keyboard.press('r')
  await page.waitForTimeout(100)
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'sketch', id }]), red)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  // Parked inside the red rectangle: its plane.
  const at = await toPage(page, RED_CENTER)
  await hoverAt(page, at.x, at.y)
  expectPlane(await planeState(page), 0, 'sketch')

  // Delete it (from the Rectangle tool) with the pointer still: the ray
  // now reaches the empty ground, so a click would draw on the ground.
  await page.keyboard.press('Delete')
  await page.waitForTimeout(100)
  expect(await page.evaluate((id) => window.__hew_test!.getSketchIds().includes(id), red)).toBe(false)
  expect((await planeState(page))?.hover).not.toBe('sketch')
  expect(await cue(page)).toBeNull()
  await expect(facing(page)).toHaveCount(0)

  // Undo brings the sketch back under the same parked pointer: its plane too.
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForTimeout(100)
  expect(await page.evaluate(() => window.__hew_test!.getSketchIds().length)).toBe(3)
  expectPlane(await planeState(page), 0, 'sketch')

  // And the click there lands in the red sketch, as the plane said.
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(80)
  const near = await toPage(page, [0, 1.2, 0.45])
  await hoverAt(page, near.x, near.y)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(100)
  const after = await page.evaluate(() => {
    const h = window.__hew_test!
    return { ids: h.getSketchIds(), error: h.getLastError() }
  })
  expect(after.error).toBeNull()
  expect(after.ids).toHaveLength(3)
  expect(await page.evaluate((id) => window.__hew_test!.getSketchRegionCount(id), red)).toBeGreaterThanOrEqual(2)
})
