import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Painting component placements (docs/agents/PLUGINS.md "Materials"): one
 * definition placed three times, one placement painted with the real Paint
 * tool (only that placement re-keys its render on the new material), a
 * second painted through the harness, then a real double-click into a
 * third to paint a member face — that face changes in every placement,
 * the painted ones included, since a face's own paint wins. The palette's
 * usage count includes the placements.
 *
 * Ground truth is the kernel (`getInstanceMaterial`, `getFaceMaterial`,
 * `materialUsage`) and the renderer's own record of what each placement
 * draws (`getInstanceRenderedMaterials`), not pixels.
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

const CAMERA: CameraParams = {
  position: { x: 4, y: -9, z: 7 },
  target: { x: 4, y: 0.5, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.1,
  far: 1000,
}

async function aim(page: Page): Promise<Ctx> {
  await page.evaluate(
    (cam) =>
      window.__hew_test!.setCamera({
        position: [cam.position.x, cam.position.y, cam.position.z],
        target: [cam.target.x, cam.target.y, cam.target.z],
        up: [cam.up.x, cam.up.y, cam.up.z],
        fovDeg: cam.fovDeg,
      }),
    CAMERA,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(CAMERA, rect.width / rect.height), rect }
}

function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

/** Move first (settles the hover pick), then click. */
async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 4, p.y - 4)
  await page.waitForTimeout(40)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(120)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

test('paint one placement, then a member face inside another', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const ctx = await aim(page)

  // A 1 m box made a component, placed three times along x.
  const s = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const second = h.placeInstance(component, 3, 0, 0)
    const third = h.placeInstance(component, 6, 0, 0)
    const red = h.addMaterial('Instance Red', 220, 40, 40, 255)
    const blue = h.addMaterial('Face Blue', 40, 40, 220, 255)
    return { first: instance, second, third, component, red, blue, member: h.getComponentMemberObjects(component)[0] }
  })

  // ---- 1. The real Paint tool at the top level paints the placement.
  const swatch = page.getByTitle('Instance Red')
  if (!(await swatch.isVisible().catch(() => false))) {
    await page.getByText('Materials', { exact: true }).first().click()
  }
  await swatch.click()
  await expect(page.getByText(/Click a face to paint it with the current material/i)).toBeVisible()
  await clickWorld(page, ctx, 3.5, 0.5, 1)
  await expect(page.getByText(/Painting a component colors this placement/i)).toBeVisible()

  const own = (id: string) => page.evaluate((i) => window.__hew_test!.getInstanceMaterial(i), id)
  const rendered = (id: string) => page.evaluate((i) => window.__hew_test!.getInstanceRenderedMaterials(i), id)
  expect(await own(s.second)).toBe(s.red)
  expect(await own(s.first)).toBeNull()
  expect(await own(s.third)).toBeNull()
  // Only the painted placement draws its member under the new material.
  expect(await rendered(s.second)).toEqual([s.red])
  expect(await rendered(s.first)).toEqual([null])
  expect(await rendered(s.third)).toEqual([null])
  // The definition's faces are untouched.
  const painted = await page.evaluate(() => window.__hew_test!.pickFace([3.5, 0.5, 5], [0, 0, -1]))
  expect(painted?.instance).toBe(s.second)
  expect(painted?.object).toBe(s.member)
  expect(
    await page.evaluate(({ o, f }) => window.__hew_test!.getFaceMaterial(o, f), { o: painted!.object, f: painted!.face }),
  ).toEqual({ face: null, objectDefault: null })

  // ---- 2. The harness paints a second placement the same way.
  await page.evaluate(({ id, m }) => window.__hew_test!.paintInstance(id, m), { id: s.first, m: s.red })
  expect(await rendered(s.first)).toEqual([s.red])
  expect(await rendered(s.third)).toEqual([null])

  // ---- 3. Double-click into the third placement (with Select: a Paint
  // click would paint the placement first) and paint its top face.
  await page.keyboard.press('Space')
  const p = px(ctx, 6.5, 0.5, 1)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(s.third)
  expect(await own(s.third)).toBeNull()
  const top = await page.evaluate(() => window.__hew_test!.pickFace([6.5, 0.5, 5], [0, 0, -1]))
  expect(top).not.toBeNull()
  expect(top!.instance).toBeNull()
  await page.evaluate(
    ({ o, f, m }) => window.__hew_test!.paintFace(o, f, m),
    { o: top!.object, f: top!.face, m: s.blue },
  )
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()

  // The face is the definition's: painted in every placement, and a
  // face's own paint wins over the placement's.
  const after = await page.evaluate(
    ({ o, f }) => window.__hew_test!.getFaceMaterial(o, f),
    { o: top!.object, f: top!.face },
  )
  expect(after?.face).toBe(s.blue)
  const hit = await page.evaluate(() => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]))
  expect(hit?.instance).toBe(s.first)
  expect(await page.evaluate(({ o, f }) => window.__hew_test!.getFaceMaterial(o, f), { o: hit!.object, f: hit!.face }))
    .toEqual({ face: s.blue, objectDefault: null })
  expect(await own(s.first)).toBe(s.red)

  // ---- The palette's usage count includes the placements.
  expect(await page.evaluate((m) => window.__hew_test!.materialUsage(m), s.red)).toBe(2)
  expect(await page.evaluate(() => window.__hew_test!.unusedMaterials())).not.toContain(s.red)
})
