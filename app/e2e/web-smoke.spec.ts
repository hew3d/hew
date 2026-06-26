import { test, expect } from '@playwright/test'
import { settleFrame } from './helpers/render'

/**
 *  — the canonical web E2E smoke flow: **launch → draw rectangle →
 * push/pull → save/reload → screenshot** (docs/DEVELOPMENT.md).
 *
 * It drives the semantic harness (`window.__hew_test`) for the modeling +
 * persistence logic — deterministic, no canvas-pixel math — and uses the *pixel*
 * channel only where it is the actual subject: confirming the WebGL viewport
 * rendered the geometry (a screenshot that differs from the empty scene) and
 * still shows it after a save/open round-trip. The strong fidelity guarantee is
 * logical, not visual: the document's `state_hash` and object count must survive
 * save→reload byte-for-byte. Pinned pixel goldens are deliberately *not* here —
 * GPU variance makes them flaky off a pinned runner (that's).
 *
 * Harness specifics (handles as decimal strings, pixel-free `pickFace`) live in
 * harness.spec.ts; this spec is the lifecycle on top of it.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

// A box framed by a fixed camera so every screenshot in the test shares one
// pose (the PINNED pose, reused by harness.spec.ts).
const CAMERA = { position: [8, 6, 8], target: [1, 1, 1], up: [0, 0, 1], fovDeg: 45 } as const

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  // Harness installed (mount) AND the WASM kernel ready (scene non-null).
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  await settleFrame(page)
})

test('draw → push/pull → save/reload preserves state and renders', async ({
  page,
}, testInfo) => {
  const canvas = page.locator('canvas').first()
  await expect(canvas).toBeVisible()

  // (0) Empty scene baseline — nothing drawn yet.
  const emptyShot = await canvas.screenshot()
  await testInfo.attach('00-empty', { body: emptyShot, contentType: 'image/png' })
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)

  // (1) Draw a 2×2×2 box and push/pull its top face (z=2) up by 1 → z=3.
  const drawn = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 2, 0], 2)
    const pick = h.pickFace([1, 1, 10], [0, 0, -1]) // ray straight down onto the top face
    h.pushPull(pick!.object, pick!.face, 1)
    return { box, count: h.getObjectCount(), hash: h.getStateHash() }
  })
  expect(drawn.count).toBe(1)

  await settleFrame(page)
  const drawnShot = await canvas.screenshot()
  await testInfo.attach('01-drawn', { body: drawnShot, contentType: 'image/png' })
  // The viewport actually rendered the solid: the framebuffer changed from empty.
  expect(drawnShot.equals(emptyShot)).toBe(false)

  // (2) Save the live `.hew` bytes, then reload them through the app's real Open
  // path. The document must come back identical — same hash, same object count.
  const reloaded = await page.evaluate(() => {
    const h = window.__hew_test!
    const bytes = h.save()
    h.load(bytes)
    return { count: h.getObjectCount(), hash: h.getStateHash(), bytes: bytes.length }
  })
  expect(reloaded.bytes).toBeGreaterThan(0)
  expect(reloaded.count).toBe(drawn.count)
  expect(reloaded.hash).toBe(drawn.hash) // serialization round-trip is lossless

  // (3) The reloaded document still renders the geometry under the same camera.
  await settleFrame(page)
  const reloadedShot = await canvas.screenshot()
  await testInfo.attach('02-reloaded', { body: reloadedShot, contentType: 'image/png' })
  expect(reloadedShot.equals(emptyShot)).toBe(false)
})
