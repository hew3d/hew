import { test, expect, type Page } from '@playwright/test'
import { buildViewProjection, worldToPagePixel, type CameraParams, type Mat4 } from './helpers/projectWorldToScreen'

/**
 * Place Image, end to end with real pointer input: a picture dropped on the
 * viewport arms the placement; a real click and a second real click place
 * it on the ground with its proportions kept; the Tape Measure resizes it
 * inside its opened component and the picture stays stretched over the
 * face (one picture, not tiles); a save and reopen keeps it.
 *
 * The picture is 4 x 2 pixels with four colored columns (red, green, blue,
 * yellow, left to right), so where each color shows on screen says how the
 * texture lies on the face. The drop itself is a DOM drop event carrying a
 * real File (a browser test cannot perform an OS drag); every placement and
 * measuring step is real mouse and keyboard input.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 4, y: -3, z: 14 },
  target: { x: 4, y: 2, z: 0 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
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

async function aim(page: Page): Promise<Ctx> {
  await page.evaluate(
    (cam) =>
      window.__hew_test!.setCamera({
        position: [cam.position.x, cam.position.y, cam.position.z],
        target: [cam.target.x, cam.target.y, cam.target.z],
        up: [cam.up.x, cam.up.y, cam.up.z],
        fovDeg: cam.fovDeg,
      }),
    CAMERA,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(CAMERA, rect.width / rect.height), rect }
}

async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 4, p.y - 4)
  await page.waitForTimeout(40)
  await page.mouse.move(p.x, p.y, { steps: 4 })
  await page.waitForTimeout(120)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

type Hue = 'red' | 'green' | 'blue' | 'yellow' | 'other'

/** The dominant hue of the rendered pixel at a world point. */
async function hueAt(page: Page, x: number, y: number, z = 0): Promise<Hue> {
  const c = await page.evaluate((w) => window.__hew_test!.pixelColorAt(w), [x, y, z] as [number, number, number])
  if (c === null) return 'other'
  const { r, g, b } = c
  const hi = Math.max(r, g, b)
  if (hi < 40) return 'other'
  const strong = (v: number) => v > hi * 0.6
  if (strong(r) && strong(g) && !strong(b)) return 'yellow'
  if (strong(r) && !strong(g) && !strong(b)) return 'red'
  if (strong(g) && !strong(r) && !strong(b)) return 'green'
  if (strong(b) && !strong(r) && !strong(g)) return 'blue'
  return 'other'
}

/** The four columns' hues along the picture's middle row, for a picture
 *  `width` wide from x = 0 (height width / 2, from y = 0). */
async function columns(page: Page, width: number): Promise<Hue[]> {
  const out: Hue[] = []
  for (const f of [0.125, 0.375, 0.625, 0.875]) out.push(await hueAt(page, width * f, width / 4))
  return out
}

/** The names of every placed component's definition, one per instance. */
function pictures(): string[] {
  const h = window.__hew_test!
  return h.getComponentIds().flatMap((c) => h.getInstancesOf(c).map(() => h.getComponentName(c) ?? ''))
}

const COLUMNS: Hue[] = ['red', 'green', 'blue', 'yellow']

