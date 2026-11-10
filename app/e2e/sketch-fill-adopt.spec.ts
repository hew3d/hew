import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * A draw tool hovering INSIDE a sketch's filled region, or on one of its
 * edges, adopts that sketch's plane (docs/design/view-plane-inference.md §2
 * step 4): the idle patch shows that plane before any key (§5), the chip
 * reads "On Face" and no "Facing" line shows, Shift pins it, and a click
 * draws into it. Before, adoption needed the cursor within the edge pick's
 * aperture of a segment, so over the interior the hover fell through to the
 * ground or the plane the view faces, and Shift pinned that instead; and
 * the patch showed only for an inferred plane, so moving onto a sketch
 * blinked it off. A ground sketch keeps the ground from any camera, even
 * one low enough that empty space infers a wall, and shows no patch.
 *
 * Fixture `sketch-planes.hew`: three sketches through the origin. Ground
 * (normal +Z), a rectangle x 0.1607..0.7797, y 0.1848..0.7678. Green
 * (normal +Y, y = 0), a rectangle x −0.8645..−0.2025, z 0.1378..0.4838 plus
 * two circles, 3 regions. Red (normal +X, x = 0), a rectangle
 * y 0.4172..1.4532, z 0.2304..0.5124, 1 region. Real pointer and key
 * events throughout.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pointer-driven spec, chromium only')

type V3 = [number, number, number]

const FIXTURE = resolve(process.cwd(), 'e2e/fixtures/sketch-planes.hew')

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((arr) => window.__hew_test!.load(arr), Array.from(readFileSync(FIXTURE)))
  await page.waitForTimeout(200)
}

async function aim(page: Page, position: V3, target: V3): Promise<void> {
  await page.evaluate(
    (cam) => window.__hew_test!.setCamera({ ...cam, up: [0, 0, 1], fovDeg: 45 }),
    { position, target },
  )
  await page.waitForTimeout(400) // past the camera-settle replay
}

async function toPage(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const p = await page.evaluate((world) => window.__hew_test!.worldToScreen(world as V3), w)
  return { x: canvas.x + p.x, y: canvas.y + p.y }
}

/** Hover `w` the way a hand arrives (a few moves, so the snap resolves). */
async function hover(page: Page, w: V3): Promise<void> {
  const p = await toPage(page, w)
  await page.mouse.move(p.x + 5, p.y + 5)
  await page.mouse.move(p.x, p.y, { steps: 3 })
  await page.waitForTimeout(100)
}

async function click(page: Page, w: V3): Promise<void> {
  await hover(page, w)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(100)
}

/** The snap chip's text. */
async function chip(page: Page): Promise<string> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll('div'))
      .filter((d) => d.style.zIndex === '20' && d.style.pointerEvents === 'none')
      .map((e) => e.textContent ?? '')
      .join(' | '),
  )
}

/** Every sketch with its plane and region count. */
async function sketches(page: Page) {
  return page.evaluate(() => {
    const h = window.__hew_test!
    return h.getSketchIds().map((id) => ({ id, plane: h.getSketchPlane(id)!, regions: h.getSketchRegionCount(id) }))
  })
}

/** The sketch whose plane normal lies along `axis` (0 x, 1 y, 2 z). */
function along(list: Awaited<ReturnType<typeof sketches>>, axis: number) {
  const s = list.find((k) => Math.abs(k.plane[3 + axis]) > 0.999)
  if (s === undefined) throw new Error(`no sketch with its normal along axis ${axis}`)
  return s
}

const cue = (page: Page) => page.evaluate(() => window.__hew_test!.getDrawPlaneCue())

function expectNormalAlong(c: { normal: V3 } | null, axis: number): void {
  expect(c).not.toBeNull()
  expect(Math.abs(c!.normal[axis])).toBeCloseTo(1, 6)
}

/** The status hint row (the idle line, a Facing/Pinned line). */
const hint = (page: Page, re: RegExp) => page.getByText(re).first()

const GREEN_CENTER: V3 = [-0.5335, 0, 0.3108]
const GREEN_TOP_EDGE: V3 = [-0.5, 0, 0.4838]
const RED_CENTER: V3 = [0, 0.935, 0.371]
const GROUND_FILL: V3 = [0.4, 0.4, 0]

