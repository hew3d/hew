import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * The unified selection (issue 106, part 1): nodes, construction guides and
 * annotations in one ordered selection, picked with real clicks and
 * modifier clicks, and deleted together as ONE undo step whose name counts
 * what went.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 8, y: 6, z: 8 },
  target: { x: 1, y: 1, z: 0 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

interface Ctx {
  vp: Mat4
  rect: { left: number; top: number; width: number; height: number }
}

async function setup(page: Page): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(
    (c) =>
      window.__hew_test!.setCamera({
        position: [c.position.x, c.position.y, c.position.z],
        target: [c.target.x, c.target.y, c.target.z],
        up: [c.up.x, c.up.y, c.up.z],
        fovDeg: c.fovDeg,
      }),
    CAMERA,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(CAMERA, rect.width / rect.height), rect }
}

async function clickWorld(page: Page, ctx: Ctx, at: [number, number, number], shift = false): Promise<void> {
  const p = worldToPagePixel({ x: at[0], y: at[1], z: at[2] }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${at.join(',')}) does not project onto the canvas`)
  if (shift) await page.keyboard.down('Shift')
  await page.mouse.click(p.x, p.y)
  if (shift) await page.keyboard.up('Shift')
}

/** A click on empty space: gives the viewport the keyboard (for Delete)
 *  and clears the selection. */
async function focusViewport(page: Page): Promise<void> {
  await page.locator('canvas').first().click({ position: { x: 10, y: 10 } })
}

/** The Edit menu's Undo item reads `label`; the viewport has the keyboard
 *  again afterwards. */
async function expectUndoLabel(page: Page, label: string): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await expect(page.getByText(label, { exact: true })).toBeVisible()
  await page.keyboard.press('Escape')
  await focusViewport(page)
}

test('Shift-clicks mix a box, a guide and a dimension; Delete takes all three in one undo step', async ({ page }) => {
  const ctx = await setup(page)
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const guide = h.addGuidePoint(4, 0, 0)
    const dim = h.addLinearDimension([0, 3, 0], [2, 3, 0], [0, 1, 0], [0, 0, 0, 0, 0, 1])
    return { box, guide, dim }
  })

  // The guide first, then the box and the dimension: the guide stays the
  // primary.
  await clickWorld(page, ctx, [4, 0, 0])
  await clickWorld(page, ctx, [0.5, 0.5, 1], true)
  await clickWorld(page, ctx, [0.5, 4, 0], true)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getSelectedGuides())).toEqual([ids.guide])
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getSelectedAnnotations())).toEqual([ids.dim])
  expect(await page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.id))).toEqual([ids.box])

  // A second Shift-click on the guide takes it out again, and a third puts
  // it back.
  await clickWorld(page, ctx, [4, 0, 0], true)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getSelectedGuides())).toEqual([])
  await clickWorld(page, ctx, [4, 0, 0], true)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getSelectedGuides())).toEqual([ids.guide])

  await page.keyboard.press('Delete')
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)
  const after = await page.evaluate(() => ({
    guides: window.__hew_test!.getGuideIds(),
    annotations: window.__hew_test!.getAnnotationIds(),
    selected: window.__hew_test!.getSelectedGuides().length + window.__hew_test!.getSelectedAnnotations().length,
  }))
  expect(after).toEqual({ guides: [], annotations: [], selected: 0 })

  await expectUndoLabel(page, 'Undo Delete 3 items')

  // ONE undo brings all three back.
  await page.keyboard.press('ControlOrMeta+z')
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
  const restored = await page.evaluate(() => ({
    guides: window.__hew_test!.getGuideIds(),
    annotations: window.__hew_test!.getAnnotationIds(),
  }))
  expect(restored).toEqual({ guides: [ids.guide], annotations: [ids.dim] })
})

test('a Delete names its undo step by what it took', async ({ page }) => {
  await setup(page)
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    return {
      a: h.drawBox([0, 0, 0], [1, 1, 0], 1),
      b: h.drawBox([3, 0, 0], [4, 1, 0], 1),
      c: h.drawBox([6, 0, 0], [7, 1, 0], 1),
      g1: h.addGuidePoint(4, 4, 0),
      g2: h.addGuidePoint(5, 4, 0),
    }
  })
  await focusViewport(page)

  await page.evaluate((o) => window.__hew_test!.selectObjects([o.a, o.b]), ids)
  // Delete reads the rendered selection: wait for it before the key.
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 2)
  await page.keyboard.press('Delete')
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
  await expectUndoLabel(page, 'Undo Delete 2 objects')

  await page.evaluate((o) => window.__hew_test!.selectObjects([o.c]), ids)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('Delete')
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)
  await expectUndoLabel(page, 'Undo Delete object')

  await page.evaluate(
    (o) =>
      window.__hew_test!.selectNodes([
        { kind: 'guide', id: o.g1 },
        { kind: 'guide', id: o.g2 },
      ]),
    ids,
  )
  await page.waitForFunction(() => window.__hew_test!.getSelectedGuides().length === 2)
  await page.keyboard.press('Delete')
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getGuideIds())).toEqual([])
  await expectUndoLabel(page, 'Undo Delete 2 guides')
})

/** Page pixel of world point `at`. */
function pagePx(ctx: Ctx, at: [number, number, number]): { x: number; y: number } {
  const p = worldToPagePixel({ x: at[0], y: at[1], z: at[2] }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${at.join(',')}) does not project onto the canvas`)
  return p
}

