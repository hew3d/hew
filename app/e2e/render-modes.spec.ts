/**
 * View > X-ray and View > Wireframe: one render mode, so the two are
 * mutually exclusive, and picking follows it (SketchUp's rule).
 *
 * A block with a mortise cut up into it from below: the mortise is hidden
 * from the camera except in X-ray and Wireframe. Through the real View menu
 * and real pointer events:
 *
 * - Shaded: the mortise's edges are behind the block's front face, so a
 *   hover there lands on the face, not the hidden midpoint.
 * - X-ray: nothing occludes, so the hidden midpoint snaps; faces are still
 *   targets, so the top face snaps On Face and a click there selects the
 *   block.
 * - Wireframe: nothing occludes and faces do not exist for input: the
 *   hidden midpoint snaps, the top face does not, a click on bare face
 *   selects nothing, and a click on a hidden edge selects the block.
 */
import { test, expect, type Page } from '@playwright/test'
import { mkdirSync } from 'node:fs'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pointer-driven spec, chromium only')

// Frames land in Playwright's per-test output directory; HEW_SHOT_DIR
// overrides it to collect them somewhere else.
let OUT = ''

type Vec3 = [number, number, number]

/** Midpoint of the mortise's front-top edge, inside the block. */
const HIDDEN_MID: Vec3 = [1.0, 0.3, 0.6]
/** A point on the hidden front-top edge away from its midpoint. */
const HIDDEN_EDGE: Vec3 = [0.85, 0.3, 0.6]
/** Midpoint of the mortise's back-top edge, seen through the block's TOP
 *  face from the camera. */
const HIDDEN_MID_TOP: Vec3 = [1.0, 0.7, 0.6]
/** The mortise ceiling's front-right corner, seen through the front face. */
const HIDDEN_CORNER: Vec3 = [1.3, 0.3, 0.6]
/** A point on the block's top face, clear of every edge and of the
 *  mortise below it along the view ray. */
const TOP_FACE: Vec3 = [1.6, 0.65, 1.0]

async function openView(page: Page): Promise<void> {
  if (await page.getByText('Wireframe').isVisible()) return
  await page.getByRole('button', { name: /^view$/i }).click()
  await expect(page.getByText('Wireframe')).toBeVisible()
}

async function choose(page: Page, name: 'X-ray' | 'Wireframe'): Promise<void> {
  await openView(page)
  await page.getByText(name).click()
  await page.waitForTimeout(250)
}

async function viewMenuChecks(page: Page): Promise<[boolean, boolean]> {
  await openView(page)
  const check = async (name: string): Promise<boolean> =>
    (await page.getByText(name).evaluate((e) => e.closest('div')?.textContent ?? '')).includes('✓')
  const r: [boolean, boolean] = [await check('X-ray'), await check('Wireframe')]
  await page.keyboard.press('Escape')
  if (await page.getByText('Wireframe').isVisible()) await page.getByRole('button', { name: /^view$/i }).click()
  await expect(page.getByText('Wireframe')).toBeHidden()
  return r
}

async function shot(page: Page, name: string): Promise<Buffer> {
  await page.mouse.move(5, 5)
  await page.waitForTimeout(200)
  // The model only: the floating tool dock over the canvas's bottom edge
  // changes with the active tool, which is not what is compared.
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  return page.screenshot({ path: `${OUT}/${name}.png`, clip: { x: box.x, y: box.y, width: box.width, height: box.height * 0.8 } })
}

async function px(page: Page, w: Vec3): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function hover(page: Page, w: Vec3): Promise<void> {
  const p = await px(page, w)
  await page.mouse.move(p.x - 6, p.y - 4)
  await page.waitForTimeout(50)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(160)
}

/** The inference chip's text (the cue label beside the cursor). */
async function chip(page: Page): Promise<string> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll('div'))
      .filter((d) => d.style.zIndex === '20' && d.style.pointerEvents === 'none')
      .map((e) => e.textContent ?? '')
      .join(' | '),
  )
}

/** Hover `w` with `tool` (Tape Measure's idle hover is a free snap; the
 *  draw tools hold an idle hover to the face under the cursor) and read
 *  the cue. */
async function hoverChip(page: Page, w: Vec3, tool: 'Tape Measure' | 'Line' = 'Tape Measure'): Promise<string> {
  await page.getByRole('radio', { name: tool }).click()
  await hover(page, w)
  const text = await chip(page)
  await page.keyboard.press('Escape')
  return text
}

/** Click `w` with the Select tool from an empty selection; returns what got
 *  selected. */