test('Place Image: drop a picture, place it with two clicks, resize it with the Tape Measure, save and reopen', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const ctx = await aim(page)

  // ---- Drop a 4 x 2 PNG (four colored columns) on the viewport.
  const drop = px(ctx, 1, 1, 0)
  await page.evaluate(async (at) => {
    const c = document.createElement('canvas')
    c.width = 4
    c.height = 2
    const g = c.getContext('2d')!
    ;['#ff0000', '#00ff00', '#0000ff', '#ffff00'].forEach((color, i) => {
      g.fillStyle = color
      g.fillRect(i, 0, 1, 2)
    })
    const blob: Blob = await new Promise((res) => c.toBlob((b) => res(b!), 'image/png'))
    const file = new File([blob], 'Columns.png', { type: 'image/png' })
    const dt = new DataTransfer()
    dt.items.add(file)
    const target = document.querySelector('canvas') as HTMLCanvasElement
    target.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, clientX: at.x, clientY: at.y, bubbles: true, cancelable: true }))
  }, drop)
  await expect(page.locator('text=Click the first corner of "Columns"').first()).toBeVisible({ timeout: 5000 })
  // Armed, not placed.
  expect(await page.evaluate(pictures)).toEqual([])

  // ---- Real clicks: first corner at the origin, second 4 m along +X (and
  // a little up: the height follows from the picture, not the cursor).
  await clickWorld(page, ctx, 0, 0, 0)
  await expect(page.locator('text=Click the opposite corner').first()).toBeVisible()
  await clickWorld(page, ctx, 4, 0.7, 0)

  // One picture, a component named after the file, on the ground where
  // the clicks put it.
  expect(await page.evaluate(pictures)).toEqual(['Columns'])
  const hit = await page.evaluate(() => window.__hew_test!.pickFace([1, 0.5, 5], [0, 0, -1]))
  expect(hit?.instance).not.toBeNull()
  // One undo step takes the whole placement away (material, definition,
  // instance), and redo puts it back.
  await page.keyboard.press('ControlOrMeta+z')
  await expect.poll(() => page.evaluate(pictures)).toEqual([])
  await page.keyboard.press('ControlOrMeta+Shift+z')
  await expect.poll(() => page.evaluate(pictures)).toEqual(['Columns'])
  // Back to Select with the picture selected.
  await expect(page.locator('text=Click the first corner of "Columns"')).toHaveCount(0)

  // The placed width follows the second click along +X (about 4 m).
  await expect.poll(() => columns(page, 4), { timeout: 10_000 }).toEqual(COLUMNS)
  if (process.env.HEW_PLACE_IMAGE_SHOTS) await page.screenshot({ path: `${process.env.HEW_PLACE_IMAGE_SHOTS}/placed.png` })

  // ---- Resize with the Tape Measure inside the opened picture: measure
  // its bottom edge (a real 4 m, give or take the click) and type 8.
  await page.mouse.click(px(ctx, 2, 1, 0).x, px(ctx, 2, 1, 0).y, { clickCount: 2 })
  await page.waitForTimeout(250)
  await page.keyboard.press('t')
  await page.locator('text=Click a point to measure from').first().waitFor({ timeout: 5000 })
  const a = px(ctx, 0, 0, 0)
  await page.mouse.move(a.x - 3, a.y - 3)
  await page.mouse.move(a.x, a.y, { steps: 3 })
  await page.waitForTimeout(120)
  await page.mouse.down()
  await page.mouse.up()
  const bottomRight = await page.evaluate(() => {
    const h = window.__hew_test!
    // The face's own corner: pick down onto the picture, read its bounds.
    const pick = h.pickFace([3.9, 0.1, 5], [0, 0, -1])
    return pick === null ? null : h.getObjectBounds(pick.object)
  })
  expect(bottomRight).not.toBeNull()
  const right = (bottomRight as number[])[3]
  const b = px(ctx, right, 0, 0)
  await page.mouse.move(b.x, b.y, { steps: 8 })
  await page.waitForTimeout(150)
  await page.keyboard.type('8')
  await page.keyboard.press('Enter')
  const dialog = page.getByRole('dialog', { name: /^Resize / })
  await dialog.waitFor({ timeout: 5000 })
  await expect(page.getByRole('dialog', { name: 'Resize the model' })).toHaveCount(0)
  await page.getByRole('button', { name: 'Resize' }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await page.keyboard.press('Escape') // Tape Measure back to idle
  await page.keyboard.press('Space') // Select
  await page.keyboard.press('Escape') // close the picture
  await page.waitForTimeout(250)

  // Twice the size, and still ONE picture stretched over the face: each
  // color fills a quarter of the 8 m width (tiles would repeat the colors).
  // The face now reaches past the old 4 m width.
  expect(await page.evaluate(() => window.__hew_test!.pickFace([7, 3, 5], [0, 0, -1])?.instance ?? null)).not.toBeNull()
  await expect.poll(() => columns(page, 8), { timeout: 10_000 }).toEqual(COLUMNS)
  if (process.env.HEW_PLACE_IMAGE_SHOTS) await page.screenshot({ path: `${process.env.HEW_PLACE_IMAGE_SHOTS}/after-resize.png` })

  // ---- Save and reopen: the picture is still there, still stretched.
  const ok = await page.evaluate(() => {
    const h = window.__hew_test!
    const bytes = h.save()
    h.load(bytes)
    return true
  })
  expect(ok).toBe(true)
  await page.waitForTimeout(250)
  expect(await page.evaluate(pictures)).toEqual(['Columns'])
  await aim(page)
  await expect.poll(() => columns(page, 8), { timeout: 10_000 }).toEqual(COLUMNS)
  if (process.env.HEW_PLACE_IMAGE_SHOTS) await page.screenshot({ path: `${process.env.HEW_PLACE_IMAGE_SHOTS}/reopened.png` })
})
