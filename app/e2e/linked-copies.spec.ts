import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Linked copies by default: a copy made with Move shares its shape with the
 * original, so editing one changes both; Unlink makes one independent;
 * Subtract on one copy cuts every copy. Real pointer and keyboard events
 * drive the tools and menus; the harness only reads the model back.
 */

const CAMERA = {
  position: [9, -9, 7] as [number, number, number],
  target: [3, 1, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

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

/** The copy modifier is Option on macOS and a Control clean tap elsewhere. */
async function tapCopyModifier(page: Page): Promise<void> {
  const mac = await page.evaluate(() => /Mac|iPod|iPhone|iPad/.test(navigator.platform))
  await page.keyboard.press(mac ? 'Alt' : 'Control')
}

async function boundsOf(page: Page, id: string): Promise<[number, number, number, number, number, number]> {
  return page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
}

/** Draw a 2 x 2 x 1 board, select it, and Move-copy it 4 m along +X with real clicks. */
async function drawAndCopyBoard(page: Page): Promise<{ board: string; copy: string; def: string }> {
  const board = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), board)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('m')
  await tapCopyModifier(page)
  await clickWorld(page, [1, 1, 0]) // base point
  await page.keyboard.press('ArrowRight') // lock X
  await moveToWorld(page, [5, 1, 0])
  await page.keyboard.type('4')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel).toHaveLength(1)
  expect(sel[0].kind).toBe('instance')
  const copy = sel[0].id
  const def = await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), copy)
  expect(def).not.toBeNull()
  return { board, copy, def: def! }
}

async function memberHeight(page: Page, def: string): Promise<number> {
  const members = await page.evaluate((d) => window.__hew_test!.getComponentMemberObjects(d), def)
  expect(members).toHaveLength(1)
  const b = await boundsOf(page, members[0])
  return b[5] - b[2]
}

async function chooseMenu(page: Page, menu: RegExp, item: string): Promise<void> {
  await page.getByRole('button', { name: menu }).click()
  await page.getByText(item, { exact: true }).first().dispatchEvent('mousedown')
}

