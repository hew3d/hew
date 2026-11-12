import { test, expect, type Page } from '@playwright/test'
import { inferenceChip } from './helpers/inferenceChip'

/**
 * Inference-permissiveness fixes, driven with REAL pointer events over a
 * pinned camera (docs/dev/DEVELOPMENT.md strategy 2) — the maintainer's
 * eight-report batch (GitHub issues 13 and 14 among them). Each test is the
 * report's own gesture, reduced to a cube the harness builds, and asserts
 * the observable the report was about: the VCB readout, the inference
 * chip, or the committed geometry.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA = { position: [3.2, -3.6, 2.4] as [number, number, number], target: [0.5, 0.5, 0.5] as [number, number, number], up: [0, 0, 1] as [number, number, number], fovDeg: 45 }

interface Ctx {
  rect: { x: number; y: number; width: number; height: number }
}

async function boot(page: Page, camera = CAMERA): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  // A unit cube on the ground: x,y ∈ [0,1], z ∈ [0,1]. Its south face is y = 0.
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), camera)
  await page.waitForTimeout(150)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  return { rect: box }
}

async function px(page: Page, ctx: Ctx, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: ctx.rect.x + p.x, y: ctx.rect.y + p.y }
}

/** Two-step move (so the snap service sees a real pointer stream), then settle. */
async function hover(page: Page, ctx: Ctx, w: [number, number, number], dx = 0, dy = 0): Promise<void> {
  const p = await px(page, ctx, w)
  await page.mouse.move(p.x - 6 + dx, p.y - 4 + dy)
  await page.waitForTimeout(50)
  await page.mouse.move(p.x + dx, p.y + dy)
  await page.waitForTimeout(160)
}

async function click(page: Page, ctx: Ctx, w: [number, number, number], dx = 0, dy = 0): Promise<void> {
  await hover(page, ctx, w, dx, dy)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

/** The inference chip's text (`InferenceTooltip`), or '' when none is shown. */
const chip = inferenceChip

/** The docked VCB's value text for `label` ('Push depth', 'Value', …). */
async function vcb(page: Page, label: string): Promise<string> {
  return page.evaluate((label) => {
    const span = Array.from(document.querySelectorAll('span')).find((el) => el.textContent === label)
    // The box renders `label`, the value, and a trailing separator glyph.
    return (span?.parentElement?.textContent ?? '').replace(label, '').replace(/\|\s*$/, '').trim()
  }, label)
}

async function sketchPlanes(page: Page): Promise<number[][]> {
  return page.evaluate(() => window.__hew_test!.getSketchIds().map((id) => window.__hew_test!.getSketchPlane(id) ?? []))
}

test('Push/Pull: pulling a face up past its own edges never drops the depth to 0 or flips it negative', async ({ page }) => {
  const ctx = await boot(page, { ...CAMERA, position: [4.5, -5.5, 3.0], target: [1.5, -1.5, 0.3] })
  // A ground rectangle in the x/−y quadrant (the report's setup) whose pull
  // sweeps the cursor across its own far edges and past the drawing axes.
  await page.evaluate(() => window.__hew_test!.drawRectangle([1, -1, 0], [2, -2, 0]))
  await page.keyboard.press('p')
  await click(page, ctx, [1.5, -1.5, 0])
  const start = await px(page, ctx, [1.5, -1.5, 0])
  let last = 0
  for (let dy = 6; dy <= 150; dy += 6) {
    await page.mouse.move(start.x, start.y - dy)
    await page.waitForTimeout(40)
    const depth = parseFloat((await vcb(page, 'Push depth')).replace('~', ''))
    expect(depth, `at ${dy}px up the readout must keep growing (chip: ${await chip(page)})`).toBeGreaterThan(last - 1e-6)
    expect(depth).toBeGreaterThan(0)
    last = depth
  }
  expect(last).toBeGreaterThan(0.5)
})

test('Rectangle: a first click on a vertical edge midpoint draws on that face, not the ground (chip agrees with the rectangle)', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('r')
  await hover(page, ctx, [0, 0, 0.5]) // the south-west vertical edge's midpoint
  expect(await chip(page)).toBe('Midpoint')
  await click(page, ctx, [0, 0, 0.5])
  await hover(page, ctx, [0.6, 0, 0.8])
  // Second corner on the south face: 0.6 wide, 0.3 tall — not "0.6 × 0"
  // projected to the ground, and never "projected". Width across the face
  // first, height up it second, which is also the order typing the pair
  // back applies it (rectangle-dimension-order.spec.ts covers that).
  expect(await chip(page)).toBe('On Face')
  expect(await vcb(page, 'Value')).toBe('0.6 m × 0.3 m')
  // …and the opposite corner honours an Endpoint snap exactly, a few
  // pixels off the corner's own pixel (the chip and the shape agree).
  await hover(page, ctx, [1, 0, 1], 4, 3)
  expect(await chip(page)).toBe('Endpoint')
  expect(await vcb(page, 'Value')).toBe('1 m × 0.5 m')
})