test('a hover inside a sketch fill adopts its plane: the patch, the Shift pin and the click all land on the sketch', async ({ page }) => {
  await setup(page)
  const before = await sketches(page)
  expect(before).toHaveLength(3)
  expect(along(before, 1).regions).toBe(3)
  expect(along(before, 0).regions).toBe(1)
  expect(along(before, 2).regions).toBe(1)

  // (1) A three-quarter view; inside the standing green rectangle.
  await aim(page, [2.5, -3, 2.5], [0, 0, 0.3])
  await page.keyboard.press('r')
  await page.waitForTimeout(100)
  // On the green rectangle's top edge: the edge pick adopts its plane.
  await hover(page, GREEN_TOP_EDGE)
  expectNormalAlong(await cue(page), 1)
  await expect(hint(page, /Facing/)).toHaveCount(0)
  // Resting on the edge's named point (a dwell) put it in the reference ring,
  // and its blue vertical line passes within the snap radius of the fill's
  // centre: by design (wireframe-faces-and-solids.md §3.2 source 3, §3.3) that
  // From Point snap replaces the weak On Face one in the chip. Esc from idle
  // clears the ring (§3.2), so the chip reports the adopted face again.
  await page.keyboard.press('Escape')
  // Inside it: the fill adopts the same plane, patch up before any key.
  await hover(page, GREEN_CENTER)
  expectNormalAlong(await cue(page), 1)
  expect(await chip(page)).toContain('On Face')
  await expect(hint(page, /Facing/)).toHaveCount(0)

  // Shift pins the green sketch's plane (not the ground): its patch shows.
  await page.keyboard.press('Shift')
  await expect(hint(page, /Pinned/)).toBeVisible()
  await hover(page, [-0.55, 0, 0.3])
  expectNormalAlong(await cue(page), 1)
  expect(await chip(page)).toContain('On Face')
  await click(page, [-0.7, 0, 0.22])
  await click(page, [-0.4, 0, 0.4])
  let now = await sketches(page)
  expect(now).toHaveLength(3)
  expect(along(now, 1).regions).toBe(4)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  await page.keyboard.press('Shift') // unpin
  await expect(hint(page, /Pinned/)).toHaveCount(0)
  await page.keyboard.press('Escape') // close the retype window, so the idle hints show

  // (2) From the +X side, inside the red rectangle.
  await aim(page, [3.5, 0.9, 2.0], [0, 0.935, 0.37])
  await page.keyboard.press('Escape') // clear the dwell ring (§3.2), as above
  await hover(page, RED_CENTER)
  expectNormalAlong(await cue(page), 0)
  expect(await chip(page)).toContain('On Face')
  await expect(hint(page, /Facing/)).toHaveCount(0)
  await page.keyboard.press('Shift') // pin: the red sketch's plane
  await hover(page, [0, 0.9, 0.35])
  expectNormalAlong(await cue(page), 0)
  await page.keyboard.press('Shift') // unpin; the click below is a plain one
  await expect(hint(page, /Pinned/)).toHaveCount(0)
  await click(page, [0, 0.7, 0.3])
  await click(page, [0, 1.1, 0.45])
  now = await sketches(page)
  expect(now).toHaveLength(3)
  expect(along(now, 0).regions).toBe(2)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  await page.keyboard.press('Escape') // close the retype window

  // (3) A low camera (about 14°) facing the green plane.
  // Close in (the same angle at half the distance). A named point under the
  // cursor (a corner or midpoint of the ground shape) starts the plane the
  // view faces THROUGH it by design (R18), so the fill is hovered where
  // none is near.
  await aim(page, [0.47, -3, 0.8], [0.47, 0.476, 0])
  // Empty ground beside the ground rectangle: the green plane the view faces.
  await hover(page, [1.3, 0.476, 0])
  expectNormalAlong(await cue(page), 1)
  await expect(hint(page, /Facing the green plane/)).toBeVisible()
  // Inside the ground rectangle, on its fill and clear of its edges and
  // their named points: the ground, no patch, no Facing line.
  await hover(page, GROUND_FILL)
  expect(await cue(page)).toBeNull()
  await expect(hint(page, /Facing/)).toHaveCount(0)

  await page.keyboard.press('Shift')
  await expect(hint(page, /Pinned/)).toBeVisible()
  await click(page, [0.3, 0.35, 0])
  await click(page, [0.65, 0.65, 0])
  now = await sketches(page)
  expect(now).toHaveLength(3)
  const ground = along(now, 2)
  // The rectangle split the ground one. At this grazing angle the whole
  // depth of the ground rectangle is a few snap radii on screen, so the
  // corners may snap onto its near and far edges (a full-depth strip, three
  // regions) — either way the new shape lies in the ground sketch.
  expect(ground.regions).toBeGreaterThanOrEqual(2)
  const lines = await page.evaluate((id) => window.__hew_test!.getSketchLines(id), ground.id)
  for (let i = 2; i < lines.length; i += 3) expect(Math.abs(lines[i])).toBeLessThan(1e-9)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
})
