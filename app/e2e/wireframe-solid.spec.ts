import { test, expect, type Page } from '@playwright/test'

/**
 * Faces and solids from lines (docs/design/wireframe-faces-and-solids.md
 * §11, the one Chromium spec) — driven with REAL pointer and key events; the
 * harness only pins the camera and reads kernel state back.
 *
 * 1. Kurt's scenario, from an empty document: a ground Rectangle, a vertical
 *    Line up from one corner, and Lines from its top to the three other
 *    corners. No solid exists until the last Line lands; that Line fills the
 *    last faces, the faces close into a solid, and the new Object is
 *    selected with a toast.
 * 2. A vertical Rectangle started on the ground Rectangle's edge snaps to the
 *    point straight above the edge's far corner ("From Point"), so its width
 *    is the edge's length to the bit.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pixel-accurate picking is only stable on pinned SwiftShader')

type V3 = [number, number, number]

// From the back-left, so no closed wall hides the corner the next Line aims
// at (a corner behind an opaque face is not offered as a snap in Shaded).
const CAMERA: V3 = [-1.4, 2.7, 2.0]
const TARGET: V3 = [0.5, 0.5, 0.4]

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(
    ({ position, target }) => {
      const t = window.__hew_test!
      t.setGridVisible(false)
      t.setAxesVisible(false)
      t.setCamera({ position, target, up: [0, 0, 1], fovDeg: 45 })
    },
    { position: CAMERA, target: TARGET },
  )
  await page.waitForTimeout(250)
})

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

async function clickAt(page: Page, w: V3): Promise<void> {
  await hover(page, w)
  await press(page)
}

/** Every drawn edge, as exact endpoint pairs, over all live sketches. */
function allSegments(page: Page): Promise<number[][]> {
  return page.evaluate(() => {
    const t = window.__hew_test!
    return t.getSketchIds().flatMap((id) => t.getSketchSegmentsExact(id))
  })
}

/** The ground Rectangle's corners A (min, min), B (max, min), C (max, max), D (min, max). */
async function groundCorners(page: Page): Promise<{ a: V3; b: V3; c: V3; d: V3 }> {
  const flat = (await allSegments(page)).flatMap((s) => [s.slice(0, 3), s.slice(3, 6)]).filter((p) => Math.abs(p[2]) < 1e-9)
  expect(flat.length, 'the ground Rectangle has four edges').toBeGreaterThanOrEqual(8)
  const xs = flat.map((p) => p[0])
  const ys = flat.map((p) => p[1])
  const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)]
  return { a: [x0, y0, 0], b: [x1, y0, 0], c: [x1, y1, 0], d: [x0, y1, 0] }
}

test("Kurt's pyramid: a Rectangle and four Lines become one selected solid Object", async ({ page }) => {
  test.setTimeout(90_000)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)

  // A 1 m Rectangle on the ground: two clicks.
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await clickAt(page, [0, 0, 0])
  await clickAt(page, [1, 1, 0])
  const { a, b, c, d } = await groundCorners(page)
  expect(b[0] - a[0]).toBeGreaterThan(0.9)
  expect(d[1] - a[1]).toBeGreaterThan(0.9)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount()), 'a flat Rectangle is a face, not an Object').toBe(0)

  // A vertical Line, 1 m straight up from corner A: lock Blue, hover up the
  // axis, type the length.
  await page.getByRole('radio', { name: 'Line' }).click()
  await clickAt(page, a)
  await page.keyboard.press('ArrowUp')
  await hover(page, [a[0], a[1], 0.5])
  await page.keyboard.type('1m')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(100)
  await page.keyboard.press('Escape')
  const apex: V3 = [a[0], a[1], 1]

  // Lines from the apex to B, C and D, each its own gesture.
  // A Line that closes a loop ends its own chain, so the last one needs no
  // Esc (and a stray one would deselect the new Object).
  const toCorner = async (corner: V3, endChain = true) => {
    await clickAt(page, apex)
    await clickAt(page, corner)
    if (endChain) await page.keyboard.press('Escape')
    await page.waitForTimeout(100)
  }
  await toCorner(b)
  await toCorner(c)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount()), 'two walls are not yet a closed shape').toBe(0)

  // The last Line fills the last two walls; the closed faces become a solid.
  await toCorner(d, false)
  await expect(page.getByText('The closed shape is now a solid.')).toBeVisible()

  const result = await page.evaluate(() => {
    const t = window.__hew_test!
    const selection = t.getSelection()
    const id = selection[0]?.id
    return {
      error: t.getLastError(),
      objects: t.getObjectIds(),
      selection,
      solid: id === undefined ? null : t.isObjectSolid(id),
      bounds: id === undefined ? null : t.getObjectBounds(id),
    }
  })
  expect(result.error).toBeNull()
  expect(result.objects).toHaveLength(1)
  expect(result.selection).toHaveLength(1)
  expect(result.selection[0].kind).toBe('object')
  expect(result.selection[0].id).toBe(result.objects[0])
  expect(result.solid).toBe(true)
  // Object bounds are single-precision.
  const [x0, y0, z0, x1, y1, z1] = result.bounds!
  expect(x0).toBeCloseTo(a[0], 5)
  expect(y0).toBeCloseTo(a[1], 5)
  expect(z0).toBeCloseTo(0, 5)
  expect(x1).toBeCloseTo(c[0], 5)
  expect(y1).toBeCloseTo(c[1], 5)
  expect(z1).toBeCloseTo(1, 5)
})

test('a vertical Rectangle snaps above the ground Rectangle\'s far corner: its width is |AB|', async ({ page }) => {
  test.setTimeout(60_000)
  await page.evaluate(() => window.__hew_test!.drawRectangle([0, 0, 0], [1, 1, 0]))
  const a: V3 = [0, 0, 0]
  const b: V3 = [1, 0, 0]

  await page.getByRole('radio', { name: 'Rectangle' }).click()
  // Left arrow: the plane through the next click, facing along the green axis.
  await page.keyboard.press('ArrowLeft')
  await clickAt(page, a)

  // Straight to a hair beside the vertical through B, 0.7 m up, with no
  // rest on B: the first click's plain corner names its Rectangle through the
  // vertex under the click (ruling R15), so B is a corner of the start shape
  // and the from-point snap pulls the cursor onto its vertical.
  await hover(page, [b[0] + 0.003, b[1], 0.7])
  await expect(page.getByText('From Point', { exact: false })).toBeVisible()
  await press(page)

  const segments = await allSegments(page)
  const raised = segments.filter((s) => s[2] > 1e-9 || s[5] > 1e-9)
  expect(raised.length, 'the vertical Rectangle has edges above the ground').toBeGreaterThan(0)
  const xs = raised.flatMap((s) => [s[0], s[3]])
  expect(Math.max(...xs) - Math.min(...xs)).toBe(b[0] - a[0])
  // It stands on the plane through A and B.
  expect(raised.every((s) => s[1] === a[1] && s[4] === a[1])).toBe(true)
})
