import { test, expect, type Page } from '@playwright/test'

/**
 * The inferred drawing plane follows the camera and the idle keys under a
 * STILL cursor (docs/design/view-plane-inference.md §3.5, §5): the plane
 * patch, the status hint and the first click are re-resolved at the held
 * pointer, on a ray rebuilt for the camera as it stands, whenever something
 * other than a pointer move changes the answer — an arrow lock released,
 * a Shift pin released after a zoom, an orbit's coast coming to rest.
 *
 * Front view, Rectangle, over empty space, so the green plane (y = 0, through
 * the drawing-axes origin) is the inferred plane. Real pointer, wheel and
 * key events throughout; the patch is read from the cue layer as drawn.
 *
 * The last test hovers a cube's faces instead: a hovered face draws no
 * patch (the plane is named by the die's pip), but the tool's
 * plane state already holds the face's own plane before any key.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

async function frames(page: Page, n = 2): Promise<void> {
  await page.evaluate(
    (count) =>
      new Promise<void>((resolve) => {
        let left = count
        const step = () => (--left <= 0 ? resolve() : requestAnimationFrame(step))
        requestAnimationFrame(step)
      }),
    n,
  )
}

async function chooseView(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: 'Camera' }).click()
  await page.getByText('Standard Views', { exact: true }).hover()
  await page.getByText(name, { exact: true }).first().click()
  await frames(page)
}

/** Front view about a target behind the green plane, so zooming toward the
 *  cursor (which holds the target-depth plane still) moves the point the
 *  cursor ray meets y = 0. Rectangle active. Returns the canvas box. */
async function setup(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    window.__hew_test!.setCamera({ position: [3, -8, 4], target: [0, 4, 1], up: [0, 0, 1], fovDeg: 45 } as never)
  })
  await chooseView(page, 'Front')
  await page.waitForTimeout(400) // the standard-view tween
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  await page.keyboard.press('r')
  await page.waitForTimeout(100)
  return canvas
}

/** Hover a page point the way a hand arrives (a few moves). */
async function hover(page: Page, x: number, y: number): Promise<void> {
  await page.mouse.move(x - 6, y + 6)
  await page.mouse.move(x, y, { steps: 3 })
  await page.waitForTimeout(80)
}

/** The patch as drawn, and where the current camera's ray at the page
 *  point (`x`, `y`) meets the green plane y = 0. */
async function cueAndExpected(page: Page, canvas: { x: number; y: number }, x: number, y: number) {
  return page.evaluate(
    ({ cx, cy }) => {
      const h = window.__hew_test!
      const ray = h.rayAtCanvasPoint(cx, cy)
      const t = -ray.origin[1] / ray.direction[1]
      const hit: V3 = [ray.origin[0] + ray.direction[0] * t, ray.origin[1] + ray.direction[1] * t, ray.origin[2] + ray.direction[2] * t]
      return { cue: h.getDrawPlaneCue(), hit, t }
    },
    { cx: x - canvas.x, cy: y - canvas.y },
  )
}

function expectGreenPatchAt(got: Awaited<ReturnType<typeof cueAndExpected>>, digits = 4): void {
  expect(got.t).toBeGreaterThan(0) // the plane is in front of the eye
  expect(got.cue).not.toBeNull()
  expect(Math.abs(got.cue!.normal[1])).toBeCloseTo(1, 9)
  for (let i = 0; i < 3; i++) expect(got.cue!.through[i]).toBeCloseTo(got.hit[i], digits)
}

const camera = (page: Page) => page.evaluate(() => window.__hew_test!.getCameraState())

test('an arrow lock released with the cursor still brings the inferred plane back, and the click lands on it', async ({ page }) => {
  const canvas = await setup(page)
  const px = canvas.x + canvas.width * 0.7
  const py = canvas.y + canvas.height * 0.3
  await hover(page, px, py)
  expectGreenPatchAt(await cueAndExpected(page, canvas, px, py))
  await expect(page.getByText(/Facing the green plane/).first()).toBeVisible()

  await page.keyboard.press('ArrowUp') // lock the ground
  await frames(page)
  await expect(page.getByText(/Locked to the blue plane/).first()).toBeVisible()
  await page.keyboard.press('ArrowUp') // release it; the cursor never moves
  await frames(page)
  expectGreenPatchAt(await cueAndExpected(page, canvas, px, py))
  await expect(page.getByText(/Facing the green plane/).first()).toBeVisible()

  // The first corner where the cursor rests, then the opposite one.
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(80)
  await hover(page, px + 80, py + 60)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(80)
  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    return { planes: h.getSketchIds().map((id) => h.getSketchPlane(id)), error: h.getLastError() }
  })
  expect(result.error).toBeNull()
  expect(result.planes).toHaveLength(1)
  expect(Math.abs(result.planes[0]![4])).toBeCloseTo(1, 9) // normal along green
  expect(result.planes[0]![1]).toBeCloseTo(0, 9) // on y = 0
})

