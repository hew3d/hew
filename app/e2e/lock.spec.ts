import { test, expect, type Page } from '@playwright/test'
import { outlinerEditingChips } from './helpers/outliner'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Lock (OW-1), driven with REAL pointer and keyboard events and the real
 * Edit menu: a locked thing refuses Move, Push/Pull, Delete and
 * double-click with the hint beside the cursor and a toast that offers
 * Unlock; the lock belongs to the shared shape (copies lock together, Make
 * Unique is the way out); locked items are left behind by a selection
 * command; and undo goes back through the lock. The harness only reads.
 */

const CAMERA = {
  position: [9, -9, 7] as [number, number, number],
  target: [3, 1, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

const LOCKED_LINE = 'Locked. Unlock it to change it.'

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  await page.waitForTimeout(100)
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function clickWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

async function moveToWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y)
}

async function chooseMenu(page: Page, menu: RegExp, item: string): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: menu }).click()
  await page.getByText(item, { exact: true }).first().dispatchEvent('mousedown')
}

/** Edit ▸ Unlock ▸ Selected / All. */
async function unlockFromMenu(page: Page, which: 'Selected' | 'All'): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: /^edit$/i }).click()
  await page.getByText('Unlock', { exact: true }).first().hover()
  await page.getByText(which, { exact: true }).first().dispatchEvent('mousedown')
}

async function box(page: Page, x0: number, x1: number): Promise<string> {
  return page.evaluate(([a, b]) => window.__hew_test!.drawBox([a, 0, 0], [b, 2, 0], 1), [x0, x1])
}

async function select(page: Page, ids: string[]): Promise<void> {
  await page.evaluate((i) => window.__hew_test!.selectObjects(i), ids)
  await page.waitForFunction((n) => window.__hew_test!.getSelection().length === n, ids.length)
}

const bounds = (page: Page, id: string) => page.evaluate((i) => window.__hew_test!.getObjectBounds(i), id)
const isLocked = (page: Page, kind: string, id: string) =>
  page.evaluate(([k, i]) => window.__hew_test!.isNodeLocked(k, i), [kind, id])

/** The toast's own message line (the cursor hint is a div, so a span filter leaves it out). */
const toastLine = (page: Page, text: string) => page.locator('span', { hasText: new RegExp(`^${text.replace(/[.]/g, '\\.')}$`) }).first()