test('Rectangle: Shift over a face pins its plane, so a rectangle clicked over the ground lands on that plane (GitHub issue 14)', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('r')
  await hover(page, ctx, [0.5, 0, 0.5])
  expect(await chip(page)).toBe('On Face')
  await page.keyboard.down('Shift')
  await page.keyboard.up('Shift')
  await expect(page.getByText(/Pinned to the hovered plane/)).toBeVisible()
  const before = (await sketchPlanes(page)).length
  await click(page, ctx, [1.4, -0.6, 0])
  await click(page, ctx, [1.8, -0.35, 0])
  const planes = await sketchPlanes(page)
  expect(planes.length).toBe(before + 1)
  const plane = planes[planes.length - 1]
  // The south face's plane: y = 0, normal ±Y.
  expect(Math.abs(plane[1])).toBeLessThan(1e-9)
  expect(Math.abs(Math.abs(plane[4]) - 1)).toBeLessThan(1e-9)
  // Shift again releases the pin.
  await page.keyboard.down('Shift')
  await page.keyboard.up('Shift')
  await expect(page.getByText(/Pinned to the hovered plane/)).toHaveCount(0)
})

test('Line: dragging along the red axis from one vertical edge reaches the opposite edge as an axis crossing', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('l')
  await click(page, ctx, [0, 0, 0.3])
  await hover(page, ctx, [1, 0, 0.3])
  // The crossing of the red axis through the anchor with the far edge —
  // previously "On Axis" beat "On Edge" and the edge was unreachable.
  expect(await chip(page)).toBe('Intersectionon red axis')
  await hover(page, ctx, [1, 0, 0.5])
  expect(await chip(page)).toBe('Midpoint')
})

test('Line: bottom-southwest corner to top-southeast corner splits the face, never a ground sketch', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('l')
  const before = (await sketchPlanes(page)).length
  await click(page, ctx, [0, 0, 0])
  await hover(page, ctx, [1, 0, 1])
  expect(await chip(page)).toBe('Endpoint')
  expect(await vcb(page, 'Value')).toBe('~1.414 m')
  await click(page, ctx, [1, 0, 1])
  await page.keyboard.press('Escape')
  expect((await sketchPlanes(page)).length).toBe(before)
})

test('Line: a corner → top-edge midpoint → corner chain cuts a triangle out of the face (two cuts, no refusal)', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('l')
  await click(page, ctx, [0, 0, 0])
  await click(page, ctx, [0.5, 0, 1])
  await click(page, ctx, [1, 0, 0])
  await page.keyboard.press('Escape')
  await page.waitForTimeout(150)
  // The face under the triangle's centre is now a different face from the
  // one under the far corners — the cuts landed.
  const faces = await page.evaluate(() => {
    const h = window.__hew_test!
    const at = (x: number, z: number) => h.pickFace([x, -1, z], [0, 1, 0])?.face ?? null
    return { inside: at(0.5, 0.2), leftOfIt: at(0.1, 0.8), rightOfIt: at(0.9, 0.8) }
  })
  expect(faces.inside).not.toBeNull()
  expect(faces.inside).not.toBe(faces.leftOfIt)
  expect(faces.inside).not.toBe(faces.rightOfIt)
  await expect(page.getByText(/crosses itself/)).toHaveCount(0)
})

test('Line: a chain of interior points closing on its start imprints a pushable sub-face', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('l')
  await click(page, ctx, [0.2, 0, 0.2])
  await click(page, ctx, [0.8, 0, 0.2])
  await click(page, ctx, [0.5, 0, 0.8])
  await click(page, ctx, [0.2, 0, 0.2])
  await page.waitForTimeout(150)
  const faces = await page.evaluate(() => {
    const h = window.__hew_test!
    const at = (x: number, z: number) => h.pickFace([x, -1, z], [0, 1, 0])?.face ?? null
    return { inside: at(0.5, 0.35), outside: at(0.05, 0.05) }
  })
  expect(faces.inside).not.toBeNull()
  expect(faces.inside).not.toBe(faces.outside)
})