test('after a wheel zoom, a Shift pin and release redraw the patch for the zoomed camera at the held pointer', async ({ page }) => {
  const canvas = await setup(page)
  const px = canvas.x + canvas.width * 0.7
  const py = canvas.y + canvas.height * 0.3
  await hover(page, px, py)
  expectGreenPatchAt(await cueAndExpected(page, canvas, px, py))
  const beforeZoom = await camera(page)

  await page.mouse.wheel(0, -300)
  await frames(page)
  const zoomed = await camera(page)
  expect(Math.hypot(zoomed.eye[0] - beforeZoom.eye[0], zoomed.eye[1] - beforeZoom.eye[1], zoomed.eye[2] - beforeZoom.eye[2])).toBeGreaterThan(1e-2)
  // Let the camera's settle replay (250 ms after the last change) run
  // first, so what is checked below is the Shift path's own re-resolve.
  await page.waitForTimeout(400)
  await page.keyboard.press('Shift') // pin
  await page.keyboard.press('Shift') // release
  // The patch is back, drawn where the zoomed camera's ray at the held
  // pointer meets the plane.
  expectGreenPatchAt(await cueAndExpected(page, canvas, px, py))
  await expect(page.getByText(/Facing the green plane/).first()).toBeVisible()
})

test('an orbit coasting to rest under a still cursor ends with the patch for the camera where it stopped', async ({ page }) => {
  const canvas = await setup(page)
  const px = canvas.x + canvas.width * 0.6
  const py = canvas.y + canvas.height * 0.3
  await hover(page, px, py)
  expectGreenPatchAt(await cueAndExpected(page, canvas, px, py))

  // A short level middle-drag orbit, released the moment the hand stops.
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(px + 50, py, { steps: 5 })
  await page.mouse.up({ button: 'middle' })
  const atRelease = await camera(page)
  const hitAtRelease = (await cueAndExpected(page, canvas, px + 50, py)).hit

  // Wait for the coast to end (two reads 100 ms apart agree), then for the
  // settle notification.
  let last = atRelease
  for (let i = 0; i < 40; i++) {
    await page.waitForTimeout(100)
    const now = await camera(page)
    const moved = Math.hypot(now.eye[0] - last.eye[0], now.eye[1] - last.eye[1], now.eye[2] - last.eye[2])
    last = now
    if (moved < 1e-9) break
  }
  await page.waitForTimeout(300)
  // The camera kept moving after the release (inertia), so a cue left from
  // the drag's last re-resolve would be stale.
  expect(Math.hypot(last.eye[0] - atRelease.eye[0], last.eye[1] - atRelease.eye[1], last.eye[2] - atRelease.eye[2])).toBeGreaterThan(1e-3)
  const final = await cueAndExpected(page, canvas, px + 50, py)
  // The coast moved the held pointer's hit on the plane well past any
  // tolerance below…
  expect(Math.hypot(final.hit[0] - hitAtRelease[0], final.hit[2] - hitAtRelease[2])).toBeGreaterThan(0.1)
  // …and the patch follows it to the camera where it stopped. Millimeter
  // tolerance: OrbitControls' last damping steps move the eye by less than
  // its change threshold without a 'change' event, so nothing re-resolves
  // after them.
  expectGreenPatchAt(final, 2)
})

/** The page point of world point `w`. */
async function pageAt(page: Page, canvas: { x: number; y: number }, w: V3): Promise<{ x: number; y: number }> {
  const p = await page.evaluate((world) => window.__hew_test!.worldToScreen(world as V3), w)
  return { x: canvas.x + p.x, y: canvas.y + p.y }
}

test('hovering a face shows its plane before the click: a cube\'s front face, then a raised top face', async ({ page }) => {
  const canvas = await setup(page)
  // A 1 m cube standing on the ground, its front face on y = 0.5 (off the
  // inferred green plane y = 0, so the patch can only be the face's).
  await page.evaluate(() => window.__hew_test!.drawBox([-0.5, 0.5, 0], [0.5, 1.5, 0], 1))
  await frames(page)

  // Front view: the front face's middle. A hovered face is named by the
  // tool's plane (the die's pip and the chip), not drawn as a patch.
  const state = () => page.evaluate(() => window.__hew_test!.getDrawPlaneState())
  const front = await pageAt(page, canvas, [0.1, 0.5, 0.45])
  await hover(page, front.x, front.y)
  let st = await state()
  expect(st).not.toBeNull()
  expect(st!.hover).toBe('face')
  expect(Math.abs(st!.normal[1])).toBeCloseTo(1, 6)
  expect(st!.origin[1]).toBeCloseTo(0.5, 4)
  expect(await page.evaluate(() => window.__hew_test!.getDrawPlaneCue())).toBeNull()
  await expect(page.getByText(/Facing/)).toHaveCount(0)

  // From above at three-quarters: the top face at z = 1.
  await page.evaluate(() => {
    window.__hew_test!.setCamera({ position: [3, -3, 4], target: [0, 1, 0.5], up: [0, 0, 1], fovDeg: 45 } as never)
  })
  await page.waitForTimeout(400) // past the camera-settle replay
  const top = await pageAt(page, canvas, [0.1, 1.05, 1])
  await hover(page, top.x, top.y)
  st = await state()
  expect(st).not.toBeNull()
  expect(st!.hover).toBe('face')
  expect(Math.abs(st!.normal[2])).toBeCloseTo(1, 6)
  expect(st!.origin[2]).toBeCloseTo(1, 4)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
})
