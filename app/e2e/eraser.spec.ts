import { test, expect } from '@playwright/test'
import { arcPolylineOnPlane } from '../src/tools/arcMath'
import { clickWorld, moveToWorld, pagePoint, setup, shot, volume } from './helpers/offset'

/**
 * Eraser, driven with real pointer events: a plain click erases a drawn line,
 * a drag sweeps several as one undo step, a plain click on a solid's edge
 * only explains, and Ctrl-click creases it (read back through the volume of
 * the smoothed box: a creased edge stays sharp, so more material remains).
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

test('Eraser: a click erases a drawn line; a drag across three erases all of them in one undo step', async ({ page }) => {
  await setup(page, { position: [3.5, -3, 6], target: [3.5, 3.5, 0] })
  const lines: string[] = []
  for (const y of [3, 4, 5]) {
    const c = await page.evaluate((y) => window.__hew_test!.drawLineChain([[3, y, 0], [4, y, 0]]), y)
    lines.push(c.sketch)
  }
  const edgeCount = async (): Promise<number> => {
    let n = 0
    for (const s of lines) {
      try {
        n += (await page.evaluate((s) => window.__hew_test!.getSketchEdgeIds(s), s)).length
      } catch {
        // an emptied sketch is gone
      }
    }
    return n
  }
  expect(await edgeCount()).toBe(3)

  await page.keyboard.press('e')
  await page.waitForTimeout(100)
  await clickWorld(page, [3.5, 3, 0])
  await page.waitForTimeout(200)
  expect(await edgeCount()).toBe(2)
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  expect(await edgeCount()).toBe(3)

  // Drag from below the first line to above the last, across x = 3.5.
  const from = await pagePoint(page, [3.5, 2.5, 0])
  const to = await pagePoint(page, [3.5, 5.5, 0])
  await page.mouse.move(from.x, from.y)
  await page.waitForTimeout(60)
  await page.mouse.down()
  const steps = 24
  for (let i = 1; i <= steps; i++) {
    await page.mouse.move(from.x + ((to.x - from.x) * i) / steps, from.y + ((to.y - from.y) * i) / steps)
  }
  await shot(page, 'eraser-sweep')
  await page.mouse.up()
  await page.waitForTimeout(250)
  expect(await edgeCount()).toBe(0)
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  expect(await edgeCount()).toBe(3)
})

test('Eraser: Escape during a drag changes nothing', async ({ page }) => {
  await setup(page, { position: [3.5, -3, 6], target: [3.5, 3.5, 0] })
  const c = await page.evaluate(() => window.__hew_test!.drawLineChain([[3, 3, 0], [4, 3, 0]]))
  await page.keyboard.press('e')
  const from = await pagePoint(page, [3.5, 2.5, 0])
  const to = await pagePoint(page, [3.5, 3.5, 0])
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  for (let i = 1; i <= 12; i++) await page.mouse.move(from.x + ((to.x - from.x) * i) / 12, from.y + ((to.y - from.y) * i) / 12)
  await page.keyboard.press('Escape')
  await page.mouse.up()
  await page.waitForTimeout(200)
  expect((await page.evaluate((s) => window.__hew_test!.getSketchEdgeIds(s), c.sketch)).length).toBe(1)
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

test('Eraser: a plain click on a solid edge erases nothing and says why', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  const before = await volume(page, box)
  await page.keyboard.press('e')
  await page.waitForTimeout(100)
  await moveToWorld(page, [0.5, 0, 0])
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)

  await expect(page.getByText(/Edges of a solid can't be erased/).first()).toBeVisible()
  expect(await volume(page, box)).toBeCloseTo(before, 6)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
  expect(await page.evaluate((id) => window.__hew_test!.getCreasedEdgeCount(id), box)).toBe(0)
})
