import { test, expect, type Page, type TestInfo } from '@playwright/test'
import { settleFrame } from './helpers/render'
import { MULTI_CLICK_MS } from '../src/viewport/multiClick'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * The Fillet tool (Shift+F), driven with real pointer and keyboard events:
 * hover highlight, the gizmo that appears on arming, sizing by dragging its
 * handle, a click on the handle to finish, typed retype, the clamp message,
 * Shift-click collection, double-click repeat, the Alt tap chamfer, the
 * Push/Pull resize, editing a band by its handle, deleting a blend from its
 * hover, and undo/redo of the whole run by document state hash. The harness
 * only builds the box, reads back state, and stands in for hovering one
 * bottom edge.
 *
 * Set HEW_E2E_SHOTS to a directory to keep the screenshots there.
 */

const CAMERA = {
  position: [7, -7, 6] as [number, number, number],
  target: [1, 1, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 60_000 })
  await page.evaluate((cam) => {
    window.__hew_test!.setLengthUnit('mm')
    window.__hew_test!.setCamera(cam)
  }, CAMERA)
  await page.waitForTimeout(100)
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function moveToWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y, { steps: 3 })
}

async function clickWorld(page: Page, w: [number, number, number]): Promise<void> {
  await moveToWorld(page, w)
  await page.mouse.down()
  await page.mouse.up()
}

async function shot(page: Page, info: TestInfo, name: string): Promise<void> {
  await settleFrame(page)
  const dir = process.env.HEW_E2E_SHOTS
  await page.screenshot({ path: dir ? `${dir}/${name}.png` : info.outputPath(`${name}.png`) })
}

const SQRT2 = Math.SQRT2

/** Where the radius handle sits on the top front edge of the 2 m box once
 *  that edge is armed: off the edge at [1, 0, 1] along the bisector of the
 *  top and front faces, `size` meters out. */
function topFrontHandle(size: number): [number, number, number] {
  return [1, -size / SQRT2, 1 + size / SQRT2]
}

/** The same for the top back edge, armed at [1, 2, 1]. */
function topBackHandle(size: number): [number, number, number] {
  return [1, 2 + size / SQRT2, 1 + size / SQRT2]
}

/** Press on the handle at `from`, drag to `to`, let go. */
async function dragHandle(
  page: Page,
  from: [number, number, number],
  to: [number, number, number],
): Promise<void> {
  await moveToWorld(page, from)
  await page.mouse.down()
  await moveToWorld(page, to)
  await page.mouse.up()
  await page.waitForTimeout(100)
}

const hash = (page: Page): Promise<string> => page.evaluate(() => window.__hew_test!.getModelStateHash())
const depth = (page: Page): Promise<number> => page.evaluate(() => window.__hew_test!.getUndoDepth())
const features = (page: Page, id: string) => page.evaluate((id) => window.__hew_test!.blendFeatures(id), id)

test('Fillet: activating with an object selected arms all its sharp edges', async ({ page }) => {
  await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), id)
  // The harness selects through app state; let the selection commit before
  // the shortcut, as a person's click-then-key always does.
  await expect(page.getByText('1 object')).toBeVisible()
  await page.keyboard.press('Shift+F')
  // Armed straight away: typing a size and Enter rounds every edge, no clicks.
  await page.keyboard.type('10')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  expect((await features(page, id)).filter((r) => r.kind === 'fillet').length).toBeGreaterThan(0)
})

