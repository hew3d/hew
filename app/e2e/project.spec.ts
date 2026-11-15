import { test, expect } from '@playwright/test'
import { activateFromToolsMenu, clickWorld, expectStatus, moveToWorld, setup, shot, volume } from './helpers/offset'

/**
 * Project, driven with real pointer and keyboard events: a rectangle drawn on
 * the ground, a cylinder lying above it. Select the rectangle (or click it in
 * the tool), pick Project, click the cylinder (the shape lands on its
 * underside), type 1cm, Enter (engraves). Two undo steps: the depth, then the
 * drawn shape.
 */

// Seen from below: the shape lands on the underside of the cylinder, facing the ground.
const CAMERA = { position: [2.2, -3, -1.2] as [number, number, number], target: [0, 0, 1] as [number, number, number] }
/** A point on the cylinder's underside, off the very bottom line. */
const BELLY: [number, number, number] = [0.1, 0, 1.5 - Math.sqrt(0.25 - 0.01)]

type PageT = import('@playwright/test').Page

async function drawRect(page: PageT): Promise<string> {
  const rect = await page.evaluate(() => window.__hew_test!.drawRectangle([-0.15, -0.2, 0], [0.15, 0.2, 0]))
  return rect.sketch
}

async function buildScene(page: PageT, drawRectangle = true): Promise<{ cylinder: string; sketch: string }> {
  // A vertical 48-sided cylinder, laid on its side and lifted so its belly is 1 m above the ground.
  const circle = await page.evaluate(() => window.__hew_test!.drawCircle([0, 0, 0], 0.5, 48))
  const cylinder = await page.evaluate(([s, r]) => window.__hew_test!.extrudeRegion(s, r, 1), [circle.sketch, circle.region])
  await page.evaluate((id) => window.__hew_test!.rotateObject(id, 90, [1, 0, 0]), cylinder)
  const b = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), cylinder)
  // Center it over the origin, belly 1 m up.
  const move = [-(b[0] + b[3]) / 2, -(b[1] + b[4]) / 2, 1.5 - (b[2] + b[5]) / 2]
  await page.evaluate(
    ([id, m]) => window.__hew_test!.moveObject(id as string, (m as number[])[0], (m as number[])[1], (m as number[])[2]),
    [cylinder, move] as const,
  )
  return { cylinder, sketch: drawRectangle ? await drawRect(page) : '' }
}

test('Project: the shape lands on the curved wall, then a typed depth engraves it; two undos restore the cylinder', async ({ page }) => {
  await setup(page, CAMERA)
  const { cylinder, sketch } = await buildScene(page)
  const before = await volume(page, cylinder)
  expect(before).toBeGreaterThan(0.7)

  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'sketch', id }]), sketch)
  await activateFromToolsMenu(page, 'Project')
  await expectStatus(page, /Click the object to draw the shape on it/)

  // Hover, then click the cylinder (the click names the object; the shape lands on the side that faces the ground).
  await moveToWorld(page, BELLY)
  await page.waitForTimeout(150)
  await shot(page, 'project-hover')
  await clickWorld(page, BELLY)
  await expectStatus(page, /Depth: straight \(Alt for follow\)/)
  // Drawing the shape on the surface alone changes no volume.
  expect(await volume(page, cylinder)).toBeCloseTo(before, 6)
  await shot(page, 'project-depth-stage')

  await page.keyboard.type('1cm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  const after = await volume(page, cylinder)
  const area = 0.3 * 0.4
  // A 1 cm straight engraving on a curved wall removes between half and all of area x depth.
  expect(before - after).toBeGreaterThan(area * 0.01 * 0.4)
  expect(before - after).toBeLessThan(area * 0.01 * 1.05)
  await shot(page, 'project-engraved')

  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(100)
  expect(await volume(page, cylinder)).toBeCloseTo(before, 6) // the depth is its own step
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(100)
  expect(await volume(page, cylinder)).toBeCloseTo(before, 6)
})

test('Project: with nothing selected, a click picks the shape and the next the object; Alt switches to follow', async ({ page }) => {
  await setup(page, CAMERA)
  const { cylinder } = await buildScene(page)
  await activateFromToolsMenu(page, 'Project')
  await expectStatus(page, /Click the closed shape to project/)

  // The rectangle on the ground, seen from below, then the cylinder.
  await clickWorld(page, [0.05, 0.05, 0])
  await expectStatus(page, /Click the object to draw the shape on it\. Esc picks another shape/)
  await page.keyboard.press('Escape')
  await expectStatus(page, /Click the closed shape to project/)
  await clickWorld(page, [0.05, 0.05, 0])
  await expectStatus(page, /Click the object to draw the shape on it/)
  await clickWorld(page, BELLY)
  await expectStatus(page, /Depth: straight \(Alt for follow\)/)
  await page.keyboard.press('Alt')
  await expectStatus(page, /Depth: follow the surface \(Alt for straight\)/)
  const before = await volume(page, cylinder)
  await page.keyboard.type('1cm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  expect(await volume(page, cylinder)).toBeLessThan(before - 1e-4)
})

test('Project: a shape picked with a real click in the drawing (not the whole sketch) is the one projected', async ({ page }) => {
  await setup(page, { position: [0, -0.01, 5], target: [0, 0, 0] })
  // From above, Select picks the rectangle's fill on the ground (before the cylinder covers it).
  await drawRect(page)
  await clickWorld(page, [0.05, 0, 0])
  const picked = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(picked.length).toBeGreaterThan(0)
  expect(picked[0].kind.startsWith('draft')).toBe(true)
  const { cylinder } = await buildScene(page, false)
  const before = await volume(page, cylinder)

  await activateFromToolsMenu(page, 'Project')
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), { ...CAMERA, up: [0, 0, 1] as [number, number, number], fovDeg: 45 })
  await page.waitForTimeout(100)
  await expectStatus(page, /Click the object to draw the shape on it/)
  await clickWorld(page, BELLY)
  await expectStatus(page, /Depth: straight \(Alt for follow\)/)
  await page.keyboard.type('1cm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  expect(await volume(page, cylinder)).toBeLessThan(before - 1e-4)
})
