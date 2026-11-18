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
  await page.keyboard.press('Control+z')
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
