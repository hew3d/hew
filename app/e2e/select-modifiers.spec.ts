import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Select tool modifier matrix (viewport/selectModifiers.ts,
 * panels/treeModel.ts `nextSelection`/`mergeSelection`) — SketchUp's click
 * and marquee modifiers, driven end to end with REAL pointer/keyboard
 * events:
 *
 *   | held                    | click / marquee          |
 *   |-------------------------|---------------------------|
 *   | nothing                 | replace                   |
 *   | Shift                   | toggle                    |
 *   | Ctrl / Cmd / Option     | add (never removes)       |
 *   | Shift + Ctrl/Cmd/Option | subtract (never adds)     |
 *
 * Plus triple-click "select all connected" (a line/curve widens to its
 * island), Edit ▸ Select None / Invert Selection (menu + Ctrl+Shift+A), and
 * drag-to-move staying unaffected by the modifier matrix (a modified press
 * never arms a move, only changes the selection).
 */

const CAMERA_BOXES = {
  position: [2, -9, 7] as [number, number, number],
  target: [2, 0.5, 0.3] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 50,
}

const CAMERA_TOPDOWN = {
  position: [2, 1, 10] as [number, number, number],
  target: [2, 1, 0] as [number, number, number],
  up: [0, 1, 0] as [number, number, number],
  fovDeg: 45,
}

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

/** Draw two 1x1x1 boxes well separated in X, front faces facing -Y (toward
 * CAMERA_BOXES). Returns their object ids. */
async function drawTwoBoxes(page: Page): Promise<{ a: string; b: string }> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const b = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    return { a, b }
  })
}

async function selKinds(page: Page): Promise<{ kind: string; id: string }[]> {
  return page.evaluate(() => window.__hew_test!.getSelection())
}

async function selIds(page: Page): Promise<string[]> {
  return (await selKinds(page)).map((s) => s.id)
}

/** Click at a world point with the given modifier keys held. */
async function modClick(
  page: Page,
  p: { x: number; y: number },
  mods: { shift?: boolean; ctrl?: boolean },
): Promise<void> {
  if (mods.shift) await page.keyboard.down('Shift')
  if (mods.ctrl) await page.keyboard.down('Control')
  await page.mouse.click(p.x, p.y)
  if (mods.ctrl) await page.keyboard.up('Control')
  if (mods.shift) await page.keyboard.up('Shift')
}

/** Rubber-band drag between two world points. Direction (as PROJECTED to
 * screen) decides window (left→right) vs crossing (right→left) — see
 * marquee.ts. `mode` picks which way this helper drives the drag,
 * regardless of which world point happens to be visually left/right under
 * the current camera. */
async function marqueeDrag(
  page: Page,
  wA: [number, number, number],
  wB: [number, number, number],
  mode: 'window' | 'crossing',
  mods: { shift?: boolean; ctrl?: boolean },
): Promise<void> {
  const pa = await pagePoint(page, wA)
  const pb = await pagePoint(page, wB)
  const [left, right] = pa.x <= pb.x ? [pa, pb] : [pb, pa]
  const [start, end] = mode === 'window' ? [left, right] : [right, left]
  if (mods.shift) await page.keyboard.down('Shift')
  if (mods.ctrl) await page.keyboard.down('Control')
  await page.mouse.move(start.x, start.y)
  await page.mouse.down()
  await page.mouse.move((start.x + end.x) / 2, (start.y + end.y) / 2, { steps: 5 })
  await page.mouse.move(end.x, end.y, { steps: 6 })
  await page.mouse.up()
  if (mods.ctrl) await page.keyboard.up('Control')
  if (mods.shift) await page.keyboard.up('Shift')
}