async function clickSelect(page: Page, w: Vec3): Promise<{ kind: string; id: string }[]> {
  await page.getByRole('radio', { name: 'Select' }).click()
  await page.evaluate(() => window.__hew_test!.selectObjects([]))
  await hover(page, w)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  return page.evaluate(() => window.__hew_test!.getSelection().map((n) => ({ kind: n.kind, id: n.id })))
}

test('X-ray and Wireframe are one mode, and picking follows it', async ({ page }, testInfo) => {
  test.setTimeout(120_000)
  OUT = process.env.HEW_SHOT_DIR ?? testInfo.outputPath('frames')
  mkdirSync(OUT, { recursive: true })
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const block = await page.evaluate(() => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
    const block = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const mortise = h.drawBox([0.7, 0.3, 0], [1.3, 0.7, 0], 0.6)
    const cut = h.boolean(1, block, mortise)
    h.setCamera({ position: [3.2, -3.6, 4.2], target: [1, 0.5, 0.5], up: [0, 0, 1], fovDeg: 40 })
    return cut
  })
  await page.waitForTimeout(300)
  const isBlock = (sel: { kind: string; id: string }[]) =>
    sel.length === 1 && sel[0].kind === 'object' && sel[0].id === block

  // --- Shaded: the mortise is behind the front face.
  const shaded = await shot(page, '1-shaded')
  expect(await viewMenuChecks(page)).toEqual([false, false])
  const shadedHidden = await hoverChip(page, HIDDEN_MID)
  expect(shadedHidden).toContain('On Face')
  expect(shadedHidden).not.toContain('Midpoint')
  // Line holds its idle hover to the top face: the midpoint under it is out.
  expect(await hoverChip(page, HIDDEN_MID_TOP, 'Line')).not.toContain('Midpoint')
  expect(await hoverChip(page, TOP_FACE)).toContain('On Face')
  expect(await clickSelect(page, TOP_FACE)).toEqual([{ kind: 'object', id: block }])

  // --- X-ray: nothing occludes; faces stay targets.
  await choose(page, 'X-ray')
  const xray = await shot(page, '2-xray')
  expect(await viewMenuChecks(page)).toEqual([true, false])
  expect(xray.equals(shaded)).toBe(false)
  expect(await hoverChip(page, HIDDEN_MID)).toContain('Midpoint')
  expect(await hoverChip(page, TOP_FACE)).toContain('On Face')
  expect(isBlock(await clickSelect(page, TOP_FACE)), 'a face is still a click target').toBe(true)
  expect(isBlock(await clickSelect(page, HIDDEN_EDGE)), 'a hidden edge selects its Object').toBe(true)
  const xrayPick = await page.evaluate(() => window.__hew_test!.pickFace([1.6, 0.65, 5], [0, 0, -1]))
  expect(xrayPick?.object, 'the near (top) face is the pick').toBe(block)


  // --- Choosing Wireframe turns X-ray off: one mode.
  await choose(page, 'Wireframe')
  const wire = await shot(page, '3-wireframe')
  expect(await viewMenuChecks(page)).toEqual([false, true])
  expect(wire.equals(shaded)).toBe(false)
  expect(wire.equals(xray)).toBe(false)
  expect(await hoverChip(page, HIDDEN_MID)).toContain('Midpoint')
  expect(await hoverChip(page, TOP_FACE), 'faces do not exist for input').not.toContain('On Face')
  expect(await clickSelect(page, TOP_FACE), 'a click on bare face selects nothing').toEqual([])
  expect(isBlock(await clickSelect(page, HIDDEN_EDGE)), 'an Object is picked through its edges').toBe(true)
  expect(await page.evaluate(() => window.__hew_test!.pickFace([1.6, 0.65, 5], [0, 0, -1]))).toBeNull()

  // --- Choosing X-ray from Wireframe turns Wireframe off.
  await choose(page, 'X-ray')
  expect(await viewMenuChecks(page)).toEqual([true, false])
  expect((await shot(page, '4-xray-again')).equals(xray)).toBe(true)

  // --- Choosing the active mode again returns to Shaded, picking included.
  await choose(page, 'X-ray')
  expect(await viewMenuChecks(page)).toEqual([false, false])
  await page.getByRole('radio', { name: 'Select' }).click()
  await page.evaluate(() => window.__hew_test!.selectObjects([]))
  expect((await shot(page, '5-shaded-again')).equals(shaded)).toBe(true)
  expect(await hoverChip(page, HIDDEN_MID)).not.toContain('Midpoint')

  // --- Drawing in X-ray (last: it changes the model the frames compare).
  await choose(page, 'X-ray')
  expect(await viewMenuChecks(page)).toEqual([true, false])
  // Line reaches through the face it is held to: the hidden midpoint under
  // the top face snaps, and a line drawn from it to the hidden ceiling
  // corner lands exactly on both.
  expect(await hoverChip(page, HIDDEN_MID_TOP, 'Line')).toContain('Midpoint')
  const before = await page.evaluate(() => window.__hew_test!.getStateHash())
  await page.getByRole('radio', { name: 'Line' }).click()
  await hover(page, HIDDEN_MID_TOP)
  expect(await chip(page)).toContain('Midpoint')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  await hover(page, HIDDEN_CORNER)
  expect(await chip(page)).toContain('Endpoint')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  await page.keyboard.press('Escape')
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getStateHash())).not.toBe(before)
  const drawn = await page.evaluate(() => {
    const t = window.__hew_test!
    const sketched = t.getSketchIds().flatMap((id) => t.getSketchSegmentsExact(id))
    const imprinted = t.getObjectIds().flatMap((o) => t.getImprints(o).map((i) => i.loop.flat()))
    return [...sketched, ...imprinted]
  })
  const near = (p: number[], i: number, q: Vec3) => Math.hypot(p[i] - q[0], p[i + 1] - q[1], p[i + 2] - q[2]) < 1e-9
  const hasLine = drawn.some((seg) => {
    for (let i = 0; i + 2 < seg.length; i += 3) {
      for (let j = 0; j + 2 < seg.length; j += 3) {
        if (i !== j && near(seg, i, HIDDEN_MID_TOP) && near(seg, j, HIDDEN_CORNER)) return true
      }
    }
    return false
  })
  expect(hasLine, 'the line runs from the hidden midpoint to the hidden corner').toBe(true)

  // A shape held to the top face meets the hidden midpoint too, and says
  // its corner drops onto the face ("projected") rather than claiming it.
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await hover(page, TOP_FACE)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  await hover(page, HIDDEN_MID_TOP)
  const rectChip = await chip(page)
  // (The line drawn above starts there, so the point is an endpoint now.)
  expect(rectChip).toMatch(/Endpoint|Midpoint/)
  expect(rectChip).toContain('projected')
  await page.keyboard.press('Escape')
})

