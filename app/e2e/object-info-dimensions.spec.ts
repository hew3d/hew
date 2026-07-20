import { test, expect, type Locator } from '@playwright/test'

/**
 * Object Info — the Bounding Box readout (world axis-aligned bounding box).
 *
 * Real-wiring coverage the jsdom component tests can't give: the panel driven
 * from an actual selection over live kernel geometry, in a real browser.
 *
 * - The first test exercises the full lifecycle across an undo that removes the
 *   selected object: the row appears for the solid, then the app stays mounted
 *   and the row cleanly disappears once the selection is reconciled. (App.tsx's
 *   `pruneDeadSelection` drops the dead handle in the same batched reconcile
 *   that bumps the panel's doc revision, so the panel never renders over a
 *   dead object; the stale-handle guard in `objectBounds.ts` is belt-and-
 *   suspenders on top, covered directly by the unit tests.)
 * - The second and third assert the readout itself: correct extents in the
 *   active unit, and that a rotated object's world AABB grows on the rotated
 *   axes (the headline "does it still fit the bed" behavior).
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
})

/** The Object Info "Bounding Box" value cell — the div immediately after the
 * "Bounding Box" label (BoundingBoxRow in ObjectInfoPanel). */
function dimensionsValue(page: import('@playwright/test').Page): Locator {
  return page
    .getByText('Bounding Box', { exact: true })
    .locator('xpath=following-sibling::div[1]')
}

/** Parse "X 220 mm × Y 150 mm × Z 100 mm" into per-axis millimetre numbers. */
function parseMm(text: string): { x: number; y: number; z: number } {
  const m = /X\s+([\d.]+)\s*mm.*Y\s+([\d.]+)\s*mm.*Z\s+([\d.]+)\s*mm/.exec(text)
  if (m === null) throw new Error(`Bounding Box text did not parse: ${JSON.stringify(text)}`)
  return { x: Number(m[1]), y: Number(m[2]), z: Number(m[3]) }
}

test('extrude → select → undo keeps the app mounted and drops the Bounding Box row', async ({
  page,
}) => {
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    // A rectangle pushed to a solid; select it so Object Info shows the Bounding Box.
    const objId = h.drawBox([0, 0, 0], [0.2, 0.2, 0], 0.2)
    h.selectObjects([objId])
    return { objId, count: h.getObjectCount() }
  })
  expect(setup.count).toBe(1)
  // The selected solid shows its Bounding Box row.
  await expect(page.getByText('Bounding Box', { exact: true })).toBeVisible()

  // Undo removes the object. The panel re-renders across the reconcile: the
  // dead selection is pruned and the doc revision bumps together, so the row
  // simply drops — and the app must stay alive through it.
  const after = await page.evaluate(() => {
    const h = window.__hew_test!
    h.undo()
    return { count: h.getObjectCount() }
  })
  expect(after.count).toBe(0)

  // The app survived: no ErrorBoundary, the panel chrome is still mounted, the
  // harness is still live, and the Bounding Box row is gone (the selection now
  // resolves to no mesh).
  await expect(page.getByText('Hew hit an error')).toHaveCount(0)
  await expect(page.getByText('Object Info')).toBeVisible()
  await expect(page.getByText('Bounding Box', { exact: true })).toHaveCount(0)
  expect(await page.evaluate(() => window.__hew_test!.isReady())).toBe(true)
})

test('Bounding Box shows the selected box extents in the active unit', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setLengthUnit('mm')
    // 220 mm × 150 mm footprint, extruded 100 mm tall.
    const objId = h.drawBox([0, 0, 0], [0.22, 0.15, 0], 0.1)
    h.selectObjects([objId])
  })

  const value = dimensionsValue(page)
  await expect(value).toContainText('X 220 mm')
  await expect(value).toContainText('Y 150 mm')
  await expect(value).toContainText('Z 100 mm')
})

test("a rotated box's world AABB grows on the rotated axes", async ({ page }) => {
  const objId = await page.evaluate(() => {
    const h = window.__hew_test!
    h.setLengthUnit('mm')
    const id = h.drawBox([0, 0, 0], [0.22, 0.15, 0], 0.1)
    h.selectObjects([id])
    return id
  })

  const value = dimensionsValue(page)
  await expect(value).toContainText('X 220 mm')
  const before = parseMm((await value.textContent()) ?? '')

  // Rotate 45° about world +Z: the footprint's world AABB grows to its
  // diagonal on X and Y; the rotation axis (Z, the height) is unchanged.
  await page.evaluate((id) => window.__hew_test!.rotateObject(id, 45, [0, 0, 1]), objId)

  // Poll until the panel re-renders with the post-rotation extents.
  await expect.poll(async () => parseMm((await value.textContent()) ?? '').x).toBeGreaterThan(
    before.x,
  )
  const after = parseMm((await value.textContent()) ?? '')
  expect(after.x).toBeGreaterThan(before.x)
  expect(after.y).toBeGreaterThan(before.y)
  // Height (rotation axis) is unchanged.
  expect(after.z).toBeCloseTo(before.z, 1)
})
