import { test, expect } from '@playwright/test'
import { arcPolylineOnPlane } from '../src/tools/arcMath'
import { moveToWorld, pagePoint, setup, shot, volume } from './helpers/offset'
import { requiresPreview } from './helpers/preview'

// The Eraser's crease half is part of the Smooth Preview feature: skipped in the production-shape run.
requiresPreview('smooth')

/**
 * Eraser creasing, driven with real pointer events: Ctrl-click creases an
 * edge of a solid (read back through the volume of the smoothed box: a
 * creased edge stays sharp, so more material remains), Ctrl-click again lets
 * it soften, a Ctrl-drag creases every edge it crosses.
 */

const CAMERA = { position: [3, -3.4, 2.4] as [number, number, number], target: [0.5, 0.5, 0.4] as [number, number, number] }

test('Eraser: E starts it; Ctrl-click creases an edge of a smoothed box and Ctrl-click again lets it soften', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((id) => window.__hew_test!.setSmoothLevel(id, 2), box)
  const smooth = await volume(page, box)

  await page.keyboard.press('e')
  await page.waitForTimeout(100)
  // The nearest vertical edge: (1,0,0)–(1,0,1).
  await moveToWorld(page, [1, 0, 0.5])
  await page.waitForTimeout(150)
  await expect(page.getByText(/Edges of a solid can't be erased/).first()).toBeVisible()

  // A plain click changes nothing.
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  expect(await volume(page, box)).toBeCloseTo(smooth, 9)

  // Ctrl-click creases: the edge stays sharp, so the smoothed box keeps more.
  await page.keyboard.down('Control')
  await moveToWorld(page, [1, 0, 0.5])
  await page.waitForTimeout(100)
  await expect(page.getByText('Crease: keeps this edge sharp under Smooth').first()).toBeVisible()
  await shot(page, 'eraser-crease-hover')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(250)
  const creased = await volume(page, box)
  expect(creased).toBeGreaterThan(smooth + 1e-4)
  await shot(page, 'eraser-creased')

  // Again: the crease comes off.
  await page.mouse.down()
  await page.mouse.up()
  await page.keyboard.up('Control')
  await page.waitForTimeout(250)
  expect(await volume(page, box)).toBeCloseTo(smooth, 9)

  // One undo is one crease.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  expect(await volume(page, box)).toBeCloseTo(creased, 9)
})

test('Eraser: Ctrl-click on one facet of a softened round creases the whole base ring', async ({ page }) => {
  // Close in, so a facet is wide enough on screen to pick away from its vertices.
  await setup(page, { position: [2, -4.3, 1.4], target: [2, -2, 0.2] })
  const arc = arcPolylineOnPlane([0, 0, 0], [4, 0, 0], -2, [1, 0, 0], [0, 1, 0])!
  // A half-round of facets, a plain curve chain with no circle behind it
  // (as an imported or boolean-cut round arrives), extruded and softened.
  const { sketch, regions } = await page.evaluate(() =>
    window.__hew_test!.drawArc([0, 0, 0], [4, 0, 0], -2, true, false),
  )
  const cyl = await page.evaluate(([s, r]) => window.__hew_test!.extrudeRegion(s, r, 2), [sketch, regions[0]])
  await page.evaluate((id) => window.__hew_test!.softenByAngle(id, 30), cyl)
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), cyl)).toBe(0)

  await page.keyboard.press('e')
  await page.keyboard.down('Control')
  // The midpoint of a base facet facing the camera, away from any vertex.
  const k = Math.floor((arc.length - 1) / 2)
  await moveToWorld(page, [(arc[k][0] + arc[k + 1][0]) / 2, (arc[k][1] + arc[k + 1][1]) / 2, 0])
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.keyboard.up('Control')
  await page.waitForTimeout(250)
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), cyl)).toBe(arc.length - 1)
})

test('Eraser: Ctrl-drag across two solid edges creases both; one undo clears both', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), box)).toBe(0)

  await page.keyboard.press('e')
  await page.waitForTimeout(100)
  // Two top edges whose faces both face the camera: front (y = 0) and right (x = 1).
  await moveToWorld(page, [0.5, 0, 1])
  await page.waitForTimeout(150)
  await page.keyboard.down('Control')
  await page.mouse.down()
  const to = await pagePoint(page, [1, 0.5, 1])
  await page.mouse.move(to.x, to.y, { steps: 6 })
  await page.mouse.up()
  await page.keyboard.up('Control')
  await page.waitForTimeout(250)
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), box)).toBe(2)

  // One undo clears both: the drag was one history step.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), box)).toBe(0)
})