test('the orbit pivot lands on the model in Wireframe: it is a camera question, not an input target', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const eye: Vec3 = [3.2, -3.6, 4.2]
  await page.evaluate((eye) => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [2, 1, 0], 1)
    h.setCamera({ position: eye, target: [1, 0.5, 0.5], up: [0, 0, 1], fovDeg: 40 })
  }, eye)
  await page.waitForTimeout(300)
  await choose(page, 'Wireframe')
  expect(await page.evaluate(() => window.__hew_test!.pickFace([1.6, 0.65, 5], [0, 0, -1])), 'no face input').toBeNull()
  // A middle-button press re-targets the orbit to the depth under the cursor.
  const p = await px(page, TOP_FACE)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down({ button: 'middle' })
  await page.mouse.up({ button: 'middle' })
  await page.waitForTimeout(150)
  const cam = await page.evaluate(() => window.__hew_test!.getCamera())
  const fwd = [1 - eye[0], 0.5 - eye[1], 0.5 - eye[2]]
  const n = Math.hypot(fwd[0], fwd[1], fwd[2])
  const axial = ((TOP_FACE[0] - eye[0]) * fwd[0] + (TOP_FACE[1] - eye[1]) * fwd[1] + (TOP_FACE[2] - eye[2]) * fwd[2]) / n
  const got = Math.hypot(cam.target[0] - cam.position[0], cam.target[1] - cam.position[1], cam.target[2] - cam.position[2])
  expect(Math.abs(got - axial), `pivot at the top face's depth (${axial.toFixed(3)} m), not the ground's`).toBeLessThan(0.01)
})

