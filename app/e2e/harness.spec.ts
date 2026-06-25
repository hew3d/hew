import { test, expect } from '@playwright/test'

/**
 * Semantic harness E2E. Drives the app through `window.__hew_test` —
 * logic, not canvas pixels (docs/DEVELOPMENT.md) — proving the harness is wired to the
 * live kernel + app reconcile. This is the substrate 's smoke flow builds
 * on. The harness installs only in debug/test builds; we run against the Vite
 * dev server (`import.meta.env.DEV`), see playwright.config.ts.
 */

// Tighten the world type for the harness inside page.evaluate.
declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  // Harness installed (mount) AND the WASM kernel ready (scene non-null).
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
})

test('harness drives kernel ops and reflects state', async ({ page }) => {
  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    // Deterministic framing first ( setCamera reuses the PINNED pose).
    h.setCamera({ position: [8, 6, 8], target: [1, 1, 1], up: [0, 0, 1], fovDeg: 45 })

    const before = h.getObjectCount()
    const box = h.drawBox([0, 0, 0], [2, 2, 0], 2) // 2×2×2, top face at z=2
    const afterDraw = h.getObjectCount()
    const hashAfterDraw = h.getStateHash()
    const ids = h.getObjectIds()

    // Pixel-free face pick: ray straight down onto the top face.
    const pick = h.pickFace([1, 1, 10], [0, 0, -1])
    h.pushPull(pick!.object, pick!.face, 1) // top face up to z=3
    const hashAfterPush = h.getStateHash()

    return { before, box, afterDraw, hashAfterDraw, ids, pick, hashAfterPush }
  })

  expect(result.before).toBe(0)
  expect(result.afterDraw).toBe(1)
  expect(result.ids).toEqual([result.box]) // handles are decimal strings
  expect(result.pick).not.toBeNull()
  expect(result.pick!.object).toBe(result.box) // picked the box we drew
  // push/pull mutated the document → the state hash changed.
  expect(result.hashAfterPush).not.toBe(result.hashAfterDraw)

  // Object-level selection goes through React state, so poll until it reflects.
  await page.evaluate((box) => window.__hew_test!.selectObjects([box]), result.box)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  const selection = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(selection).toEqual([{ kind: 'object', id: result.box }])
})

test('recording captures the high-level call stream + low-level input', async ({
  page,
}) => {
  const artifact = await page.evaluate(() => {
    const h = window.__hew_test!
    h.startRecording()
    // A camera change is captured as a low-level `input` event (the Viewport
    // hook fires on OrbitControls 'change' while recording).
    h.setCamera({ position: [10, 8, 9], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 50 })
    h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const wasRecording = h.isRecording()
    h.stopRecording()
    return { json: h.takeRecording(), wasRecording }
  })

  expect(artifact.wasRecording).toBe(true)
  const rec = JSON.parse(artifact.json)
  expect(rec.version).toBe(2)
  // High-level: the box's kernel calls were recorded.
  expect(Array.isArray(rec.calls)).toBe(true)
  expect(rec.calls.length).toBeGreaterThan(0)
  expect(rec.calls.some((c: { method: string }) => c.method === 'extrude_region')).toBe(true)
  // Low-level sibling array: at least the camera change.
  expect(Array.isArray(rec.input)).toBe(true)
  expect(rec.input.some((e: { kind: string }) => e.kind === 'camera')).toBe(true)
})

test('getLastError surfaces a failed op', async ({ page }) => {
  const err = await page.evaluate(() => {
    const h = window.__hew_test!
    try {
      // Bogus handles → the kernel rejects; the harness records lastError.
      h.pushPull('999999', '999999', 1)
    } catch {
      /* expected to throw */
    }
    return h.getLastError()
  })
  expect(err).not.toBeNull()
  expect(typeof err).toBe('string')
})
