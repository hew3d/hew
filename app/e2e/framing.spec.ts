import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Zoom Extents framing over the INSTANCED-BATCH render path. Component
 * placements draw as one THREE.InstancedMesh per (member, bucket) with the
 * poses in the instanceMatrix attribute — the batch node's matrixWorld is
 * identity and its geometry bbox covers only the definition at the origin.
 * The regression this pins: the visibility-aware fit-box traversal read
 * geometry.boundingBox × matrixWorld, so any model with placed instances
 * mis-framed (the camera ignored every placement's actual position).
 *
 * Assertions read the camera pose through the harness (`getCamera`), the
 * read complement of the pinned `setCamera` the other specs use.
 */

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

test('Zoom Extents frames a component instance placed far from the origin', async ({ page }) => {
  await setup(page)

  await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { component } = h.makeComponent([box])
    // A second placement 50 m out — both placements render batched
    // (nothing selected, nothing hidden).
    h.placeInstance(component, 50, 0, 0)
    h.zoomExtents()
  })

  const cam = await page.evaluate(() => window.__hew_test!.getCamera())
  // The visible extent spans x ∈ [0, 51]; its center is ≈ 25.5. Before the
  // instanced-bounds fix the batch contributed only the definition at the
  // origin, leaving the target at ≈ 0.5.
  expect(cam.target[0]).toBeGreaterThan(10)
  expect(cam.target[0]).toBeLessThan(40)
})

test('mixed scene: a plain object plus a distant instance both frame', async ({ page }) => {
  await setup(page)

  await page.evaluate(() => {
    const h = window.__hew_test!
    // A plain (non-component) solid near the origin…
    h.drawBox([0, 0, 0], [1, 1, 0], 1)
    // …plus a component whose second placement sits 50 m out.
    const other = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    const { component } = h.makeComponent([other])
    h.placeInstance(component, 50, 0, 0)
    h.zoomExtents()
  })

  const cam = await page.evaluate(() => window.__hew_test!.getCamera())
  // Extent spans x ∈ [0, 54]: the target re-centers between the plain solid
  // and the distant placement, and the camera pulls back far enough to
  // cover the whole span (fit distance for a ~27 m half-diagonal at the
  // default 45° FOV is well over 30 m).
  expect(cam.target[0]).toBeGreaterThan(10)
  const dx = cam.position[0] - cam.target[0]
  const dy = cam.position[1] - cam.target[1]
  const dz = cam.position[2] - cam.target[2]
  expect(Math.hypot(dx, dy, dz)).toBeGreaterThan(30)
})
