/**
 * Desktop smoke: the real Hew binary boots under tauri-driver, the WASM
 * kernel comes up inside WebKitGTK, and the semantic harness round-trips a
 * model. This is *desktop-wiring* proof — modeling correctness lives in the
 * kernel/harness suites, so keep this thin (docs/DEVELOPMENT.md: top of the pyramid).
 */

// Make this file a module so the `declare global` below augments (not replaces)
// the global scope. WebdriverIO's `browser`/`expect` are ambient globals.
export {}

type Vec3 = [number, number, number]

// The slice of `window.__hew_test` (app/src/test/harness.ts) this smoke uses.
// Kept minimal + local so the desktop E2E typechecks in isolation, without
// pulling app source across the package boundary.
interface HarnessSlice {
  isReady(): boolean
  setCamera(pose: { position: Vec3; target: Vec3; up?: Vec3; fovDeg?: number }): void
  drawBox(p0: Vec3, p1: Vec3, height: number): string
  pickFace(origin: Vec3, dir: Vec3): { object: string; face: string } | null
  pushPull(object: string, face: string, distance: number): void
  save(): number[]
  load(bytes: number[]): void
  getObjectCount(): number
  getStateHash(): string
}

declare global {
  interface Window {
    __hew_test?: HarnessSlice
  }
}

describe('Hew desktop', () => {
  it('boots WebKitGTK + WASM kernel and the harness round-trips a model', async () => {
    // The harness installs on mount (VITE_HEW_TEST=1 build) and reports ready
    // once the WASM kernel is live.
    await browser.waitUntil(
      async () =>
        (await browser.execute(() => window.__hew_test?.isReady() === true)) === true,
      { timeout: 30_000, timeoutMsg: 'harness/kernel never became ready' },
    )

    const result = await browser.execute(() => {
      const h = window.__hew_test!
      h.setCamera({ position: [8, 6, 8], target: [1, 1, 1], up: [0, 0, 1], fovDeg: 45 })
      const before = h.getObjectCount()
      h.drawBox([0, 0, 0], [2, 2, 0], 2) // 2×2×2, top face at z=2
      const pick = h.pickFace([1, 1, 10], [0, 0, -1]) // ray down onto the top face
      h.pushPull(pick!.object, pick!.face, 1) // → z=3
      const hashBefore = h.getStateHash()
      // Save → reopen through the app's real Open path; the doc must come back identical.
      h.load(h.save())
      return { before, after: h.getObjectCount(), hashBefore, hashAfter: h.getStateHash() }
    })

    expect(result.before).toBe(0)
    expect(result.after).toBe(1)
    expect(result.hashAfter).toBe(result.hashBefore) // save/reload round-trip is lossless
  })
})
