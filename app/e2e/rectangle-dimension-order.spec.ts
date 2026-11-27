import { test, expect, type Page } from '@playwright/test'
import { inferenceChip } from './helpers/inferenceChip'

/**
 * A rectangle's two numbers read width across the surface, then height up
 * it — on the ground, on the top of a box, and on a wall alike — and typing
 * that pair back (after the first click, or as a retype after the second)
 * builds exactly the rectangle the readout described. Driven with REAL
 * pointer events over a pinned camera against the harness's unit cube.
 *
 * Why this needs pinning: the plane basis (`facePlaneBasis`) puts world Y
 * first on a horizontal face and the vertical axis first on a wall, so a
 * readout or a typed commit that follows the raw basis says depth × width
 * on a box top and height × width on a wall. `rectangleDimensionAxes`
 * reorders it, and every path must measure along the same reordered pair.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA = { position: [3.2, -3.6, 2.4] as [number, number, number], target: [0.5, 0.5, 0.5] as [number, number, number], up: [0, 0, 1] as [number, number, number], fovDeg: 45 }

interface Ctx {
  rect: { x: number; y: number; width: number; height: number }
  cube: string
}

async function boot(page: Page): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  // A unit cube on the ground: x,y ∈ [0,1], z ∈ [0,1]. Its south face is y = 0.
  const cube = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  await page.waitForTimeout(150)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  return { rect: box, cube }
}

async function px(page: Page, ctx: Ctx, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: ctx.rect.x + p.x, y: ctx.rect.y + p.y }
}

/** Two-step move (so the snap service sees a real pointer stream), then settle. */
async function hover(page: Page, ctx: Ctx, w: [number, number, number]): Promise<void> {
  const p = await px(page, ctx, w)
  await page.mouse.move(p.x - 6, p.y - 4)
  await page.waitForTimeout(50)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(160)
}

async function click(page: Page, ctx: Ctx, w: [number, number, number]): Promise<void> {
  await hover(page, ctx, w)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

/** The inference chip's text (`InferenceTooltip`), or '' when none is shown. */
const chip = inferenceChip

/** The docked VCB's value text for `label`. */
async function vcb(page: Page, label: string): Promise<string> {
  return page.evaluate((label) => {
    const span = Array.from(document.querySelectorAll('span')).find((el) => el.textContent === label)
    return (span?.parentElement?.textContent ?? '').replace(label, '').replace(/\|\s*$/, '').trim()
  }, label)
}

/** The axis-aligned extents [x, y, z] of the cube's single sub-face imprint. */
async function imprintExtents(page: Page, ctx: Ctx): Promise<[number, number, number]> {
  const imprints = await page.evaluate((id) => window.__hew_test!.getImprints(id), ctx.cube)
  expect(imprints.filter((i) => i.kind === 'sub_face')).toHaveLength(1)
  const loop = imprints.find((i) => i.kind === 'sub_face')!.loop
  const ext = (k: 0 | 1 | 2) => Math.max(...loop.map((p) => p[k])) - Math.min(...loop.map((p) => p[k]))
  return [ext(0), ext(1), ext(2)].map((x) => Math.round(x * 1e6) / 1e6) as [number, number, number]
}

async function typeDims(page: Page, dims: string): Promise<void> {
  await page.keyboard.type(dims)
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
}

test('on the top of a box the pair reads X then Y like the ground, and typing it back (then retyping) builds that', async ({ page }) => {
  // The retype undoes and recommits the imprint under a stationary cursor;
  // the tool's memoized face pick then names the sub-face the undo removed,
  // and the next key press re-resolves the snap through it. That must read
  // as a miss (worldFacePlane's documented null), never escape as an
  // uncaught kernel error.
  const pageErrors: string[] = []
  page.on('pageerror', (e) => pageErrors.push(e.message))
  const ctx = await boot(page)
  await page.keyboard.press('r')
  // 0.6 east, 0.3 north on the top face.
  await click(page, ctx, [0.2, 0.2, 1])
  await hover(page, ctx, [0.8, 0.5, 1])
  expect(await chip(page)).toBe('On Face')
  expect(await vcb(page, 'Value')).toBe('0.6 m × 0.3 m')

  await typeDims(page, '0.6,0.3')
  expect(await imprintExtents(page, ctx)).toEqual([0.6, 0.3, 0])

  // The retype window is open after the commit: the same order applies.
  await typeDims(page, '0.4,0.2')
  expect(await imprintExtents(page, ctx)).toEqual([0.4, 0.2, 0])
  expect(pageErrors).toEqual([])
})

test('on a wall the pair reads width across then height up, whichever way it was dragged, and typing it back builds that', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('r')
  // Dragging east and up on the south face: 0.6 wide, 0.3 tall.
  await click(page, ctx, [0.2, 0, 0.2])
  await hover(page, ctx, [0.8, 0, 0.5])
  expect(await chip(page)).toBe('On Face')
  expect(await vcb(page, 'Value')).toBe('0.6 m × 0.3 m')
  await typeDims(page, '0.6,0.3')
  expect(await imprintExtents(page, ctx)).toEqual([0.6, 0, 0.3])
  await page.keyboard.press('Escape')
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)

  // Dragging west and up — the winding-reversing direction — reads the same
  // way round: 0.4 wide, 0.3 tall.
  await page.keyboard.press('r')
  await click(page, ctx, [0.7, 0, 0.3])
  await hover(page, ctx, [0.3, 0, 0.6])
  expect(await chip(page)).toBe('On Face')
  expect(await vcb(page, 'Value')).toBe('0.4 m × 0.3 m')
  await typeDims(page, '0.4,0.3')
  expect(await imprintExtents(page, ctx)).toEqual([0.4, 0, 0.3])
})
