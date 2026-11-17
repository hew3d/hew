import { test, expect, type Page, type TestInfo } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * A sketch shape moved onto other shapes joins them exactly as drawing it
 * there would (unified drafting, Stage 0): lines split and merge, regions
 * re-form, one undo step. Same for dragging a vertex across other lines
 * with Edit Vertex. Both are driven with REAL pointer events on the canvas;
 * the harness only pins the camera and reads state back.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/** Straight down onto the ground, +Y up the screen. */
const CAMERA: CameraParams = {
  position: { x: 3, y: 1, z: 10 },
  target: { x: 3, y: 1, z: 0 },
  up: { x: 0, y: 1, z: 0 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

interface Ctx {
  vp: Mat4
  rect: { left: number; top: number; width: number; height: number }
}

function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

async function setup(page: Page): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await page.evaluate(
    (c) =>
      window.__hew_test!.setCamera({
        position: [c.position.x, c.position.y, c.position.z],
        target: [c.target.x, c.target.y, c.target.z],
        up: [c.up.x, c.up.y, c.up.z],
        fovDeg: c.fovDeg,
      }),
    CAMERA,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(CAMERA, rect.width / rect.height), rect }
}

async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z = 0): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 3, p.y - 3)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(60)
  await page.mouse.down()
  await page.mouse.up()
}

/** Rectangle tool: click the corner, steer toward `(cx, cy)` for the
 * quadrant, type `w,h`, Enter — all through the real tool. */
async function drawRect(
  page: Page,
  ctx: Ctx,
  corner: [number, number],
  size: [number, number],
): Promise<void> {
  await page.keyboard.press('r')
  await clickWorld(page, ctx, corner[0], corner[1])
  const steer = px(ctx, corner[0] + size[0] * 0.5, corner[1] + size[1] * 0.5, 0)
  await page.mouse.move(steer.x, steer.y)
  await page.keyboard.type(`${size[0]},${size[1]}`)
  await page.keyboard.press('Enter')
}

async function shot(page: Page, testInfo: TestInfo, name: string): Promise<void> {
  await page.screenshot({ path: testInfo.outputPath(`${name}.png`) })
}

async function sketchState(
  page: Page,
): Promise<{ sketches: number; islands: number; regions: number }> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    const ids = h.getSketchIds()
    let islands = 0
    let regions = 0
    for (const s of ids) {
      islands += h.getSketchIslands(s).length
      regions += h.getSketchRegionCount(s)
    }
    return { sketches: ids.length, islands, regions }
  })
}

/** No kernel-refusal toast about lines crossing/merging is on screen. */
async function expectNoRefusalToast(page: Page): Promise<void> {
  await expect(page.getByText(/would make lines cross or merge/)).toHaveCount(0)
  await expect(page.getByText(/WouldRetopologize/)).toHaveCount(0)
}

test('Move one ground rectangle half over another: they merge into one island with three regions; undo/redo round-trips', async ({
  page,
}, testInfo) => {
  const ctx = await setup(page)

  // Two separate rectangles through the real Rectangle tool. The right one
  // is offset in Y so the landing CROSSES edges rather than overlapping
  // collinearly.
  await drawRect(page, ctx, [0, 0], [2, 1.5])
  await drawRect(page, ctx, [4, 0.5], [2, 1.5])
  expect(await sketchState(page)).toEqual({ sketches: 1, islands: 2, regions: 2 })
  await shot(page, testInfo, 'before')

  // Select the left rectangle (click its left edge) with the Select tool.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0, 0.75)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  // Move: base on that edge, lock X, steer to +X, type 3 m. The left
  // rectangle (x 0..2) lands at x 3..5, half over the right one (x 4..6).
  await page.keyboard.press('m')
  await clickWorld(page, ctx, 0, 0.75)
  await page.keyboard.press('ArrowRight')
  const steer = px(ctx, 1.5, 0.75, 0)
  await page.mouse.move(steer.x, steer.y)
  await page.keyboard.type('3')
  await page.keyboard.press('Enter')

  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().every((s) => h.getSketchIslands(s).length === 1)
  })
  expect(await sketchState(page)).toEqual({ sketches: 1, islands: 1, regions: 3 })
  await expectNoRefusalToast(page)

  // The moved shape is still selected: as the merged island it now lives in.
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel.length).toBeGreaterThan(0)
  const merged = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getPlaneUnits(h.ctxGround()).units[0].key
  })
  expect(sel.some((n) => n.kind === 'draft-island' && n.key === merged)).toBe(true)
  await shot(page, testInfo, 'after')

  // One undo restores the two separate shapes; redo re-applies the merge.
  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().reduce((n, s) => n + h.getSketchIslands(s).length, 0) === 2
  })
  expect(await sketchState(page)).toEqual({ sketches: 1, islands: 2, regions: 2 })
  await page.keyboard.press('ControlOrMeta+Shift+Z')
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().reduce((n, s) => n + h.getSketchIslands(s).length, 0) === 1
  })
  expect(await sketchState(page)).toEqual({ sketches: 1, islands: 1, regions: 3 })
})

test('Edit Vertex: dragging a rectangle corner across the opposite side re-derives the regions', async ({
  page,
}, testInfo) => {
  const ctx = await setup(page)

  await drawRect(page, ctx, [0, 0], [2, 1.5])
  expect(await sketchState(page)).toEqual({ sketches: 1, islands: 1, regions: 1 })
  await shot(page, testInfo, 'vertex-before')

  // Edit Vertex has no shortcut: pick it from the Tools menu.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Tools' }).click()
  await page.getByText('Edit Vertex', { exact: true }).click()

  // Grab the (0,0) corner, then drop it at (3, 0.75): its edges now sweep
  // across the far side and the shape arranges into two triangles.
  await clickWorld(page, ctx, 0, 0)
  await clickWorld(page, ctx, 3, 0.75)

  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().reduce((n, s) => n + h.getSketchRegionCount(s), 0) === 2
  })
  await expectNoRefusalToast(page)
  await shot(page, testInfo, 'vertex-after')

  await page.keyboard.press('ControlOrMeta+z')
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().reduce((n, s) => n + h.getSketchRegionCount(s), 0) === 1
  })
  expect(await sketchState(page)).toEqual({ sketches: 1, islands: 1, regions: 1 })
})