test('click modifier matrix: replace / add / subtract / toggle, and a modified click on air leaves the selection alone', async ({ page }) => {
  await setup(page)
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA_BOXES)
  const { a, b } = await drawTwoBoxes(page)
  await page.waitForTimeout(100)

  const aFront: [number, number, number] = [0.5, 0, 0.5]
  const bFront: [number, number, number] = [3.5, 0, 0.5]
  const air: [number, number, number] = [2, -3, 0]

  const pA = await pagePoint(page, aFront)
  const pB = await pagePoint(page, bFront)
  const pAir = await pagePoint(page, air)

  // Plain click A -> [A]
  await modClick(page, pA, {})
  expect(await selIds(page)).toEqual([a])

  // Ctrl-click B -> [A, B] (add, order preserved)
  await modClick(page, pB, { ctrl: true })
  expect(await selIds(page)).toEqual([a, b])

  // Ctrl-click A again -> still [A, B] (add never removes)
  await modClick(page, pA, { ctrl: true })
  expect(await selIds(page)).toEqual([a, b])

  // Shift+Ctrl-click A -> [B] (subtract)
  await modClick(page, pA, { shift: true, ctrl: true })
  expect(await selIds(page)).toEqual([b])

  // Shift-click B -> [] (toggle off)
  await modClick(page, pB, { shift: true })
  expect(await selIds(page)).toEqual([])

  // Shift-click on empty ground with an EMPTY selection: still nothing to
  // assert beyond "unchanged" -- reselect something first.
  await modClick(page, pA, {})
  expect(await selIds(page)).toEqual([a])

  // Shift-click on empty air with a selection present -> unchanged (a
  // modified click on air is not a deselect).
  await modClick(page, pAir, { shift: true })
  expect(await selIds(page)).toEqual([a])

  // Plain click on empty ground -> [].
  await modClick(page, pAir, {})
  expect(await selIds(page)).toEqual([])
})

test('marquee modifier matrix: window select toggles, adds, and subtracts', async ({ page }) => {
  await setup(page)
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA_BOXES)
  const { a, b } = await drawTwoBoxes(page)
  await page.waitForTimeout(100)

  const aFront: [number, number, number] = [0.5, 0, 0.5]
  const swA: [number, number, number] = [-0.6, -0.6, -0.2]
  const neA: [number, number, number] = [1.6, 1.6, 1.3]
  const swBoth: [number, number, number] = [-0.6, -0.6, -0.2]
  const neBoth: [number, number, number] = [4.6, 1.6, 1.3]
  const air: [number, number, number] = [2, -3, 0]

  // Select A with a click.
  await modClick(page, await pagePoint(page, aFront), {})
  expect(await selIds(page)).toEqual([a])

  // Shift-drag a window box around BOTH -> toggle: A leaves, B joins -> [B].
  await marqueeDrag(page, swBoth, neBoth, 'window', { shift: true })
  expect(await selIds(page)).toEqual([b])

  // Clear, then Ctrl-drag around both with nothing selected -> [A, B] (add).
  await modClick(page, await pagePoint(page, air), {})
  expect(await selIds(page)).toEqual([])
  await marqueeDrag(page, swBoth, neBoth, 'window', { ctrl: true })
  expect(await selIds(page)).toEqual(expect.arrayContaining([a, b]))
  expect((await selIds(page)).length).toBe(2)

  // Shift+Ctrl-drag around A ONLY with [A,B] selected -> subtract A -> [B].
  await marqueeDrag(page, swA, neA, 'window', { shift: true, ctrl: true })
  expect(await selIds(page)).toEqual([b])
})

test('triple-click on a polyline segment selects the whole connected island; a single click selects only that edge', async ({ page }) => {
  await setup(page)
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA_TOPDOWN)
  await page.evaluate(() => {
    window.__hew_test!.drawLineChain([
      [0, 0, 0],
      [2, 0, 0],
      [2, 2, 0],
      [4, 2, 0],
    ])
  })
  await page.waitForTimeout(100)

  const midSegment: [number, number, number] = [2, 1, 0] // midpoint of the middle segment
  const p = await pagePoint(page, midSegment)

  // Single click -> one draft-edge.
  await page.mouse.click(p.x, p.y)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'draft-edge'
  })

  // Triple-click at the same point -> one draft-island. The 2nd press of
  // the run may be routed as a plain click (a double-click on a bare line
  // does nothing) -- only the FINAL state matters.
  await page.mouse.click(p.x, p.y, { clickCount: 3 })
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'draft-island'
  })
})