/** A real marquee drag from `from` to `to` (page pixels): left to right is
 *  a window, right to left a crossing. */
async function marquee(page: Page, from: { x: number; y: number }, to: { x: number; y: number }): Promise<void> {
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  for (let i = 1; i <= 8; i++) {
    await page.mouse.move(from.x + ((to.x - from.x) * i) / 8, from.y + ((to.y - from.y) * i) / 8)
  }
  await page.mouse.up()
}

async function selectedKinds(page: Page): Promise<{ nodes: string[]; guides: string[]; annotations: string[] }> {
  return page.evaluate(() => ({
    nodes: window.__hew_test!.getSelection().map((n) => n.id),
    guides: window.__hew_test!.getSelectedGuides(),
    annotations: window.__hew_test!.getSelectedAnnotations(),
  }))
}

/** A box, a point guide, an endless vertical guide line between them, and a
 *  dimension, all on screen. */
async function guidesScene(page: Page) {
  return page.evaluate(() => {
    const h = window.__hew_test!
    return {
      box: h.drawBox([0, 0, 0], [1, 1, 0], 1),
      point: h.addGuidePoint(3, 0, 0),
      line: h.addGuideLine(2, 0, 0, 0, 0, 1),
      dim: h.addLinearDimension([0, 3, 0], [2, 3, 0], [0, 1, 0], [0, 0, 0, 0, 0, 1]),
    }
  })
}

test('a window marquee takes the box, the point guide and the dimension, never an endless guide line', async ({ page }) => {
  const ctx = await setup(page)
  const ids = await guidesScene(page)
  const pts = ([
    [0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1],
    [3, 0, 0], [0, 4, 0], [2, 4, 0],
  ] as [number, number, number][]).map((p) => pagePx(ctx, p))
  const margin = 25
  const minX = Math.min(...pts.map((p) => p.x)) - margin
  const maxX = Math.max(...pts.map((p) => p.x)) + margin
  const minY = Math.min(...pts.map((p) => p.y)) - margin
  const maxY = Math.max(...pts.map((p) => p.y)) + margin

  await marquee(page, { x: minX, y: minY }, { x: maxX, y: maxY })
  expect(await selectedKinds(page)).toEqual({ nodes: [ids.box], guides: [ids.point], annotations: [ids.dim] })

  // A crossing across the guide line alone, Shift held, adds it.
  const onLine = pagePx(ctx, [2, 0, 0.5])
  await page.keyboard.down('Shift')
  await marquee(page, { x: onLine.x + 15, y: onLine.y - 15 }, { x: onLine.x - 15, y: onLine.y + 15 })
  await page.keyboard.up('Shift')
  const after = await selectedKinds(page)
  expect(new Set(after.guides)).toEqual(new Set([ids.point, ids.line]))
  expect(after.nodes).toEqual([ids.box])
  expect(after.annotations).toEqual([ids.dim])
})

