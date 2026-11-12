import { test, expect, type Page, type TestInfo } from '@playwright/test'
import { inferenceChip } from './helpers/inferenceChip'

/**
 * A value you CLICK is exactly the value the measurement box shows, as if
 * you had typed it — unless the click snapped onto existing geometry, which
 * keeps its exact point. Driven with REAL pointer events over a pinned
 * camera in millimetres: hover until the box reads a value, click, then read
 * the committed geometry back and compare it with the text that was on
 * screen at the click.
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
  await page.evaluate(() => window.__hew_test!.setLengthUnit('mm'))
  // A unit cube on the ground: x,y in [0,1], z in [0,1].
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
async function hover(page: Page, ctx: Ctx, w: [number, number, number], dx = 0, dy = 0): Promise<void> {
  const p = await px(page, ctx, w)
  await page.mouse.move(p.x + dx - 6, p.y + dy - 4)
  await page.waitForTimeout(50)
  await page.mouse.move(p.x + dx, p.y + dy)
  await page.waitForTimeout(160)
}

async function click(page: Page, ctx: Ctx, w: [number, number, number], dx = 0, dy = 0): Promise<void> {
  await hover(page, ctx, w, dx, dy)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
}

const chip = inferenceChip

async function vcb(page: Page, label: string): Promise<string> {
  return page.evaluate((label) => {
    const span = Array.from(document.querySelectorAll('span')).find((el) => el.textContent === label)
    return (span?.parentElement?.textContent ?? '').replace(label, '').replace(/\|\s*$/, '').trim()
  }, label)
}

/** "412.3 mm" -> metres, the way typing it would parse. */
function mm(text: string): number {
  const m = /^~?(-?\d+(?:\.\d+)?)\s*mm$/.exec(text.trim())
  if (m === null) throw new Error(`not a millimetre reading: "${text}"`)
  return parseFloat(m[1]) * 0.001
}

async function shot(page: Page, info: TestInfo, name: string): Promise<void> {
  await page.screenshot({ path: info.outputPath(`${name}.png`) })
}

/** Every sketch edge on the page as exact [ax,ay,az,bx,by,bz]. */
async function allSketchSegments(page: Page): Promise<number[][]> {
  // Exact f64 endpoints — `getSketchLines` is the f32 render buffer.
  const perSketch = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getSketchIds().map((id) => h.getSketchSegmentsExact(id))
  })
  return perSketch.flat()
}

test('ground rectangle: the committed width and depth are exactly the displayed ones', async ({ page }, info) => {
  const ctx = await boot(page)
  await page.keyboard.press('r')
  await click(page, ctx, [-0.9, -0.6, 0])
  // Off-grid pixel offset so the raw extents carry digits the box rounds away.
  await hover(page, ctx, [-0.3, -0.2, 0], 3, 2)
  const text = await vcb(page, 'Value')
  const [wText, dText] = text.split(' × ')
  await shot(page, info, 'rect-ground-before')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
  await shot(page, info, 'rect-ground-after')

  const segs = (await allSketchSegments(page))
  const xs = segs.flatMap((s) => [s[0], s[3]])
  const ys = segs.flatMap((s) => [s[1], s[4]])
  expect(Math.max(...xs) - Math.min(...xs)).toBeCloseTo(mm(wText), 12)
  expect(Math.max(...ys) - Math.min(...ys)).toBeCloseTo(mm(dText), 12)
})

test('line: the committed length is exactly the displayed length', async ({ page }, info) => {
  const ctx = await boot(page)
  await page.keyboard.press('l')
  await click(page, ctx, [-0.9, -0.6, 0])
  await hover(page, ctx, [-0.35, -0.15, 0], 2, 3)
  const text = await vcb(page, 'Value')
  await shot(page, info, 'line-before')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
  await shot(page, info, 'line-after')

  const segs = await allSketchSegments(page)
  expect(segs.length).toBe(1)
  const s = segs[0]
  expect(Math.hypot(s[3] - s[0], s[4] - s[1], s[5] - s[2])).toBeCloseTo(mm(text), 12)
})

test('rectangle on a box top: the imprint is exactly the displayed width and depth', async ({ page }, info) => {
  const ctx = await boot(page)
  await page.keyboard.press('r')
  await click(page, ctx, [0.2, 0.2, 1])
  await hover(page, ctx, [0.8, 0.55, 1], 2, 1)
  expect(await chip(page)).toBe('On Face')
  const [wText, dText] = (await vcb(page, 'Value')).split(' × ')
  await shot(page, info, 'rect-face-before')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
  await shot(page, info, 'rect-face-after')

  const imprints = await page.evaluate((id) => window.__hew_test!.getImprints(id), ctx.cube)
  const loop = imprints.find((i) => i.kind === 'sub_face')!.loop
  const ext = (k: 0 | 1) => Math.max(...loop.map((p) => p[k])) - Math.min(...loop.map((p) => p[k]))
  expect(ext(0)).toBeCloseTo(mm(wText), 12)
  expect(ext(1)).toBeCloseTo(mm(dText), 12)
})

test('a click snapped onto an existing endpoint keeps that exact point (no rounding)', async ({ page }, info) => {
  const ctx = await boot(page)
  await page.keyboard.press('l')
  await click(page, ctx, [-0.9, -0.6, 0])
  // The cube's ground corner (0, 0, 0).
  await hover(page, ctx, [0, 0, 0])
  expect(await chip(page)).toBe('Endpoint')
  await shot(page, info, 'endpoint-before')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
  await shot(page, info, 'endpoint-after')

  const segs = await allSketchSegments(page)
  expect(segs.length).toBe(1)
  expect(segs[0].slice(3)).toEqual([0, 0, 0])
})