test('Edit menu: Select None and Invert Selection, plus the Ctrl+Shift+A binding', async ({ page }) => {
  await setup(page)
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA_BOXES)
  const { a, b } = await drawTwoBoxes(page)
  await page.evaluate(() => window.__hew_test!.drawLineChain([[0, 3, 0], [2, 3, 0]]))
  await page.waitForTimeout(100)

  const aFront: [number, number, number] = [0.5, 0, 0.5]
  await modClick(page, await pagePoint(page, aFront), {})
  expect(await selIds(page)).toEqual([a])
  // The Viewport's OWN internal selection ref (what invertSelection/menu
  // actions read) syncs from the app-level selection via a `useEffect`, one
  // tick behind the harness's `getSelection()` (an app-level accessor kept
  // fresh every render) -- give it a moment to flush before driving a menu
  // action that depends on it.
  await page.waitForTimeout(100)

  // Edit -> Select None.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText('Select None', { exact: true }).click()
  expect(await selIds(page)).toEqual([])

  // Reselect A, then Edit -> Invert Selection -> everything else Select All
  // would pick: B and the polyline's sketch island. Assert it contains B
  // and not A.
  await modClick(page, await pagePoint(page, aFront), {})
  expect(await selIds(page)).toEqual([a])
  await page.waitForTimeout(100)
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText('Invert Selection', { exact: true }).click()
  const inverted = await selKinds(page)
  // Note: sketch-island handles and object handles are separate id spaces,
  // so comparing bare id strings across kinds is unsound (a coincidental
  // numeric collision is expected, not a bug) -- match on (kind, id).
  expect(inverted.some((n) => n.kind === 'object' && n.id === b)).toBe(true)
  expect(inverted.some((n) => n.kind === 'object' && n.id === a)).toBe(false)

  // Keyboard: with [A] selected, Ctrl+Shift+A -> [].
  await modClick(page, await pagePoint(page, aFront), {})
  expect(await selIds(page)).toEqual([a])
  await page.waitForTimeout(100)
  await page.keyboard.press('ControlOrMeta+Shift+A')
  expect(await selIds(page)).toEqual([])
})

test('drag-to-move: a plain click-drag on a node still moves it', async ({ page }) => {
  await setup(page)
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA_BOXES)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.waitForTimeout(100)

  const before = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
  const grab = await pagePoint(page, [0.5, 0, 0.5])
  const drop = await pagePoint(page, [1.7, 0, 0.5])

  await page.mouse.move(grab.x, grab.y)
  await page.mouse.down()
  await page.mouse.move((grab.x + drop.x) / 2, (grab.y + drop.y) / 2, { steps: 5 })
  await page.mouse.move(drop.x, drop.y, { steps: 6 })
  await page.mouse.up()
  await page.waitForTimeout(150)

  const after = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
  // Moved a noticeable amount along +X (the drag direction); Y/Z stay
  // roughly where they were (a ground drag stays on the ground plane).
  expect(after[0] - before[0]).toBeGreaterThan(0.3)
  expect(await selIds(page)).toEqual([id])
})

test('drag-to-move: a Ctrl-held press on a node does NOT arm a move, only changes the selection', async ({ page }) => {
  await setup(page)
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA_BOXES)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.waitForTimeout(100)

  const before = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
  const grab = await pagePoint(page, [0.5, 0, 0.5])
  // A short drag, deliberately RIGHT-TO-LEFT so the resulting marquee is a
  // "crossing" selection (touching, not full-containment) -- the press
  // itself lands exactly on the object, so a crossing marquee is guaranteed
  // to pick it up regardless of how far the drag travels.
  const away = { x: grab.x - 60, y: grab.y - 20 }

  await page.keyboard.down('Control')
  await page.mouse.move(grab.x, grab.y)
  await page.mouse.down()
  await page.mouse.move((grab.x + away.x) / 2, (grab.y + away.y) / 2, { steps: 5 })
  await page.mouse.move(away.x, away.y, { steps: 6 })
  await page.mouse.up()
  await page.keyboard.up('Control')
  await page.waitForTimeout(150)

  const after = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
  expect(Math.hypot(after[0] - before[0], after[1] - before[1], after[2] - before[2])).toBeLessThan(1e-6)
  expect(await selIds(page)).toEqual([id])
})