test.describe('lock', () => {
  test('a_locked_box_refuses_move_push_pull_delete_and_double_click_with_the_hint', async ({ page }, info) => {
    await setup(page)
    const id = await box(page, 0, 2)
    await select(page, [id])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(true)
    const before = await bounds(page, id)

    // Move: the click that would pick it up is refused, with the Unlock button.
    await page.keyboard.press('m')
    await clickWorld(page, [1, 1, 1])
    await expect(toastLine(page, LOCKED_LINE)).toBeVisible()
    await expect(page.locator('[data-toast-action]', { hasText: 'Unlock' })).toBeVisible()
    await page.screenshot({ path: info.outputPath('move-refused.png') })

    // Push/Pull: the hint sits beside the cursor before any click; the click toasts it.
    await page.keyboard.press('p')
    await moveToWorld(page, [1, 1, 1])
    await expect(page.getByTestId('inference-tip-hint')).toHaveText(LOCKED_LINE)
    await clickWorld(page, [1, 1, 1])
    await expect(toastLine(page, LOCKED_LINE)).toBeVisible()
    await page.keyboard.press('Escape')

    // Delete (Edit menu): refused with the toast, the box is still there.
    await chooseMenu(page, /^edit$/i, 'Delete')
    await expect(toastLine(page, LOCKED_LINE)).toBeVisible()
    expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)

    // Double-click to open for editing: refused, nothing opens.
    await page.keyboard.press('Space')
    const p = await pagePoint(page, [1, 1, 1])
    await page.mouse.dblclick(p.x, p.y)
    await expect(toastLine(page, LOCKED_LINE)).toBeVisible()
    expect(await page.evaluate(() => window.__hew_test!.getSessionPlacements())).toEqual([])

    expect(await bounds(page, id)).toEqual(before)
  })

  test('unlock_from_the_toast_then_undo_goes_back_through_the_lock', async ({ page }) => {
    await setup(page)
    const id = await box(page, 0, 2)
    await select(page, [id])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(true)
    await page.keyboard.press('m')
    await clickWorld(page, [1, 1, 1])
    await page.locator('[data-toast-action]', { hasText: 'Unlock' }).click()
    await expect.poll(() => isLocked(page, 'object', id)).toBe(false)
    // One undo takes back the Unlock, the next the Lock.
    await page.evaluate(() => window.__hew_test!.undo())
    expect(await isLocked(page, 'object', id)).toBe(true)
    await page.evaluate(() => window.__hew_test!.undo())
    expect(await isLocked(page, 'object', id)).toBe(false)
  })

  test('marquee_a_locked_and_an_unlocked_box_then_move_only_the_unlocked_one_moves', async ({ page }) => {
    await setup(page)
    const locked = await box(page, 0, 2)
    const free = await box(page, 4, 6)
    await select(page, [locked])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', locked)).toBe(true)
    await page.evaluate(() => window.__hew_test!.selectObjects([]))
    const lockedBefore = await bounds(page, locked)
    const freeBefore = await bounds(page, free)

    // A real window drag around both boxes with the Select tool, from outside
    // the hull of their screen corners to outside the opposite corner.
    await page.keyboard.press('Space')
    const xs: number[] = []
    const ys: number[] = []
    for (const x of [0, 6]) for (const y of [0, 2]) for (const z of [0, 1]) {
      const q = await pagePoint(page, [x, y, z])
      xs.push(q.x)
      ys.push(q.y)
    }
    const a = { x: Math.min(...xs) - 30, y: Math.min(...ys) - 30 }
    const b = { x: Math.max(...xs) + 30, y: Math.max(...ys) + 30 }
    await page.mouse.move(a.x, a.y)
    await page.mouse.down()
    await page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2, { steps: 6 })
    await page.mouse.move(b.x, b.y, { steps: 6 })
    await page.mouse.up()
    await expect.poll(() => page.evaluate(() => window.__hew_test!.getSelection().length)).toBe(2)

    // Move the selection 3 m along X with real clicks and a typed distance.
    await page.keyboard.press('m')
    await clickWorld(page, [5, 1, 1])
    await page.keyboard.press('ArrowRight')
    await moveToWorld(page, [8, 1, 1])
    await page.keyboard.type('3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(150)
    await expect(toastLine(page, '1 locked item was left alone.')).toBeVisible()
    expect(await bounds(page, locked)).toEqual(lockedBefore)
    const freeAfter = await bounds(page, free)
    expect(freeAfter[0] - freeBefore[0]).toBeCloseTo(3, 5)
  })

  test('copies_lock_together_make_unique_is_the_way_out_and_the_other_three_stay_locked', async ({ page }, info) => {
    await setup(page)
    const first = await box(page, 0, 2)
    // Three linked copies by Move-copy with real clicks: each new copy is selected for the next.
    await select(page, [first])
    const mac = await page.evaluate(() => /Mac|iPod|iPhone|iPad/.test(navigator.platform))
    for (let n = 1; n <= 3; n++) {
      await page.keyboard.press('m')
      await page.keyboard.press(mac ? 'Alt' : 'Control')
      await clickWorld(page, [1 + 3 * (n - 1), 1, 0])
      await page.keyboard.press('ArrowRight')
      await moveToWorld(page, [1 + 3 * n, 1, 0])
      await page.keyboard.type('3')
      await page.keyboard.press('Enter')
      await page.waitForTimeout(150)
    }
    const sel = await page.evaluate(() => window.__hew_test!.getSelection())
    expect(sel).toHaveLength(1)
    expect(sel[0].kind).toBe('instance')
    const def = (await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), sel[0].id))!
    const placements = await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)
    expect(placements).toHaveLength(4)

    // Lock one copy: all four show the padlock, and all four report locked.
    await chooseMenu(page, /^edit$/i, 'Lock')
    for (const p of placements) await expect.poll(() => isLocked(page, 'instance', p)).toBe(true)
    await expect(page.getByTestId('outliner-lock')).toHaveCount(4)
    await page.screenshot({ path: info.outputPath('four-locked.png') })

    // The copy 6 m along (third from the left, in view) is the one we work with.
    const poses = await Promise.all(placements.map((p) => page.evaluate((i) => window.__hew_test!.getInstancePose(i), p)))
    const target = placements[poses.findIndex((m) => m !== null && Math.abs(m[3] - 6) < 1e-6)]
    expect(target).toBeTruthy()
    const topFace: [number, number, number] = [7, 1, 1]

    // Push/Pull on a locked copy: the hint names the set; the toast offers Unlock and Make Unique.
    await page.keyboard.press('p')
    await moveToWorld(page, topFace)
    await expect(page.getByTestId('inference-tip-hint')).toHaveText('Locked, along with its 3 copies.')
    await clickWorld(page, topFace)
    await expect(page.locator('[data-toast-action]')).toHaveText(['Unlock', 'Make Unique'])
    await page.locator('[data-toast-action]', { hasText: 'Make Unique' }).click()

    // The new object stays locked with a lock of its own; the other three stay locked too.
    const members = (id: string) => page.evaluate((d) => window.__hew_test!.getInstancesOf(d), id)
    await expect.poll(async () => (await members(def)).length).toBe(3)
    for (const p of placements) await expect.poll(() => isLocked(page, 'instance', p)).toBe(true)
    await expect(page.getByTestId('outliner-lock')).toHaveCount(4)

    // Unlock the unique one (Edit ▸ Unlock ▸ Selected): three padlocks remain.
    await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), target)
    await unlockFromMenu(page, 'Selected')
    await expect.poll(() => isLocked(page, 'instance', target)).toBe(false)
    const remaining = await members(def)
    expect(remaining).toHaveLength(3)
    for (const p of remaining) expect(await isLocked(page, 'instance', p)).toBe(true)
    await expect(page.getByTestId('outliner-lock')).toHaveCount(3)

    // Push/Pull the now-free copy: it grows; the shared three are unchanged.
    const uniqueDef = (await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), target))!
    const heightOf = async (d: string) => {
      const m = await page.evaluate((x) => window.__hew_test!.getComponentMemberObjects(x), d)
      const b = await bounds(page, m[0])
      return b[5] - b[2]
    }
    expect(await heightOf(uniqueDef)).toBeCloseTo(1, 6)
    await page.keyboard.press('p')
    await clickWorld(page, topFace)
    await page.keyboard.type('0.5')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(150)
    await expect.poll(() => heightOf(uniqueDef)).toBeCloseTo(1.5, 5)
    expect(await heightOf(def)).toBeCloseTo(1, 6)
    await expect(page.getByTestId('outliner-lock')).toHaveCount(3)
  })

  test('the_outliner_double_click_refuses_a_locked_group_and_a_locked_member_like_the_viewport', async ({ page }) => {
    await setup(page)
    const ids = await page.evaluate(() => {
      const h = window.__hew_test!
      const a = h.drawBox([0, 0, 0], [2, 2, 0], 1)
      const b = h.drawBox([4, 0, 0], [6, 2, 0], 1)
      const g = h.groupNodes([{ kind: 'object', id: a }, { kind: 'object', id: b }])
      h.setNodeName('group', g, 'Shed')
      return { a, b, g }
    })
    // A locked group: its row refuses to open, with the toast and no session.
    await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'group', id }]), ids.g)
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'group', ids.g)).toBe(true)
    await page.getByText('Shed').first().dblclick()
    await expect(toastLine(page, LOCKED_LINE)).toBeVisible()
    await expect(page.locator('[data-toast-action]', { hasText: 'Unlock' }).first()).toBeVisible()
    await expect(outlinerEditingChips(page)).toHaveCount(0)
    expect(await page.evaluate(() => window.__hew_test!.getSessionPlacements())).toEqual([])

    // Unlock the group, lock one member: its row refuses too, and the group stays closed
    // (no half-entered object context).
    await unlockFromMenu(page, 'All')
    await expect.poll(() => isLocked(page, 'group', ids.g)).toBe(false)
    await select(page, [ids.a])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', ids.a)).toBe(true)
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), ids.a)
    await page.getByTestId('outliner-row-label').filter({ hasText: 'Object 1' }).first().dblclick()
    await expect(toastLine(page, LOCKED_LINE)).toBeVisible()
    await expect(outlinerEditingChips(page)).toHaveCount(0)
  })

  test('unlock_all_works_while_a_component_is_open_and_group_says_so_when_too_little_is_left', async ({ page }) => {
    await setup(page)
    const locked = await box(page, 0, 2)
    const part = await box(page, 4, 6)
    const free = await box(page, 8, 10)
    const made = await page.evaluate((id) => window.__hew_test!.makeComponent([id]), part)
    await select(page, [locked])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', locked)).toBe(true)

    // Group with one locked and one free box: nothing to group, and the toast says so.
    await select(page, [locked, free])
    await chooseMenu(page, /^object$/i, 'Group')
    await expect(page.locator('span', { hasText: '1 locked item was left alone. Grouping needs two or more, so nothing was grouped.' }).first()).toBeVisible()
    expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBeGreaterThanOrEqual(2)

    // Open the component for editing: Lock is off, Unlock ▸ All still works.
    await page.keyboard.press('Space')
    const p = await pagePoint(page, [5, 1, 1])
    await page.mouse.dblclick(p.x, p.y)
    await expect(outlinerEditingChips(page)).toHaveCount(1)
    expect(made.instance).toBeTruthy()
    await unlockFromMenu(page, 'All')
    await expect.poll(() => isLocked(page, 'object', locked)).toBe(false)
  })

  test('move_copy_of_a_locked_box_makes_a_locked_linked_pair_and_the_original_stays_put', async ({ page }) => {
    await setup(page)
    const id = await box(page, 0, 2)
    await select(page, [id])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(true)
    const mac = await page.evaluate(() => /Mac|iPod|iPhone|iPad/.test(navigator.platform))

    // Move, tap the copy modifier, click the locked box: it is not refused.
    await page.keyboard.press('m')
    await page.keyboard.press(mac ? 'Alt' : 'Control')
    await clickWorld(page, [1, 1, 0])
    await page.keyboard.press('ArrowRight')
    await moveToWorld(page, [4, 1, 0])
    await page.keyboard.type('3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(150)
    await expect(toastLine(page, LOCKED_LINE)).toHaveCount(0)

    // A linked pair that joined the lock; the original did not move.
    const sel = await page.evaluate(() => window.__hew_test!.getSelection())
    expect(sel).toHaveLength(1)
    expect(sel[0].kind).toBe('instance')
    const def = (await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), sel[0].id))!
    const placements = await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)
    expect(placements).toHaveLength(2)
    for (const p of placements) expect(await isLocked(page, 'instance', p)).toBe(true)
    const xs = (await Promise.all(placements.map((p) => page.evaluate((i) => window.__hew_test!.getInstancePose(i), p))))
      .map((m) => (m === null ? NaN : m[3]))
      .sort((a, b) => a - b)
    expect(xs[0]).toBeCloseTo(0, 6)
    expect(xs[1]).toBeCloseTo(3, 6)
  })

  test('a_refused_move_click_swallows_the_next_typed_digits_instead_of_switching_tools', async ({ page }) => {
    await setup(page)
    const id = await box(page, 0, 2)
    await select(page, [id])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(true)
    await page.keyboard.press('m')
    await clickWorld(page, [1, 1, 1])
    await expect(toastLine(page, LOCKED_LINE)).toBeVisible()
    // 3 is the Push/Pull quick pick: with it live, hovering the locked box
    // would put the Locked line beside the cursor. Move is still the tool.
    await page.keyboard.type('3')
    await moveToWorld(page, [1, 1, 1])
    await page.waitForTimeout(300)
    await expect(page.getByTestId('inference-tip-hint')).toHaveCount(0)
  })

  test('cut_with_a_locked_and_a_free_box_takes_only_the_free_one_to_the_clipboard', async ({ page }) => {
    await setup(page)
    const locked = await box(page, 0, 2)
    const free = await box(page, 4, 6)
    await select(page, [locked])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', locked)).toBe(true)
    await select(page, [locked, free])
    await chooseMenu(page, /^edit$/i, 'Cut')
    await expect(toastLine(page, '1 locked item was left alone.')).toBeVisible()
    await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
    // Paste in place: the free box comes back, and nothing else (no copy of the locked one).
    await chooseMenu(page, /^edit$/i, 'Paste In Place')
    await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(2)
    await page.waitForTimeout(300)
    expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(2)
  })

  test('lock_from_the_palette_while_a_component_is_open_says_why_it_cannot', async ({ page }) => {
    await setup(page)
    const part = await box(page, 4, 6)
    const free = await box(page, 8, 10)
    await page.evaluate((id) => window.__hew_test!.makeComponent([id]), part)
    await page.keyboard.press('Space')
    const p = await pagePoint(page, [5, 1, 1])
    await page.mouse.dblclick(p.x, p.y)
    await expect(outlinerEditingChips(page)).toHaveCount(1)
    await select(page, [free])
    await page.keyboard.press('ControlOrMeta+/')
    await page.getByPlaceholder(/search tools, actions, help/i).fill('lock the selection')
    await page.getByRole('option').filter({ hasText: /^Lock/ }).first().click()
    await expect(page.locator('span', { hasText: /open for editing/ }).first()).toBeVisible()
    expect(await isLocked(page, 'object', free)).toBe(false)
  })

  test('unlock_all_in_the_palette_says_nothing_is_locked_when_nothing_is', async ({ page }) => {
    await setup(page)
    await box(page, 0, 2)
    await page.keyboard.press('ControlOrMeta+/')
    await page.getByPlaceholder(/search tools, actions, help/i).fill('unlock all')
    const row = page.getByRole('option').filter({ hasText: 'Unlock All' }).first()
    await expect(row).toContainText('nothing is locked')
    await expect(row).not.toContainText('needs a selection')
  })
  // Drawing on a locked Object's face: the kernel opens the draft detached
  // by itself, so the drawing is a free sketch that survives the unlock and
  // pushes into a NEW Object (the blue cue), never into the box.
  test('LK25_a_rectangle_drawn_on_a_locked_top_face_is_a_free_shape_that_pushes_as_a_new_object_after_unlock', async ({ page }) => {
    await setup(page)
    const id = await box(page, 0, 2)
    await select(page, [id])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(true)
    const verts = (await page.evaluate((i) => window.__hew_test!.getObjectVertices(i), id)).length
    const before = await bounds(page, id)

    await page.getByRole('radio', { name: 'Rectangle' }).click()
    await clickWorld(page, [0.5, 0.5, 1])
    await moveToWorld(page, [1, 1, 1])
    await moveToWorld(page, [1.5, 1.5, 1])
    await clickWorld(page, [1.5, 1.5, 1])
    await expect(page.locator('span', { hasText: /locked/i })).toHaveCount(0)
    expect(await page.evaluate(() => window.__hew_test!.getSketchIds().length)).toBe(1)
    expect((await page.evaluate((i) => window.__hew_test!.getObjectVertices(i), id)).length).toBe(verts)
    expect(await bounds(page, id)).toEqual(before)

    await select(page, [id])
    await unlockFromMenu(page, 'Selected')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(false)
    await page.getByRole('radio', { name: 'Push/Pull' }).click()
    await moveToWorld(page, [0.8, 0.8, 1])
    await moveToWorld(page, [1, 1, 1])
    await page.waitForTimeout(150)
    expect(await page.evaluate(() => window.__hew_test!.getFaceCue())).toMatchObject({ kind: 'region', color: '--preview-extrude-new' })
    expect((await page.evaluate((i) => window.__hew_test!.getObjectVertices(i), id)).length).toBe(verts)
  })

  test('LK25b_a_line_loop_on_a_locked_top_face_is_one_sketch_with_one_region_and_pushes_as_a_new_object', async ({ page }) => {
    await setup(page)
    const id = await box(page, 0, 2)
    await select(page, [id])
    await chooseMenu(page, /^edit$/i, 'Lock')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(true)
    const verts = (await page.evaluate((i) => window.__hew_test!.getObjectVertices(i), id)).length
    const depth0 = await page.evaluate(() => window.__hew_test!.getUndoDepth())

    await page.getByRole('radio', { name: 'Line' }).click()
    const corners: Array<[number, number, number]> = [[0.5, 0.5, 1], [1.5, 0.5, 1], [1.5, 1.5, 1], [0.5, 1.5, 1], [0.5, 0.5, 1]]
    for (const c of corners) {
      await moveToWorld(page, c)
      await clickWorld(page, c)
    }
    await expect.poll(() => page.evaluate(() => window.__hew_test!.getSketchIds().length)).toBe(1)
    const sketch = (await page.evaluate(() => window.__hew_test!.getSketchIds()))[0]
    expect(await page.evaluate((k) => window.__hew_test!.getSketchRegionCount(k), sketch)).toBe(1)
    expect((await page.evaluate((i) => window.__hew_test!.getObjectVertices(i), id)).length).toBe(verts)

    const depthLoop = await page.evaluate(() => window.__hew_test!.getUndoDepth())
    expect(depthLoop - depth0).toBe(4)

    // The blue cue while the box is still locked, and again after the unlock.
    const cueAt = async () => {
      await moveToWorld(page, [1.1, 1.1, 1])
      await moveToWorld(page, [1, 1, 1])
      await page.waitForTimeout(150)
      return page.evaluate(() => window.__hew_test!.getFaceCue())
    }
    await page.getByRole('radio', { name: 'Push/Pull' }).click()
    expect(await cueAt()).toMatchObject({ kind: 'region', color: '--preview-extrude-new' })
    await select(page, [id])
    await unlockFromMenu(page, 'Selected')
    await expect.poll(() => isLocked(page, 'object', id)).toBe(false)
    expect(await cueAt()).toMatchObject({ kind: 'region', color: '--preview-extrude-new' })

    // The pull makes a NEW Object; the box itself is untouched.
    await clickWorld(page, [1, 1, 1])
    await moveToWorld(page, [1, 1, 1.1])
    await page.keyboard.type('0.5')
    await page.keyboard.press('Enter')
    await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(2)
    expect(await bounds(page, id)).toEqual([0, 0, 0, 2, 2, 1])
    expect((await page.evaluate((i) => window.__hew_test!.getObjectVertices(i), id)).length).toBe(verts)

    // Undo takes back the push in one step, the unlock in one, then the loop
    // one segment per step (a Line chain commits segment by segment).
    await page.evaluate(() => window.__hew_test!.undo())
    expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
    expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBe(depthLoop + 1)
    await page.evaluate(() => window.__hew_test!.undo())
    expect(await isLocked(page, 'object', id)).toBe(true)
    expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBe(depthLoop)
    await page.evaluate(() => window.__hew_test!.undo())
    expect(await page.evaluate((k) => window.__hew_test!.getSketchRegionCount(k), sketch)).toBe(0)
    for (let i = 0; i < 3; i++) await page.evaluate(() => window.__hew_test!.undo())
    expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBe(depth0)
    expect(await page.evaluate(() => window.__hew_test!.getSketchIds().length)).toBe(0)
    expect(await isLocked(page, 'object', id)).toBe(true)
  })
})
