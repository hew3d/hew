import { test, expect, type Page, type Locator } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Selection path — end to end with REAL pointer input
 * (docs/design/selection-path.md, "Decisions (2026-10-19)"). Clicking a deep
 * leaf selects the outermost group (SketchUp's drill rule); the strip at the
 * top of the viewport shows where that leaf lives (Model › Kitchen › Base
 * Cabinets › Cabinet 2 › Door) with the selection emphasized and the rest of
 * the path "below" it. Crumb clicks re-select along the path, never open a
 * session; a double-click enters; Object Info repeats the ancestry as an
 * "in A › B › C" line; and hovering a nested object shows "Door · in
 * Cabinet 2" in the inference tooltip after a short dwell.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

interface Ctx {
  vp: Mat4
  rect: { left: number; top: number; width: number; height: number }
}

interface Scene {
  door: string
  cabinet2: string
  base: string
  kitchen: string
  floor: string
}

// Front view (looking +Y). Door is the unit box at the origin; Floor sits to
// its right, so nothing occludes the Door's front face (y = 0).
const CAMERA: CameraParams = {
  position: { x: 2, y: -9, z: 1 },
  target: { x: 2, y: 0, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
}

const DOOR_FRONT: [number, number, number] = [0.5, 0, 0.5]
const FLOOR_FRONT: [number, number, number] = [4, 0, 0.1]

async function ready(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

async function build(page: Page): Promise<Scene> {
  return page.evaluate((cam) => {
    const h = window.__hew_test!
    const door = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const floor = h.drawBox([3, 0, 0], [5, 1, 0], 0.2)
    h.setNodeName('object', door, 'Door')
    h.setNodeName('object', floor, 'Floor')
    const cabinet2 = h.groupNodes([{ kind: 'object', id: door }])
    h.setNodeName('group', cabinet2, 'Cabinet 2')
    const base = h.groupNodes([{ kind: 'group', id: cabinet2 }])
    h.setNodeName('group', base, 'Base Cabinets')
    const kitchen = h.groupNodes([{ kind: 'group', id: base }])
    h.setNodeName('group', kitchen, 'Kitchen')
    h.setCamera({
      position: [cam.position.x, cam.position.y, cam.position.z],
      target: [cam.target.x, cam.target.y, cam.target.z],
      up: [cam.up.x, cam.up.y, cam.up.z],
      fovDeg: cam.fovDeg,
    })
    return { door, cabinet2, base, kitchen, floor }
  }, CAMERA)
}

async function ctxFor(page: Page): Promise<Ctx> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(CAMERA, rect.width / rect.height), rect }
}

function px(ctx: Ctx, w: [number, number, number]): { x: number; y: number } {
  const p = worldToPagePixel({ x: w[0], y: w[1], z: w[2] }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${w.join(',')}) does not project onto the canvas`)
  return p
}

async function setupScene(page: Page): Promise<{ s: Scene; ctx: Ctx }> {
  await ready(page)
  const s = await build(page)
  // The camera applies on the next frame; wait for the layout to settle.
  const ctx = await ctxFor(page)
  return { s, ctx }
}

async function clickAt(page: Page, ctx: Ctx, w: [number, number, number]): Promise<void> {
  const p = px(ctx, w)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y)
}

const getSelection = (page: Page) => page.evaluate(() => window.__hew_test!.getSelection())
const strip = (page: Page) => page.locator('[data-testid="selection-path"]')
const crumbs = (page: Page) => strip(page).locator('[data-testid="selection-path-crumb"]')
const crumb = (page: Page, label: string) =>
  strip(page).locator('[data-testid="selection-path-crumb"]', { hasText: new RegExp(`^\\s*${label}`) })
const tray = (page: Page): Locator => page.getByLabel('Tray', { exact: true })
const pathLine = (page: Page) => page.locator('[data-testid="object-info-path"]')

async function crumbLabels(page: Page): Promise<string[]> {
  return (await crumbs(page).allInnerTexts()).map((t) => t.replace(/\s*editing\s*$/, '').trim())
}

async function expectSelectionIs(page: Page, kind: string, id: string): Promise<void> {
  await expect
    .poll(async () => {
      const sel = await getSelection(page)
      return sel.length === 1 ? `${sel[0].kind}:${sel[0].id}` : `n=${sel.length}`
    })
    .toBe(`${kind}:${id}`)
}

test.describe('selection path', () => {
  test('clicking a deep Door selects the outermost group and the strip shows the full path', async ({ page }) => {
    const { s, ctx } = await setupScene(page)
    await clickAt(page, ctx, DOOR_FRONT)
    await expectSelectionIs(page, 'group', s.kitchen)

    await expect(crumbs(page)).toHaveCount(5)
    expect(await crumbLabels(page)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet 2', 'Door'])
    await expect(crumbs(page).nth(1)).toHaveAttribute('data-state', 'selected')
    for (const i of [2, 3, 4]) {
      await expect(crumbs(page).nth(i)).toHaveAttribute('data-state', 'below')
    }
    await expect(strip(page).locator('[data-editing="true"]')).toHaveCount(0)
  })

  test('clicking the Cabinet 2 crumb selects that group without opening a session', async ({ page }) => {
    const { s, ctx } = await setupScene(page)
    await clickAt(page, ctx, DOOR_FRONT)
    await expectSelectionIs(page, 'group', s.kitchen)

    await crumb(page, 'Cabinet 2').click()
    await expectSelectionIs(page, 'group', s.cabinet2)

    await expect(strip(page).locator('[data-editing="true"]')).toHaveCount(0)
    expect(await crumbLabels(page)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet 2', 'Door'])
    await expect(crumb(page, 'Cabinet 2')).toHaveAttribute('data-state', 'selected')
    await expect(crumb(page, 'Door')).toHaveAttribute('data-state', 'below')
    await expect(crumb(page, 'Kitchen')).not.toHaveAttribute('data-state', 'selected')

    // The Outliner reveals and shows the selected row.
    await expect(tray(page).getByText('Cabinet 2', { exact: true })).toBeVisible()
    await expect(pathLine(page)).toHaveText(/in Kitchen › Base Cabinets$/)
  })

  test('clicking the Door crumb selects the object; Object Info path segments select ancestors', async ({ page }) => {
    const { s, ctx } = await setupScene(page)
    await clickAt(page, ctx, DOOR_FRONT)
    await expectSelectionIs(page, 'group', s.kitchen)

    await crumb(page, 'Door').click()
    await expectSelectionIs(page, 'object', s.door)
    await expect(crumb(page, 'Door')).toHaveAttribute('data-state', 'selected')
    await expect(pathLine(page)).toHaveText(/in Kitchen › Base Cabinets › Cabinet 2$/)

    await pathLine(page).getByRole('button', { name: 'Base Cabinets', exact: true }).click()
    await expectSelectionIs(page, 'group', s.base)
    await expect(crumb(page, 'Base Cabinets')).toHaveAttribute('data-state', 'selected')
  })

  test('double-clicking the Cabinet 2 crumb opens sessions down to it; Model closes them all', async ({ page }) => {
    const { s, ctx } = await setupScene(page)
    await clickAt(page, ctx, DOOR_FRONT)
    await expectSelectionIs(page, 'group', s.kitchen)

    await crumb(page, 'Cabinet 2').dblclick()

    const editing = strip(page).locator('[data-testid="selection-path-crumb"][data-editing="true"]')
    await expect(editing).toHaveCount(3)
    await expect(editing.nth(0)).toContainText('Kitchen')
    await expect(editing.nth(1)).toContainText('Base Cabinets')
    await expect(editing.nth(2)).toContainText('Cabinet 2')

    await crumb(page, 'Model').click()
    await expect(strip(page).locator('[data-editing="true"]')).toHaveCount(0)
    // Back at the top level: the whole Kitchen is pickable as a unit again.
    await page.mouse.move(0, 0)
    await clickAt(page, ctx, DOOR_FRONT)
    await expectSelectionIs(page, 'group', s.kitchen)
  })

  test('a lone top-level object and a multi-selection show no strip', async ({ page }) => {
    const { s, ctx } = await setupScene(page)
    await clickAt(page, ctx, FLOOR_FRONT)
    await expectSelectionIs(page, 'object', s.floor)
    await expect(strip(page)).toHaveCount(0)

    await page.evaluate(
      ([k, f]) =>
        window.__hew_test!.selectNodes([
          { kind: 'group', id: k },
          { kind: 'object', id: f },
        ]),
      [s.kitchen, s.floor],
    )
    await expect.poll(async () => (await getSelection(page)).length).toBe(2)
    await expect(strip(page)).toHaveCount(0)
  })

  test('hover detail: nested object names its parent after the dwell; top-level does not; Line tool shows none', async ({ page }) => {
    const { ctx } = await setupScene(page)
    const detail = page.locator('[data-testid="inference-tip-detail"]')

    const door = px(ctx, DOOR_FRONT)
    await page.mouse.move(door.x - 6, door.y - 6)
    await page.mouse.move(door.x, door.y, { steps: 4 })
    await expect(detail).toHaveText('Door · in Cabinet 2', { timeout: 2000 })

    const floor = px(ctx, FLOOR_FRONT)
    await page.mouse.move(floor.x, floor.y, { steps: 6 })
    await expect(detail).toHaveText('Floor', { timeout: 2000 })

    await page.keyboard.press('l')
    await page.mouse.move(door.x, door.y, { steps: 6 })
    await page.waitForTimeout(700) // negative case: outlast the 400 ms dwell
    await expect(detail).toHaveCount(0)
  })

  test('press and hold on the Door hides the whole tip, detail line included', async ({ page }) => {
    const { ctx } = await setupScene(page)
    const detail = page.locator('[data-testid="inference-tip-detail"]')
    const tip = page.locator('[data-testid="inference-tip"]')

    const door = px(ctx, DOOR_FRONT)
    await page.mouse.move(door.x - 6, door.y - 6)
    await page.mouse.move(door.x, door.y, { steps: 4 })
    await expect(detail).toHaveText('Door · in Cabinet 2', { timeout: 2000 })

    await page.mouse.down()
    await page.waitForTimeout(600)
    await expect(detail).toHaveCount(0)
    await expect(tip).toHaveCount(0)
    await page.mouse.up()
  })

  test('re-clicking the already-selected group in the Outliner drops the crumbs below', async ({ page }) => {
    const { s, ctx } = await setupScene(page)
    await clickAt(page, ctx, DOOR_FRONT)
    await expectSelectionIs(page, 'group', s.kitchen)
    await expect(strip(page).locator('[data-state="below"]')).toHaveCount(3)

    await tray(page).getByText('Kitchen', { exact: true }).click()
    await expectSelectionIs(page, 'group', s.kitchen)
    await expect(strip(page).locator('[data-state="below"]')).toHaveCount(0)
  })

  test('at 1024 px the strip clears the view die and the measurement box', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 700 })
    const { s, ctx } = await setupScene(page)
    // Start a Line so the measurement box shows a live length, then select
    // the Door: the strip is at its longest with the box up.
    await page.keyboard.press('l')
    const a = px(ctx, FLOOR_FRONT)
    await page.mouse.click(a.x, a.y)
    await page.mouse.move(a.x + 120, a.y - 40, { steps: 6 })
    const box = page.locator('[data-testid="measurement-box"]')
    await expect(box).toBeVisible()
    await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'object', id }]), s.door)
    await expect(strip(page)).toBeVisible()
    await expect(box).toBeVisible()

    const stripBox = (await strip(page).boundingBox())!
    const dieBox = (await page.locator('[data-testid="view-die"]').boundingBox())!
    const vcbBox = (await box.boundingBox())!
    const overlaps = (p: typeof stripBox, q: typeof stripBox) =>
      p.x < q.x + q.width && q.x < p.x + p.width && p.y < q.y + q.height && q.y < p.y + p.height
    expect(overlaps(stripBox, dieBox)).toBe(false)
    expect(overlaps(stripBox, vcbBox)).toBe(false)
    // Five crumbs do not fit the narrow band: the strip drops to a second row,
    // which still stops short of the die's column.
    await expect(strip(page)).toHaveAttribute('data-row', 'second')
    expect(stripBox.x + stripBox.width).toBeLessThanOrEqual(dieBox.x)
  })

  test('at 1600 px the five-crumb path fits the top-center band', async ({ page }) => {
    // The side panels leave the viewport ~500 px narrower than the window, so
    // the band must have headroom for the wider fallback UI font on Linux.
    await page.setViewportSize({ width: 1600, height: 900 })
    const { s } = await setupScene(page)
    await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'object', id }]), s.door)
    await expect(crumbs(page)).toHaveCount(5)
    await expect(strip(page)).toHaveAttribute('data-row', 'band')
  })

  test('selecting from the Outliner row shows the strip with the Door selected and nothing below', async ({ page }) => {
    const { s } = await setupScene(page)
    // Groups start collapsed; expand down to the Door row via the Kitchen path.
    await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'object', id }]), s.door)
    await expect(tray(page).getByText('Door', { exact: true })).toBeVisible()
    await page.evaluate(() => window.__hew_test!.selectNodes([]))
    await expect(strip(page)).toHaveCount(0)

    await tray(page).getByText('Door', { exact: true }).click()
    await expectSelectionIs(page, 'object', s.door)
    expect(await crumbLabels(page)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet 2', 'Door'])
    await expect(crumb(page, 'Door')).toHaveAttribute('data-state', 'selected')
    await expect(strip(page).locator('[data-state="below"]')).toHaveCount(0)
  })
})