test('Fillet: pick, size, retype, clamp, collect, repeat, chamfer, resize, delete, undo and redo', async ({ page }, info) => {
  test.setTimeout(120_000)
  await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  const hashes: string[] = [await hash(page)]
  const depths: number[] = [await depth(page)]

  // 1. Shift+F, then hover the top front edge: it highlights.
  await page.keyboard.press('Shift+F')
  await expect(page.getByText('Click an edge or a face to round it', { exact: false })).toBeVisible()
  await moveToWorld(page, [1, 0.03, 1])
  await page.waitForTimeout(150)
  await shot(page, info, '01-hover-edge')
  const px = await page.evaluate(() => window.__hew_test!.pixelColorAt([1, 0, 1]))
  expect(px, 'the edge pixel under the highlight').not.toBeNull()
  expect(px!.r).toBeGreaterThan(180)
  expect(px!.g).toBeGreaterThan(100)
  expect(px!.b).toBeLessThan(110)

  // 2. Click the edge: it arms at once at a tenth of its 2 m length (0.2),
  // with the handle off the edge and the value beside it. Nothing is
  // committed yet, and moving the pointer sizes nothing.
  await clickWorld(page, [1, 0.03, 1])
  await page.waitForTimeout(150)
  // At this zoom the handle sits within a few pixels of the pointer, so the
  // line may already read as the over-the-handle hint.
  await expect(page.getByText(/Drag (the|this) handle/)).toBeVisible()
  await expect(page.getByText('Radius', { exact: true })).toBeVisible()
  expect(await features(page, id)).toHaveLength(0)
  await shot(page, info, '02-armed-gizmo')
  const armedPx = await page.evaluate((w) => window.__hew_test!.pixelColorAt(w), topFrontHandle(0.2))
  await moveToWorld(page, [1, 0.6, 1])
  await page.waitForTimeout(100)
  expect(await features(page, id)).toHaveLength(0)

  // Drag the handle out along its line to 0.5 and let go: still armed, the
  // preview holds at the new size.
  await dragHandle(page, topFrontHandle(0.2), topFrontHandle(0.5))
  await expect(page.getByText(/Drag (the|this) handle/)).toBeVisible()
  expect(await features(page, id)).toHaveLength(0)
  await shot(page, info, '02b-handle-dragged')

  // A click on the handle, with no drag, finishes: one blend at 0.5.
  await clickWorld(page, topFrontHandle(0.5))
  await page.waitForTimeout(150)
  let rows = await features(page, id)
  expect(rows).toHaveLength(1)
  expect(rows[0].kind).toBe('fillet')
  expect(rows[0].size!).toBeGreaterThan(0.45)
  expect(rows[0].size!).toBeLessThan(0.55)
  // The handle is gone from where it stood.
  const doneHandlePx = await page.evaluate((w) => window.__hew_test!.pixelColorAt(w), topFrontHandle(0.5))
  expect(doneHandlePx).not.toEqual(armedPx)
  hashes.push(await hash(page))
  depths.push(await depth(page))

  // 3. Type 5 (millimeters), Enter: the same fillet at 5 mm, still one blend.
  await page.keyboard.type('5')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  rows = await features(page, id)
  expect(rows).toHaveLength(1)
  expect(rows[0].size!).toBeCloseTo(0.005, 6)
  hashes[1] = await hash(page) // a retype replaces the step, it does not add one
  depths[1] = await depth(page)
  await shot(page, info, '03-retyped')

  // 4. Arm the top back edge, then drag its handle far past what the box
  // allows: the preview clamps and the status line names the limit.
  await clickWorld(page, [1, 1.97, 1])
  // The 5 mm handle sits within a few pixels of that click, so a press on
  // it inside the double-click window is the click's second half (a double
  // click finishes the pick). Wait the window out: this is a fresh press.
  await page.waitForTimeout(MULTI_CLICK_MS + 100)
  // It arms at the last size made, 5 mm, so that is where its handle is.
  await dragHandle(page, topBackHandle(0.005), topBackHandle(1.6))
  await page.waitForTimeout(150)
  await expect(page.getByText('Radius limited to', { exact: false })).toBeVisible()
  await shot(page, info, '04-clamped')
  await page.keyboard.press('Escape')
  expect(await hash(page)).toBe(hashes[1]) // nothing committed

  // 5. Shift-click two edges, type 2, Enter: both rounded in one undo step.
  await page.keyboard.down('Shift')
  await clickWorld(page, [0.03, 1, 1])
  await clickWorld(page, [1, 1.97, 1])
  await page.keyboard.up('Shift')
  await page.keyboard.type('2')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  rows = await features(page, id)
  expect(rows.length).toBeGreaterThanOrEqual(2)
  hashes.push(await hash(page))
  depths.push(await depth(page))
  await shot(page, info, '05-collected')

  // 6. Double-click another edge: the last size (2 mm) is repeated on it.
  const before = rows.length
  const p = await pagePoint(page, [2, 1, 0.03])
  await page.mouse.move(p.x, p.y, { steps: 3 })
  await page.mouse.dblclick(p.x, p.y)
  await page.waitForTimeout(200)
  rows = await features(page, id)
  expect(rows.length).toBeGreaterThan(before)
  expect(rows[rows.length - 1].size!).toBeCloseTo(0.002, 6)
  hashes.push(await hash(page))
  depths.push(await depth(page))

  // 7. Alt (a tap) swaps to Chamfer; type 3 on the bottom front edge of a
  // second box (clear of the first box's blends).
  const id2 = await page.evaluate(() => window.__hew_test!.drawBox([3, 0, 0], [5, 2, 0], 1))
  hashes.push(await hash(page))
  depths.push(await depth(page))
  await page.keyboard.press('Alt')
  await clickWorld(page, [4, 0.02, 0.03])
  await page.keyboard.type('3')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  rows = await features(page, id2)
  expect(rows.some((r) => r.kind === 'chamfer')).toBe(true)
  hashes.push(await hash(page))
  depths.push(await depth(page))
  await shot(page, info, '07-chamfer')
  await page.keyboard.press('Alt') // back to Round

  // 8. Push/Pull on a strip facet changes the radius. The box's top front
  // edge takes a 0.2 fillet (the harness stands in for hovering it).
  await page.keyboard.press('Escape')
  const big = await page.evaluate(
    (id) => window.__hew_test!.filletEdges(id, [[4, 0, 1]], '{"kind":"round","radius":0.2,"segments":6}'),
    id2,
  )
  expect(big).toHaveLength(1)
  hashes.push(await hash(page))
  depths.push(await depth(page))
  const bigSize = (await features(page, id2)).find((r) => r.kind === 'fillet')!.size!
  const strip: [number, number, number] = [4, 0.0586, 0.9414]
  await page.keyboard.press('p')
  await clickWorld(page, strip)
  await page.keyboard.type('30')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  const resized = (await features(page, id2)).filter((r) => r.kind === 'fillet')
  expect(resized).toHaveLength(1)
  expect(Math.abs(resized[0].size! - bigSize)).toBeGreaterThan(1e-4)
  hashes.push(await hash(page))
  depths.push(await depth(page))
  await shot(page, info, '08-pushpull')

  // 9. Edit the band. A side view, so the handle's line runs across the
  // screen instead of at the eye. Click the band: its handle appears on it at
  // the current size, out along the band's normal.
  const bandSize = (await features(page, id2)).find((r) => r.kind === 'fillet')!.size!
  await page.evaluate(() => window.__hew_test!.setCamera({
    position: [14, 1, 1.5], target: [4, 1, 0.6], up: [0, 0, 1], fovDeg: 14,
  }))
  await settleFrame(page)
  // A point on the blend's strip: the middle of the facet 37.5 degrees round
  // the arc of the top front edge (x = 4), the arc's centre r in from both faces.
  const bandAt = (r: number): [number, number, number] => {
    const a = (37.5 * Math.PI) / 180
    const rr = r * Math.cos((7.5 * Math.PI) / 180)
    return [4, r - rr * Math.cos(a), 1 - r + rr * Math.sin(a)]
  }
  await page.keyboard.press('Shift+F')
  await moveToWorld(page, bandAt(bandSize))
  await page.waitForTimeout(150)
  await expect(page.getByText('Click to resize this fillet', { exact: false })).toBeVisible()
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  await expect(page.getByText('Drag the handle or type a size', { exact: false })).toBeVisible()
  await shot(page, info, '09a-edit-handle')
  // Esc closes the edit with nothing changed.
  await page.keyboard.press('Escape')
  expect((await features(page, id2)).filter((r) => r.kind === 'fillet')[0].size!).toBeCloseTo(bandSize, 9)

  // Press on the band, drag outward along its bisector, let go: one gesture
  // that resizes and keeps the edit open. A click on empty space finishes it.
  await dragHandle(page, bandAt(bandSize), [4, -0.17, 1.17])
  await expect(page.getByText('Drag the handle or type a size', { exact: false })).toBeVisible()
  await shot(page, info, '09b-edit-dragged')
  expect((await features(page, id2)).filter((r) => r.kind === 'fillet')[0].size!).toBeCloseTo(bandSize, 9) // not yet
  await page.mouse.click(930, 480)
  await page.waitForTimeout(250)
  const edited = (await features(page, id2)).filter((r) => r.kind === 'fillet')
  expect(edited).toHaveLength(1)
  // Sized by where the pointer let go; the exact figure depends on the facet
  // under the press, so the check is that the band grew.
  expect(edited[0].size!).toBeGreaterThan(bandSize + 0.03)
  hashes.push(await hash(page))
  depths.push(await depth(page))

  // 10. Hover the blend and press Delete: its sharp edge is restored.
  await page.keyboard.press('Shift+F')
  // The strip's middle, on the arc itself (its 45 degree point).
  const mid = edited[0].size! * (1 - 1 / SQRT2)
  await moveToWorld(page, [4, mid, 1 - mid])
  await page.waitForTimeout(150)
  await expect(page.getByText('Click to resize this fillet', { exact: false })).toBeVisible()
  await page.keyboard.press('Delete')
  await page.waitForTimeout(200)
  expect((await features(page, id2)).filter((r) => r.kind === 'fillet')).toHaveLength(0)
  hashes.push(await hash(page))
  depths.push(await depth(page))
  await shot(page, info, '10-deleted')

  // 11. Undo back through every recorded step to the bare box, then redo
  // them all; each stop's state hash matches the one recorded going forward.
  // (A harness-built box is two steps, so walk by undo depth.)
  for (let i = hashes.length - 1; i > 0; i--) {
    while ((await depth(page)) > depths[i - 1]) await page.evaluate(() => window.__hew_test!.undo())
    expect(await hash(page), `after undoing back to step ${i - 1}`).toBe(hashes[i - 1])
  }
  for (let i = 1; i < hashes.length; i++) {
    while ((await depth(page)) < depths[i]) await page.evaluate(() => window.__hew_test!.redo())
    expect(await hash(page), `after redoing to step ${i}`).toBe(hashes[i])
  }
})

