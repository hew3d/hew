import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Flip: the tool's squares mirror the selection across a plane through its
 * center in one undo step, and with the copy toggle on it places a mirrored
 * linked copy. Real pointer and keyboard events drive the tool; the harness
 * only builds the starting boards and reads the model back.
 */

const CAMERA = {
  position: [9, -9, 7] as [number, number, number],
  target: [2, 1, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

type Bounds = [number, number, number, number, number, number]

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

/** Hover a world point, then click it there. */
async function hoverAndClick(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x - 4, p.y - 4)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(50)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
}

/** The copy modifier is Option on macOS and a Control clean tap elsewhere. */
async function tapCopyModifier(page: Page): Promise<void> {
  const mac = await page.evaluate(() => /Mac|iPod|iPhone|iPad/.test(navigator.platform))
  await page.keyboard.press(mac ? 'Alt' : 'Control')
}

async function boundsOf(page: Page, id: string): Promise<Bounds> {
  return page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
}

async function pickFlip(page: Page): Promise<void> {
  await page.getByRole('radio', { name: 'Flip', exact: true }).click()
  await page.waitForTimeout(100)
}

function det(p: number[]): number {
  return p[0] * (p[5] * p[10] - p[6] * p[9]) - p[1] * (p[4] * p[10] - p[6] * p[8]) + p[2] * (p[4] * p[9] - p[5] * p[8])
}

test.describe('flip', () => {
  test('flip_a_board_across_the_blue_plane_and_undo', async ({ page }, info) => {
    await setup(page)
    // A short board and a tall one: together they span z in [0, 2], so the
    // blue plane through their center is z = 1.
    const short = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
    const tall = await page.evaluate(() => window.__hew_test!.drawBox([2, 0, 0], [3, 1, 0], 2))
    await page.evaluate((ids) => window.__hew_test!.selectObjects(ids), [short, tall])
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 2)
    await pickFlip(page)
    const hash = await page.evaluate(() => window.__hew_test!.getStateHash())
    await page.screenshot({ path: info.outputPath('before.png') })

    // The blue square at z = 1, on the camera's side of the other two.
    await hoverAndClick(page, [2.8, 0.1, 1])
    const b = await boundsOf(page, short)
    expect(b[2]).toBeCloseTo(1, 9)
    expect(b[5]).toBeCloseTo(2, 9)
    const t = await boundsOf(page, tall)
    expect([t[2], t[5]]).toEqual([0, 2])
    expect(await page.evaluate((id) => window.__hew_test!.isObjectSolid(id), short)).toBe(true)
    // The flipped boards stay selected.
    expect(await page.evaluate(() => window.__hew_test!.getSelection().length)).toBe(2)
    await page.screenshot({ path: info.outputPath('after.png') })

    // One undo restores the document exactly.
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    expect(await page.evaluate(() => window.__hew_test!.getStateHash())).toBe(hash)
    const back = await boundsOf(page, short)
    expect([back[2], back[5]]).toEqual([0, 1])
  })

  test('mirrored_copy_is_a_linked_placement_with_a_mirrored_pose', async ({ page }, info) => {
    await setup(page)
    // An L-shaped board: x in [0, 4], y in [0, 3], the notch at (2..4, 1..3).
    const board = await page.evaluate(() => {
      const h = window.__hew_test!
      const { sketch, regions } = h.drawLineChain([
        [0, 0, 0], [4, 0, 0], [4, 1, 0], [2, 1, 0], [2, 3, 0], [0, 3, 0], [0, 0, 0],
      ])
      return h.extrudeRegion(sketch, regions[0], 0.5)
    })
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), board)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
    await pickFlip(page)
    await tapCopyModifier(page)
    await page.screenshot({ path: info.outputPath('copy-armed.png') })

    // The red square at x = 2, below the green and above the blue planes'
    // crossings as seen from the camera.
    await hoverAndClick(page, [2, 0.5, 0.5])
    const sel = await page.evaluate(() => window.__hew_test!.getSelection())
    expect(sel).toHaveLength(1)
    expect(sel[0].kind).toBe('instance')
    const copy = sel[0].id
    const pose = await page.evaluate((c) => window.__hew_test!.getInstancePose(c), copy)
    expect(pose).not.toBeNull()
    expect(det(pose!)).toBeLessThan(0)
    const def = await page.evaluate((c) => window.__hew_test!.getInstanceDef(c), copy)
    expect(def).not.toBeNull()
    const placements = await page.evaluate((d) => window.__hew_test!.getInstancesOf(d), def!)
    expect(placements).toHaveLength(2)
    expect(await page.evaluate(() => window.__hew_test!.getObjectIds())).toHaveLength(0)
    await page.screenshot({ path: info.outputPath('mirrored-copy.png') })

    // Push/Pull the copy's top: the shared shape changes, so both do.
    await page.keyboard.press('p')
    await hoverAndClick(page, [1, 0.5, 0.5])
    await page.keyboard.type('0.3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(150)
    const members = await page.evaluate((d) => window.__hew_test!.getComponentMemberObjects(d), def!)
    expect(members).toHaveLength(1)
    const mb = await boundsOf(page, members[0])
    expect(mb[5] - mb[2]).toBeCloseTo(0.8, 5)
    await page.screenshot({ path: info.outputPath('both-pushed.png') })

    // Undo the push, then the copy: the loose board is back.
    await page.evaluate(() => window.__hew_test!.undo())
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    expect(await page.evaluate(() => window.__hew_test!.getObjectIds())).toEqual([board])
  })
})
