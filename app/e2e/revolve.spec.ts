import { test, expect } from '@playwright/test'
import type { CameraParams } from './helpers/projectWorldToScreen'
import { segmentsPerTurn } from '../src/tools/arcMath'
import { aim, clickAt, moveTo, sweep, activateFromMenu, shot } from './helpers/shapes'

/**
 * Revolve — driven end to end with REAL pointer and keyboard input.
 *
 * A rectangle (x 1..2, y 0..1) lies on the ground beside a drawn line at
 * x = -0.5 running along Y. Revolve, click the rectangle, click the line,
 * press Enter: one watertight solid, a tube 1.5..2.5 m from the line. The
 * scene is built through the harness; only the tool is exercised with the
 * pointer.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 4, y: -6, z: 5 },
  target: { x: -0.2, y: 0.5, z: 0 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

async function buildScene(page: import('@playwright/test').Page): Promise<void> {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawRectangle([1, 0, 0], [2, 1, 0])
    h.drawLineChain([[-0.5, -0.5, 0], [-0.5, 1.5, 0]])
  })
}

test('Revolve: rectangle, line, Enter makes one smooth tube', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await buildScene(page)
  await activateFromMenu(page, 'Revolve', 'Click a drawn shape or a solid face to revolve.')

  await moveTo(page, ctx, 1.5, 0.5, 0)
  await shot(page, 'revolve-1-hover-profile')
  await clickAt(page, ctx, 1.5, 0.5, 0)
  await expect(page.getByText('Click an edge or two points for the axis.')).toBeVisible()

  await moveTo(page, ctx, -0.5, 1.2, 0)
  await shot(page, 'revolve-2-hover-axis')
  await clickAt(page, ctx, -0.5, 1.2, 0)
  await expect(page.getByText('Move to set the angle, click to finish, Enter for a full turn.')).toBeVisible()
  await shot(page, 'revolve-3-full-turn-ghost', true)

  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getObjectIds()[0]
    return { solid: h.isObjectSolid(id), bounds: h.getObjectBounds(id) }
  })
  expect(result.solid).toBe(true)
  const [x0, y0, z0, x1, y1, z1] = result.bounds
  expect(x0).toBeCloseTo(-3, 3)
  expect(x1).toBeCloseTo(2, 3)
  expect(y0).toBeCloseTo(0, 3)
  expect(y1).toBeCloseTo(1, 3)
  expect(z0).toBeCloseTo(-2.5, 3)
  expect(z1).toBeCloseTo(2.5, 3)
  await shot(page, 'revolve-4-tube')

  // One Undo takes the whole revolve back and restores the rectangle.
  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 0)
})

test('Revolve: moving sets the angle (15 degree snaps), a click finishes there', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await buildScene(page)
  await activateFromMenu(page, 'Revolve', 'Click a drawn shape or a solid face to revolve.')
  await clickAt(page, ctx, 1.5, 0.5, 0)
  await clickAt(page, ctx, -0.5, 1.2, 0)

  // Sweep the pointer over the top of the line: straight above the axis is a
  // three-quarter turn about it (right-handed), shown in the measurements box.
  await sweep(page, ctx, [[-0.5, 0.5, 0.4], [-0.5, 0.5, 1.0], [-0.5, 0.5, 1.6]])
  await expect(page.getByText('270°')).toBeVisible()
  await shot(page, 'revolve-5-partial-angle')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  const vertices = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getObjectVertices(h.getObjectIds()[0])
  })
  // Four profile corners per ring: a full tube has one ring per segment, a
  // three-quarter turn has about three quarters of them (plus the end ring).
  const full = 4 * segmentsPerTurn(2.5)
  expect(vertices.length).toBeLessThan(full)
  expect(vertices.length).toBeGreaterThan(full / 2)
})

test('Revolve: a typed angle and segments, then retype after the commit', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await buildScene(page)
  await activateFromMenu(page, 'Revolve', 'Click a drawn shape or a solid face to revolve.')
  await clickAt(page, ctx, 1.5, 0.5, 0)
  await clickAt(page, ctx, -0.5, 1.2, 0)

  await page.keyboard.type('48s')
  await page.keyboard.press('Enter')
  await page.keyboard.type('90')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  const quarter = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getObjectBounds(h.getObjectIds()[0])
  })
  // A quarter turn about the line stays on the profile's side of it.
  expect(quarter[0]).toBeGreaterThan(-0.6)

  // Retype: a different angle redoes the revolve in place.
  await page.keyboard.type('180')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(300)
  const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  const half = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids[0])
  // Half a turn reaches the far side of the line (x = -0.5 - 2.5).
  expect(half[0]).toBeCloseTo(-3, 3)
})

test('Revolve: an axis outside the profile plane is refused in plain words', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await buildScene(page)
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([3, 0, 0], [4, 1, 0], 1)
  })
  await activateFromMenu(page, 'Revolve', 'Click a drawn shape or a solid face to revolve.')
  await clickAt(page, ctx, 1.5, 0.5, 0)
  // The box's vertical edge at (3, 0): not in the ground plane.
  await moveTo(page, ctx, 3, 0, 0.5)
  await expect(page.getByText('The axis has to lie in the plane of the shape. Move the axis onto that plane.')).toBeVisible()
})