test('Select All and Invert Selection take the shown guides and annotations at the model level', async ({ page }) => {
  await setup(page)
  const ids = await guidesScene(page)
  await focusViewport(page)

  await page.keyboard.press('ControlOrMeta+a')
  await expect.poll(async () => (await selectedKinds(page)).guides.length).toBe(2)
  let sel = await selectedKinds(page)
  expect(sel.nodes).toEqual([ids.box])
  expect(new Set(sel.guides)).toEqual(new Set([ids.point, ids.line]))
  expect(sel.annotations).toEqual([ids.dim])

  // With View ▸ Guides off, the guides stay out.
  await page.evaluate(() => window.__hew_test!.setGuidesVisible(false))
  await page.evaluate(() => window.__hew_test!.selectAll())
  await expect.poll(async () => (await selectedKinds(page)).guides).toEqual([])
  await page.evaluate(() => window.__hew_test!.setGuidesVisible(true))

  // Invert of one selected guide is everything else.
  await page.evaluate((g) => window.__hew_test!.selectNodes([{ kind: 'guide', id: g }]), ids.point)
  await expect.poll(async () => (await selectedKinds(page)).guides).toEqual([ids.point])
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText('Invert Selection', { exact: true }).click()
  await expect.poll(async () => (await selectedKinds(page)).guides).toEqual([ids.line])
  sel = await selectedKinds(page)
  expect(sel.nodes).toEqual([ids.box])
  expect(sel.annotations).toEqual([ids.dim])
})

/** A real drag between two world points (page pixels in between). */
async function dragWorld(page: Page, ctx: Ctx, from: [number, number, number], to: [number, number, number]): Promise<void> {
  const a = pagePx(ctx, from)
  const b = pagePx(ctx, to)
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.move(b.x, b.y, { steps: 12 })
  await page.mouse.up()
}

/** A box and a point guide beside it, both selected. */
async function boxAndGuideSelected(page: Page): Promise<{ box: string; guide: string }> {
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    return { box: h.drawBox([0, 0, 0], [1, 1, 0], 1), guide: h.addGuidePoint(4, 0, 0) }
  })
  await focusViewport(page)
  await page.evaluate(
    (o) =>
      window.__hew_test!.selectNodes([
        { kind: 'object', id: o.box },
        { kind: 'guide', id: o.guide },
      ]),
    ids,
  )
  await page.waitForFunction(() => window.__hew_test!.getSelectedGuides().length === 1)
  return ids
}

test('Move carries the selected guide with the box, in one undo step', async ({ page }) => {
  const ctx = await setup(page)
  const ids = await boxAndGuideSelected(page)

  await page.keyboard.press('m')
  await dragWorld(page, ctx, [1, 0, 0], [3, 0, 0])
  // A typed distance redoes the move exactly, guide included.
  await page.keyboard.type('4')
  await page.keyboard.press('Enter')
  await expect.poll(() => page.evaluate((id) => window.__hew_test!.getObjectBounds(id)[0], ids.box)).toBeCloseTo(4, 5)
  const guide = await page.evaluate((id) => window.__hew_test!.getGuideGeometry(id), ids.guide)
  expect(guide?.[0]).toBeCloseTo(8, 5)
  expect(guide?.[1]).toBeCloseTo(0, 5)
  // Both stay selected.
  expect(await selectedKinds(page)).toEqual({ nodes: [ids.box], guides: [ids.guide], annotations: [] })

  await expectUndoLabel(page, 'Undo Move 2 items')
  await page.keyboard.press('ControlOrMeta+z')
  await expect.poll(() => page.evaluate((id) => window.__hew_test!.getObjectBounds(id)[0], ids.box)).toBeCloseTo(0, 5)
  expect((await page.evaluate((id) => window.__hew_test!.getGuideGeometry(id), ids.guide))?.[0]).toBeCloseTo(4, 5)
})

test('a Select drag on the selected box carries the selected guide too', async ({ page }) => {
  const ctx = await setup(page)
  const ids = await boxAndGuideSelected(page)

  await dragWorld(page, ctx, [0.5, 0.5, 1], [0.5, 2.5, 1])
  await expect.poll(() => page.evaluate((id) => window.__hew_test!.getObjectBounds(id)[1], ids.box)).toBeGreaterThan(0.5)
  const box = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids.box)
  const guide = await page.evaluate((id) => window.__hew_test!.getGuideGeometry(id), ids.guide)
  // The guide moved by the box's own offset.
  expect(guide?.[0]).toBeCloseTo(4 + box[0], 5)
  expect(guide?.[1]).toBeCloseTo(box[1], 5)
  expect(guide?.[2]).toBeCloseTo(box[2], 5)
  expect(await selectedKinds(page)).toEqual({ nodes: [ids.box], guides: [ids.guide], annotations: [] })
})