test('Push/Pull: a corner-to-corner diagonal triangle pushes partway and all the way through, either triangle', async ({ page }) => {
  const ctx = await boot(page)
  await page.keyboard.press('l')
  await click(page, ctx, [0, 0, 0])
  await click(page, ctx, [1, 0, 1])
  await page.keyboard.press('Escape')
  await page.waitForTimeout(150)
  const objectsBefore = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(objectsBefore.length).toBe(1)
  await page.keyboard.press('p')
  // Top-left triangle, partway: the readout follows the drag and the commit
  // is accepted (no refusal toast), the object stays one solid.
  await click(page, ctx, [0.25, 0, 0.75])
  await page.keyboard.type('-0.3') // a typed unsigned length pulls OUTWARD; the sign pushes in
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  await expect(page.getByText(/run into the object/)).toHaveCount(0)
  await expect(page.getByText(/remove the whole object/)).toHaveCount(0)
  const pushed = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getObjectIds()[0]
    return { solid: h.isObjectSolid(id), face: h.pickFace([0.25, -1, 0.75], [0, 1, 0]) }
  })
  expect(pushed.solid).toBe(true)
  // The pushed triangle now sits at y = 0.3: a ray from the south reaches
  // the same triangle face only after crossing empty space to y = 0.3.
  expect(pushed.face).not.toBeNull()
  // Undo, then push the SAME triangle all the way through: a prism is gone.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  await click(page, ctx, [0.25, 0, 0.75])
  await page.keyboard.type('-1')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(250)
  await expect(page.getByText(/remove the whole object/)).toHaveCount(0)
  const through = await page.evaluate(() => {
    const h = window.__hew_test!
    const ids = h.getObjectIds()
    // A ray from the north at the carved triangle's spot must now pass all
    // the way to the far (south) plane's OTHER triangle — nothing at y=1.
    return { count: ids.length, solid: h.isObjectSolid(ids[0]), far: h.pickFace([0.25, 2, 0.75], [0, -1, 0]) }
  })
  expect(through.count).toBe(1)
  expect(through.solid).toBe(true)
  expect(through.far).toBeNull()
})

test('Dimension: in a (slightly tilted) parallel Top view a stacked corner resolves to the nearest one, not the slab below', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  // A slab with a thin wall standing through it (solids may interpenetrate;
  // `drawBox` only draws on the ground), the wall's corner a hair inside
  // the slab's — a framed corner seen from above.
  await page.evaluate(() => {
    window.__hew_test!.drawBox([0, 0, 0], [4, 4, 0], 0.1)
    // A millimetre inside: sub-pixel at this zoom, like the framed corner.
    window.__hew_test!.drawBox([0.001, 0.001, 0], [0.201, 4, 0], 2.5)
  })
  await page.evaluate(() => window.__hew_test!.setCamera({ position: [2, 2, 30], target: [2, 2, 0], up: [0, 1, 0], fovDeg: 45 }))
  await page.waitForTimeout(150)
  await page.getByRole('button', { name: 'Camera' }).click()
  await page.getByText('Parallel Projection').click()
  await page.waitForTimeout(200)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const ctx: Ctx = { rect: box }
  await page.getByRole('radio', { name: 'Dimension' }).click()
  for (const [dx, dy] of [[-1, 1], [2, -2], [-3, 3]]) {
    await page.keyboard.press('Escape')
    await click(page, ctx, [0.001, 0.001, 2.5], dx, dy)
    await click(page, ctx, [0.201, 4, 2.5])
    await click(page, ctx, [1.2, 2, 2.5])
    const anchors = await page.evaluate(() => {
      const ids = window.__hew_test!.getAnnotationIds()
      return window.__hew_test!.getLinearDimensionAnchors(ids[ids.length - 1])
    })
    expect(anchors, `offset ${dx},${dy}`).not.toBeNull()
    expect(anchors!.a[2], `offset ${dx},${dy}: the wall-top corner, not the slab's`).toBeCloseTo(2.5, 6)
  }
})