test('Wireframe adopts no face: a rectangle started on an edge point is drawn on the plane through that point', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const block = await page.evaluate(() => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
    const block = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    h.setCamera({ position: [3.2, -3.6, 4.2], target: [1, 0.5, 0.5], up: [0, 0, 1], fovDeg: 40 })
    return block
  })
  await page.waitForTimeout(300)
  await choose(page, 'Wireframe')
  const before = await page.evaluate(() => window.__hew_test!.getSketchIds())
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  // The top-front edge's midpoint: an edge, not a face.
  await hover(page, [1, 0, 1])
  expect(await chip(page)).toContain('Midpoint')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  await hover(page, [1.6, 0.6, 1])
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  await page.keyboard.press('Escape')
  // Design wireframe-faces-and-solids §2.1 / D2: the click is a named point
  // (the edge midpoint) off the picked ground plane and nearer the eye, so
  // the plane becomes the camera-facing axis plane THROUGH the point, z = 1.
  // That is not a face adoption (no face input, no projection, no error):
  // the rectangle starts at (1, 0, 1) on z = 1, and because the box's top
  // face is coplanar the kernel's ordinary arrangement imprints it there.
  const result = await page.evaluate(({ before, block }) => {
    const t = window.__hew_test!
    const fresh = t.getSketchIds().filter((id) => !before.includes(id))
    return {
      fresh: fresh.length,
      imprints: t.getImprints(block).map((i) => i.loop),
      error: t.getLastError(),
    }
  }, { before, block })
  expect(result.error, 'no error').toBeFalsy()
  expect(result.fresh, 'no free sketch on the ground').toBe(0)
  expect(result.imprints.length, 'one rectangle, arranged on the top face').toBe(1)
  const loop = result.imprints[0]
  for (const v of loop) expect(Math.abs(v[2] - 1), 'on the plane through the point, z = 1').toBeLessThan(1e-9)
  expect(loop.some((v) => Math.abs(v[0] - 1) < 1e-9 && Math.abs(v[1]) < 1e-9), 'starts at the clicked edge point').toBe(true)
})

test('the orbit pivot finds a drawn, not extruded shape in Wireframe too', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  // The orbit target sits half a meter in front of the eye, so the plain
  // ground fallback (at most four times that) cannot reach the fill 6.7 m
  // away: only the sketch region can move the pivot there.
  const eye: Vec3 = [0, -6, 3]
  const dir = [0 - eye[0], 0 - eye[1], 0 - eye[2]]
  const len = Math.hypot(dir[0], dir[1], dir[2])
  const target: Vec3 = [eye[0] + (dir[0] / len) * 0.5, eye[1] + (dir[1] / len) * 0.5, eye[2] + (dir[2] / len) * 0.5]
  await page.evaluate(({ eye, target }) => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
    h.drawRectangle([-1, -1, 0], [1, 1, 0])
    h.setCamera({ position: eye, target, up: [0, 0, 1], fovDeg: 40 })
  }, { eye, target })
  await page.waitForTimeout(300)
  await choose(page, 'Wireframe')
  const p = await px(page, [0, 0, 0])
  await page.mouse.move(p.x, p.y)
  await page.mouse.down({ button: 'middle' })
  await page.mouse.up({ button: 'middle' })
  await page.waitForTimeout(150)
  const cam = await page.evaluate(() => window.__hew_test!.getCamera())
  const got = Math.hypot(cam.target[0] - cam.position[0], cam.target[1] - cam.position[1], cam.target[2] - cam.position[2])
  expect(Math.abs(got - len), `pivot on the fill (${len.toFixed(3)} m), not left at 0.5 m`).toBeLessThan(0.01)
})

test('switching the mode with the pointer parked re-resolves the cue there, both ways', async ({ page }) => {
  test.setTimeout(60_000)
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
    const block = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const mortise = h.drawBox([0.7, 0.3, 0], [1.3, 0.7, 0], 0.6)
    h.boolean(1, block, mortise)
    h.setCamera({ position: [3.2, -3.6, 4.2], target: [1, 0.5, 0.5], up: [0, 0, 1], fovDeg: 40 })
  })
  await page.waitForTimeout(300)
  await page.getByRole('radio', { name: 'Tape Measure' }).click()
  await hover(page, HIDDEN_MID)
  expect(await chip(page)).toContain('On Face')
  // Through the command palette, by keyboard only: the mouse never moves.
  const viaPalette = async (): Promise<void> => {
    await page.keyboard.press('Control+/')
    // Type only once the palette has reset and focused its search field:
    // the reset runs on open and the focus a frame later, so keys sent
    // straight after the shortcut land on the previous run's text.
    const search = page.getByRole('dialog', { name: /command palette/i }).getByPlaceholder(/search/i)
    await expect(search).toBeFocused()
    await expect(search).toHaveValue('')
    await page.keyboard.type('X-ray')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(300)
  }
  await viaPalette()
  await expect.poll(() => chip(page), { timeout: 3_000 }).toContain('Midpoint')
  await viaPalette()
  await expect.poll(() => chip(page), { timeout: 3_000 }).toContain('On Face')
})