test.describe('linked copies', () => {
  test('copy_a_board_with_move_then_push_pull_the_copy_changes_both', async ({ page }, info) => {
    await setup(page)
    const { copy, def } = await drawAndCopyBoard(page)
    // Two placements of one shape: the original stays on screen and follows.
    const placements = await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)
    expect(placements).toHaveLength(2)
    expect(await memberHeight(page, def)).toBeCloseTo(1, 6)
    await page.screenshot({ path: info.outputPath('copy-made.png') })

    // Push/Pull the top face of the copy: no double-click, no mode to enter.
    await page.keyboard.press('p')
    await clickWorld(page, [5, 1, 1])
    await page.keyboard.type('0.5')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(150)
    expect(await memberHeight(page, def)).toBeCloseTo(1.5, 5)
    expect(await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)).toHaveLength(2)
    expect(copy).toBeTruthy()
    await page.screenshot({ path: info.outputPath('both-pushed.png') })

    // One undo removes the push, the next the copy; the original is a plain
    // loose board again.
    await page.evaluate(() => window.__hew_test!.undo())
    expect(await memberHeight(page, def)).toBeCloseTo(1, 6)
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
  })

  test('unlink_one_then_push_pull_changes_only_that_one', async ({ page }, info) => {
    await setup(page)
    const { copy, def } = await drawAndCopyBoard(page)
    expect(await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)).toHaveLength(2)

    // Make Unique and Place Copy speak of components; a linked copy has its
    // own actions, so they do nothing here.
    await page.getByRole('button', { name: /^object$/i }).click()
    await page.getByText('Make Unique', { exact: true }).first().dispatchEvent('mousedown')
    await page.getByText('Place Copy', { exact: true }).first().dispatchEvent('mousedown')
    await page.getByRole('button', { name: /^object$/i }).click() // close it again
    await page.waitForTimeout(100)
    expect(await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), copy)).toBe(def)
    expect(await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)).toHaveLength(2)

    // Object ▸ Unlink on the selected copy.
    await chooseMenu(page, /^object$/i, 'Unlink')
    await page.waitForTimeout(150)
    const copyDef = await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), copy)
    expect(copyDef).not.toBe(def)
    expect(await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)).toHaveLength(1)
    // It is still the selected thing.
    expect((await page.evaluate(() => window.__hew_test!.getSelection()))[0].id).toBe(copy)

    await page.keyboard.press('p')
    await clickWorld(page, [5, 1, 1])
    await page.keyboard.type('0.5')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(150)
    expect(await memberHeight(page, copyDef!)).toBeCloseTo(1.5, 5)
    expect(await memberHeight(page, def)).toBeCloseTo(1, 6) // the original did not change
    await page.screenshot({ path: info.outputPath('unlinked-one-pushed.png') })
  })

  test('subtract_a_mortise_from_one_of_four_legs_shows_in_all_four', async ({ page }, info) => {
    await setup(page)
    // A leg, copied three times by typing x3 after the first copy.
    const leg = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 3))
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), leg)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
    await page.keyboard.press('m')
    await tapCopyModifier(page)
    await clickWorld(page, [0.5, 0.5, 0])
    await page.keyboard.press('ArrowRight')
    await moveToWorld(page, [2.5, 0.5, 0])
    await page.keyboard.type('2')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    await page.keyboard.type('x3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(200)
    const copies = await page.evaluate(() => window.__hew_test!.getSelection())
    expect(copies).toHaveLength(3)
    const def = (await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), copies[0].id))!
    expect(await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)).toHaveLength(4)
    expect(await memberHeight(page, def)).toBeCloseTo(3, 6)

    // A cutter across the first leg's top, drawn as a loose solid.
    const cutter = await page.evaluate(() => {
      const id = window.__hew_test!.drawBox([1.75, -0.25, 0], [3.25, 1.25, 0], 2)
      window.__hew_test!.moveObject(id, 0, 0, 2)
      return id
    })
    await page.evaluate(
      ([host, tool]) =>
        window.__hew_test!.selectNodes([
          { kind: 'instance', id: host },
          { kind: 'object', id: tool },
        ]),
      [copies[0].id, cutter],
    )
    await chooseMenu(page, /^object$/i, 'Subtract')
    // The scope strip asks how far the cut reaches; Enter takes the default.
    await expect(page.getByRole('button', { name: 'Cut all 4 copies' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Only this one' })).toBeVisible()
    await page.screenshot({ path: info.outputPath('scope-strip.png') })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(200)
    // Every copy shows the cut: one shared shape, now 2 m tall.
    expect(await memberHeight(page, def)).toBeCloseTo(2, 5)
    expect(await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def)).toHaveLength(4)
    await page.screenshot({ path: info.outputPath('all-four-cut.png') })
  })

  test('only_this_one_cuts_a_single_leg_and_unlinks_it', async ({ page }) => {
    await setup(page)
    const { copy, def } = await drawAndCopyBoard(page)
    const cutter = await page.evaluate(() => {
      const id = window.__hew_test!.drawBox([3.5, -0.5, 0], [6.5, 2.5, 0], 1)
      window.__hew_test!.moveObject(id, 0, 0, 0.5)
      return id
    })
    await page.evaluate(
      ([host, tool]) =>
        window.__hew_test!.selectNodes([
          { kind: 'instance', id: host },
          { kind: 'object', id: tool },
        ]),
      [copy, cutter],
    )
    await chooseMenu(page, /^object$/i, 'Subtract')
    await page.getByRole('button', { name: 'Only this one' }).click()
    await page.waitForTimeout(200)
    const copyDef = await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), copy)
    expect(copyDef).not.toBe(def)
    expect(await memberHeight(page, copyDef!)).toBeCloseTo(0.5, 5)
    expect(await memberHeight(page, def)).toBeCloseTo(1, 6)
    await expect(page.getByText('Unlinked this copy and cut it')).toBeVisible()
  })

  test('undo_of_a_copy_restores_the_loose_board', async ({ page }) => {
    await setup(page)
    const { board } = await drawAndCopyBoard(page)
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(150)
    const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toEqual([board])
  })

  test('tapping the copy key cycles Copy, Independent copy, and Move', async ({ page }) => {
    await setup(page)
    const board = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), board)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
    await page.keyboard.press('m')
    await clickWorld(page, [1, 1, 0])
    await tapCopyModifier(page)
    await expect(page.getByText(/^Copy · /)).toBeVisible()
    await tapCopyModifier(page)
    await expect(page.getByText(/^Independent copy · /)).toBeVisible()
    await tapCopyModifier(page)
    await expect(page.getByText(/^(Copy|Independent copy) · /)).toHaveCount(0)
  })
})
