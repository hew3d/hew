import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Grid arrays in Move: after a copy, two counts (`x4,x3`) lay the copies out
 * as a lattice in one undo step. Real pointer and keyboard events drive the
 * tools; the harness only reads the model back.
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

/** Draw a 1 x 1 x 1 board, select it, and Move-copy it 2 m along +X with real clicks. */
async function drawAndCopyBoard(page: Page, taps = 1): Promise<string> {
  const board = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), board)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('m')
  for (let i = 0; i < taps; i += 1) await tapCopyModifier(page)
  await clickWorld(page, [0.5, 0.5, 0])
  await page.keyboard.press('ArrowRight')
  await moveToWorld(page, [2.5, 0.5, 0])
  await page.keyboard.type('2')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  return board
}

/** World-space minimum corners of every placement or object, as sorted "x,y,z" strings. */
async function slotKeys(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    const out: string[] = []
    for (const id of h.getObjectIds()) {
      const b = h.getObjectBounds(id)
      out.push([b[0], b[1], b[2]].map((n) => Math.round(n)).join(','))
    }
    return out.sort()
  })
}

test.describe('grid arrays', () => {
  test('x4_3_makes_a_grid_and_undo_removes_it', async ({ page }, info) => {
    await setup(page)
    // The alternate copy kind is independent when the default copies are linked,
    // so the grid is plain objects here and the lattice reads straight off their bounds.
    await drawAndCopyBoard(page, 2)
    await page.keyboard.type('x4,x3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)

    const sel = await page.evaluate(() => window.__hew_test!.getSelection())
    expect(sel).toHaveLength(19)
    const keys = await slotKeys(page)
    expect(keys).toHaveLength(20)
    // 5 columns 2 m apart along X, 4 rows 2 m apart along Y: the original sits at a corner.
    const want: string[] = []
    for (let i = 0; i <= 4; i++) for (let j = 0; j <= 3; j++) want.push(`${2 * i},${2 * j},0`)
    expect(keys).toEqual(want.sort())
    await page.screenshot({ path: info.outputPath('grid.png') })

    // One undo takes the whole grid away.
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(150)
    expect(await slotKeys(page)).toEqual(['0,0,0'])
  })

  test('up_arrow_at_enter_puts_the_rows_on_blue_and_x40_x40_is_refused', async ({ page }) => {
    await setup(page)
    await drawAndCopyBoard(page, 2)
    await page.keyboard.type('x2,x1')
    await page.keyboard.press('ArrowUp')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    expect(await slotKeys(page)).toEqual(
      ['0,0,0', '0,0,2', '2,0,0', '2,0,2', '4,0,0', '4,0,2'].sort(),
    )

    // A product over the cap changes nothing.
    await page.keyboard.type('x40,x40')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    expect(await slotKeys(page)).toHaveLength(6)
  })
})