test('Fillet: Ctrl+A arms only the object under the cursor, not every object', async ({ page }) => {
  await setup(page)
  const a = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  const b = await page.evaluate(() => window.__hew_test!.drawBox([3, 0, 0], [5, 2, 0], 1))
  await page.evaluate((a) => window.__hew_test!.selectObjects([a]), a)
  await page.keyboard.press('Shift+F')
  await moveToWorld(page, [4, 1, 1])
  await page.waitForTimeout(150)
  await page.keyboard.press('Control+a')
  await page.keyboard.type('3')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  expect((await features(page, b)).filter((r) => r.kind === 'fillet').length).toBeGreaterThan(0)
  expect(await features(page, a)).toHaveLength(0)
})

test('Fillet: a Ctrl+Alt chord (precision snapping) keeps the profile; a bare Alt tap swaps it; 8s is confirmed', async ({ page }) => {
  await setup(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.keyboard.press('Shift+F')
  const round = page.getByText('Click an edge or a face to round it', { exact: false })
  await expect(round).toBeVisible()
  // The chord, released either way round, is not an Alt tap.
  await page.keyboard.down('Control')
  await page.keyboard.down('Alt')
  await page.keyboard.up('Alt')
  await page.keyboard.up('Control')
  await expect(round).toBeVisible()
  await page.keyboard.down('Control')
  await page.keyboard.down('Alt')
  await page.keyboard.up('Control')
  await page.keyboard.up('Alt')
  await expect(round).toBeVisible()
  // A bare tap still swaps.
  await page.keyboard.press('Alt')
  await expect(page.getByText('Click an edge or a face to chamfer it', { exact: false })).toBeVisible()
  await page.keyboard.press('Alt')
  await expect(round).toBeVisible()
  // A typed facet count is confirmed in the status line while the pick stays armed.
  await clickWorld(page, [1, 0.03, 1])
  await page.keyboard.type('8s')
  await page.keyboard.press('Enter')
  await expect(page.getByText('Facets set to 8.')).toBeVisible()
})

test('Fillet: after a swept blend commits, the refusal text and the radius readout are gone', async ({ page }, info) => {
  await setup(page)
  await page.evaluate(() => window.__hew_test!.setCamera({
    position: [0.5, 0.9, 0.9], target: [0.5, -0.4, 0.4], up: [0, 0, 1], fovDeg: 45,
  }))
  // `setCamera` leaves the camera's matrices stale until the next render, so
  // `worldToScreen` below would project through the previous pose on a busy
  // machine. Yield to the render loop first.
  await settleFrame(page)
  const h = await page.evaluate(() => {
    const t = window.__hew_test!
    const a = t.drawCircle([0.5, -0.5, 0], 0.1, 32)
    const pipe = t.extrudeRegion(a.sketch, a.region, 0.8)
    const b = t.drawCircle([0.5, 0.4, 0], 0.07, 32)
    const branch = t.extrudeRegion(b.sketch, b.region, 0.5)
    t.rotateObject(branch, 90, [1, 0, 0])
    return t.boolean(0, pipe, branch)
  })
  await page.evaluate((h) => window.__hew_test!.selectObjects([h]), h)
  await page.keyboard.press('Shift+F')
  // The selection armed every sharp edge of the tee; Esc drops that, then the
  // junction loop is picked on its own and sized by typing.
  await page.keyboard.press('Escape')
  await moveToWorld(page, [0.533, -0.4056, 0.4617])
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(250)
  // Armed on the junction loop: the handle and its value are up, the pointer
  // sizes nothing.
  await page.mouse.move(900, 90, { steps: 8 })
  await page.waitForTimeout(250)
  await shot(page, info, '11-junction-armed')
  await page.keyboard.type('30')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(250)
  await shot(page, info, '12-junction-committed')
  await page.mouse.move(905, 95, { steps: 3 })
  await page.waitForTimeout(250)
  for (const v of ['10', '20', '50']) {
    await page.keyboard.type(v)
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    await shot(page, info, `13-retype-${v}`)
  }
  expect((await features(page, h)).length).toBeGreaterThan(0)
  await expect(page.getByText("Can't blend this corner", { exact: false })).toHaveCount(0)
  await expect(page.getByText('Radius', { exact: true })).toHaveCount(0)
})

/** The rail's checked radio is the active tool. */
const activeTool = (page: Page): Promise<string> =>
  page.locator('[role="radio"][aria-checked="true"]').first().getAttribute('aria-label').then(
    (l) => l ?? '',
  )

/** After a commit the keyboard must be the app's again: Space resets to
 *  Select, then a bare letter picks its tool. */
async function expectKeyboardFree(page: Page, label: string): Promise<void> {
  await page.keyboard.press(' ')
  await expect.poll(() => activeTool(page), { message: `${label}: Space` }).toContain('Select')
  await page.keyboard.press('Shift+F')
  await expect.poll(() => activeTool(page)).toContain('Fillet')
  await page.keyboard.press('l')
  await expect.poll(() => activeTool(page), { message: `${label}: L` }).toContain('Line')
}

test('Fillet: after a click commit and after a typed commit the keyboard is free again', async ({ page }) => {
  await setup(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.keyboard.press('Shift+F')
  await clickWorld(page, [1, 0.03, 1])
  await page.waitForTimeout(150)
  await dragHandle(page, topFrontHandle(0.2), topFrontHandle(0.5))
  // A click on the handle finishes the pick.
  await clickWorld(page, topFrontHandle(0.5))
  await page.waitForTimeout(150)
  await expectKeyboardFree(page, 'after a click commit')

  await page.keyboard.press('Shift+F')
  await clickWorld(page, [1, 1.97, 1])
  await page.keyboard.type('5')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  await expectKeyboardFree(page, 'after a typed commit')
})

test('Fillet: a refused commit leaves the tool armed but not the keyboard', async ({ page }) => {
  await setup(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.keyboard.press('Shift+F')
  await clickWorld(page, [1, 0.03, 1])
  await page.keyboard.type('5000')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBeLessThan(4)
  // Nothing typed any more: Space resets to Select, a letter picks its tool.
  for (let i = 0; i < 4; i++) await page.keyboard.press('Backspace')
  await page.keyboard.press('l')
  await expect.poll(() => activeTool(page)).toContain('Line')
  await page.keyboard.press('Shift+F')
  await clickWorld(page, [1, 1.97, 1])
  await page.keyboard.type('5000')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  for (let i = 0; i < 4; i++) await page.keyboard.press('Backspace')
  await page.keyboard.press(' ')
  await expect.poll(() => activeTool(page)).toContain('Select')
})

test('Fillet: from an armed pick that never committed, Space and L still work; a digit after a commit retypes', async ({ page }) => {
  await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.keyboard.press('Shift+F')
  await clickWorld(page, [1, 0.03, 1])
  // Armed and never finished: Space and a tool letter still reach the app.
  await expectKeyboardFree(page, 'armed, uncommitted')

  await page.keyboard.press('Shift+F')
  await clickWorld(page, [1, 1.97, 1])
  await page.keyboard.type('5')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  await page.keyboard.type('8')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  const sizes = (await features(page, id)).filter((r) => r.kind === 'fillet').map((r) => r.size)
  expect(sizes.some((s) => s !== null && Math.abs(s - 0.008) < 1e-6)).toBe(true)
})

test('Fillet: a click on another edge finishes the armed one and arms the next at the same size', async ({ page }) => {
  await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.keyboard.press('Shift+F')
  await clickWorld(page, [1, 0.03, 1])
  await dragHandle(page, topFrontHandle(0.2), topFrontHandle(0.5))
  expect(await features(page, id)).toHaveLength(0)
  // The back edge: the first pick is committed at 0.5 and the click arms the back edge at 0.5.
  await clickWorld(page, [1, 1.97, 1])
  await page.waitForTimeout(200)
  let rows = await features(page, id)
  expect(rows).toHaveLength(1)
  expect(rows[0].size!).toBeGreaterThan(0.45)
  expect(rows[0].size!).toBeLessThan(0.55)
  const first = rows[0].size!
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  rows = await features(page, id)
  expect(rows).toHaveLength(2)
  expect(rows[1].size!).toBeCloseTo(first, 6)
})

test('Fillet: Shift-click collects edges on two objects, one size rounds both in one undo step', async ({ page }, info) => {
  test.setTimeout(120_000)
  await setup(page)
  const a = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  const b = await page.evaluate(() => window.__hew_test!.drawBox([3, 0, 0], [5, 2, 0], 1))
  const h0 = await hash(page)
  const d0 = await depth(page)

  await page.keyboard.press('Shift+F')
  await page.keyboard.down('Shift')
  await clickWorld(page, [1, 0.03, 1])
  await clickWorld(page, [4, 0.03, 1])
  await page.keyboard.up('Shift')
  await expect(page.getByText(/2 edges picked/)).toBeVisible()
  await shot(page, info, 'x1-two-objects-collected')

  await page.keyboard.type('20')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(250)
  expect((await features(page, a)).length).toBe(1)
  expect((await features(page, b)).length).toBe(1)
  expect(await depth(page)).toBe(d0 + 1)
  await shot(page, info, 'x2-both-rounded')

  await page.evaluate(() => window.__hew_test!.undo())
  expect(await hash(page)).toBe(h0)
  expect(await features(page, a)).toHaveLength(0)
  expect(await features(page, b)).toHaveLength(0)
})

test('Fillet: a swept junction round on a pipe tee resizes from its band and Delete gives the sharp junction back', async ({ page }, info) => {
  // A tee's junction rounds as a rolling-ball (swept) blend. It stays a
  // fillet you can edit: hover the band, click it, type a new size; hover
  // it again and press Delete to get the sharp junction back (D-228).
  test.setTimeout(180_000)
  await setup(page)
  await page.evaluate(() => window.__hew_test!.setCamera({
    position: [0.5, 0.9, 0.9], target: [0.5, -0.4, 0.4], up: [0, 0, 1], fovDeg: 30,
  }))
  await settleFrame(page)
  const tee = await page.evaluate(() => {
    const t = window.__hew_test!
    const a = t.drawCircle([0.5, -0.5, 0], 0.1, 32)
    const pipe = t.extrudeRegion(a.sketch, a.region, 0.8)
    const b = t.drawCircle([0.5, 0.4, 0], 0.07, 32)
    const branch = t.extrudeRegion(b.sketch, b.region, 0.5)
    t.rotateObject(branch, 90, [1, 0, 0])
    return t.boolean(0, pipe, branch)
  })
  const sorted = (vs: [number, number, number][]) =>
    vs.map((v) => v.map((c) => c.toFixed(6)).join(',')).sort()
  const sharp = sorted(await page.evaluate((id) => window.__hew_test!.getObjectVertices(id), tee))
  await shot(page, info, 's1-sharp-junction')

  // Round the junction loop at 20 mm with real pointer and keys.
  await page.keyboard.press('Shift+F')
  await moveToWorld(page, [0.533, -0.4056, 0.4617])
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(250)
  await page.keyboard.type('20')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(400)
  let rows = await features(page, tee)
  expect(rows).toHaveLength(1)
  expect(rows[0].intact).toBe(true)
  expect(rows[0].size!).toBeCloseTo(0.02, 9)
  await page.mouse.move(900, 90, { steps: 4 })
  await page.waitForTimeout(200)
  await shot(page, info, 's2-rounded-20mm')

  // The band at the top of the junction: the round's arc between the main
  // pipe's wall (y = -0.4 at x = 0.5) and the branch's top (z = 0.47), its
  // 45 degree point.
  const bandAt = (r: number): [number, number, number] => {
    const k = r * (1 - Math.SQRT1_2)
    return [0.5, -0.4 + k, 0.47 + k]
  }
  await page.keyboard.press('Escape')
  await moveToWorld(page, bandAt(0.02))
  await page.waitForTimeout(200)
  await expect(page.getByText('Click to resize this fillet', { exact: false })).toBeVisible()
  await shot(page, info, 's3-hover-band')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(250)
  await expect(page.getByText('Drag the handle or type a size', { exact: false })).toBeVisible()
  await page.keyboard.type('10')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(400)
  rows = await features(page, tee)
  expect(rows).toHaveLength(1)
  expect(rows[0].intact).toBe(true)
  expect(rows[0].size!).toBeCloseTo(0.01, 9)
  await page.mouse.move(900, 90, { steps: 4 })
  await page.waitForTimeout(200)
  await shot(page, info, 's4-resized-10mm')

  // Hover the band again and press Delete: the sharp junction comes back.
  await page.keyboard.press('Escape')
  await moveToWorld(page, bandAt(0.01))
  await page.waitForTimeout(200)
  await expect(page.getByText('Click to resize this fillet', { exact: false })).toBeVisible()
  await page.keyboard.press('Delete')
  await page.waitForTimeout(400)
  expect(await features(page, tee)).toHaveLength(0)
  expect(sorted(await page.evaluate((id) => window.__hew_test!.getObjectVertices(id), tee))).toEqual(sharp)
  await page.mouse.move(900, 90, { steps: 4 })
  await page.waitForTimeout(200)
  await shot(page, info, 's5-deleted')

  // Undo brings the 10 mm round back, still editable.
  await page.evaluate(() => window.__hew_test!.undo())
  rows = await features(page, tee)
  expect(rows).toHaveLength(1)
  expect(rows[0].intact).toBe(true)
  expect(rows[0].size!).toBeCloseTo(0.01, 9)
})

test('Fillet: an end-radius grip dragged with the mouse is placed on release and the round commits', async ({ page }, info) => {
  test.setTimeout(90_000)
  await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  // Close in on the far end of the edge, so a short drag of the grip is
  // well over the few pixels that tell a drag from a click.
  await page.evaluate(() => window.__hew_test!.setCamera({
    position: [4, -2.5, 2.5], target: [1.5, 0, 1], up: [0, 0, 1], fovDeg: 30,
  }))
  await settleFrame(page)
  await page.keyboard.press('Shift+F')
  await moveToWorld(page, [1, 0.03, 1])
  await page.waitForTimeout(150)
  await clickWorld(page, [1, 0.03, 1])
  await page.waitForTimeout(150)
  // Size the round to 0.3 with the handle, so the end grips stand clear of the stops.
  await page.keyboard.type('150')
  await page.waitForTimeout(MULTI_CLICK_MS + 100)
  // The far end's radius grip, out along the bisector at the end of the edge.
  const endGrip = (r: number): [number, number, number] => [2, -r / SQRT2, 1 + r / SQRT2]
  const px = (w: [number, number, number]) => page.evaluate((w) => window.__hew_test!.pixelColorAt(w), w)
  const oldBefore = await px(endGrip(0.15))
  const newBefore = await px(endGrip(0.04))
  await dragHandle(page, endGrip(0.15), endGrip(0.04))
  await shot(page, info, 'endgrip-released')
  // Placed on release: the grip now stands at the released radius (0.04) and
  // has left the radius it was picked up at (0.15).
  expect(await px(endGrip(0.04))).not.toEqual(newBefore)
  expect(await px(endGrip(0.15))).not.toEqual(oldBefore)
  // Nothing is carried any more: the status is the handle's, not the grip's.
  await expect(page.getByText('Move to set the radius here')).toHaveCount(0)
  await moveToWorld(page, [1, 1.2, 1])
  await shot(page, info, 'endgrip-away')
  await page.waitForTimeout(100)
  await expect(page.getByText('Move to set the radius here')).toHaveCount(0)
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  const rows = await features(page, id)
  expect(rows).toHaveLength(1)
})

test('Fillet: clicking a whole loop arms it without a refusal message first', async ({ page }, info) => {
  test.setTimeout(120_000)
  await setup(page)
  await page.evaluate(() => window.__hew_test!.setCamera({
    position: [0.5, 0.9, 0.9], target: [0.5, -0.4, 0.4], up: [0, 0, 1], fovDeg: 30,
  }))
  await settleFrame(page)
  const tee = await page.evaluate(() => {
    const t = window.__hew_test!
    const a = t.drawCircle([0.5, -0.5, 0], 0.1, 32)
    const pipe = t.extrudeRegion(a.sketch, a.region, 0.8)
    const b = t.drawCircle([0.5, 0.4, 0], 0.07, 32)
    const branch = t.extrudeRegion(b.sketch, b.region, 0.5)
    t.rotateObject(branch, 90, [1, 0, 0])
    return t.boolean(0, pipe, branch)
  })
  await page.keyboard.press('Shift+F')
  await moveToWorld(page, [0.533, -0.4056, 0.4617])
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.keyboard.type('50')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(500)
  await page.evaluate(() => window.__hew_test!.undo())
  await page.keyboard.press('Escape')
  await moveToWorld(page, [0.533, -0.4056, 0.4617])
  await page.waitForTimeout(300)
  await page.mouse.down()
  await page.mouse.up()
  // The refusal text would show in the status bar or a toast for a moment.
  const seen: string[] = []
  for (let i = 0; i < 12; i++) {
    seen.push(await page.locator('body').innerText())
    await page.waitForTimeout(60)
  }
  await shot(page, info, 'loop-armed')
  expect(seen.some((t) => t.includes('one at a time'))).toBe(false)
  await page.keyboard.type('20')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(400)
  expect(await features(page, tee)).toHaveLength(1)
})
