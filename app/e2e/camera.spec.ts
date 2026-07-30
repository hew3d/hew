import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Camera P1 (docs/design/camera.md): Parallel Projection toggle and Zoom
 * Window, driven through the REAL Camera menu and real pointer/keyboard
 * events — the design's own Verification section calls for both, and
 * neither had any e2e coverage before this file.
 */

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

async function openCameraMenu(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Camera' }).click()
}

/** Project two world points differing only in DEPTH (same lateral offset,
 * different distance along the view axis) and return their screen-x
 * separation — a projection-sensitive ground truth: under perspective this
 * separation shrinks with depth (foreshortening); under TRUE orthographic
 * projection it is depth-INVARIANT (no perspective divide), so the two
 * points land at the SAME screen x regardless of how far apart they are
 * along the view axis. */
async function depthPairScreenXGap(page: Page): Promise<number> {
  const [near, far] = await page.evaluate(() => {
    const h = window.__hew_test!
    return [h.worldToScreen([2, 0, 0]), h.worldToScreen([2, 5, 0])]
  })
  expect(near.behind).toBe(false)
  expect(far.behind).toBe(false)
  return Math.abs(near.x - far.x)
}

test.describe('Camera ▸ Parallel Projection (menu-driven)', () => {
  test('toggling to parallel renders true (depth-invariant) orthographic projection; toggling back restores perspective AND live OrbitControls input', async ({ page }) => {
    await setup(page)

    // Pin a known perspective pose: eye on the -Y axis looking at the
    // origin, +Z up — so X is the pure "lateral" (screen-horizontal) axis
    // and Y is pure depth, matching depthPairScreenXGap's assumption.
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [0, -10, 0], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100) // let the render loop catch up to setCamera

    const perspectiveGap = await depthPairScreenXGap(page)
    // Perspective foreshortens: the far point's apparent lateral offset is
    // measurably smaller than the near point's — a nontrivial gap.
    expect(perspectiveGap).toBeGreaterThan(5)

    await openCameraMenu(page)
    // Not `exact: true`: once checked, the item's rendered text is
    // "✓Parallel Projection" (the checkmark glyph shares the label's text
    // node — see `CheckMenuItem` — so no element's text is EVER exactly
    // "Parallel Projection" once it's toggled on). Substring match is
    // unambiguous — no other Camera-menu entry contains this text.
    await page.getByText('Parallel Projection').click()
    await page.waitForTimeout(100)

    const parallelGap = await depthPairScreenXGap(page)
    // True orthographic: the same world-x offset projects to the SAME
    // screen-x regardless of depth. Before the projection-correct fix,
    // `worldToPixels`'s `behind` heuristic (not this gap) was the bug, but
    // this assertion is the one the design's Verification section calls
    // for — a programmatic ground truth, not eyeballing.
    expect(parallelGap).toBeLessThan(1)

    // Toggle back — and prove OrbitControls is still LIVE (the setCamera/
    // toggleProjection control-rebind bug froze input permanently here).
    await openCameraMenu(page)
    // Not `exact: true`: once checked, the item's rendered text is
    // "✓Parallel Projection" (the checkmark glyph shares the label's text
    // node — see `CheckMenuItem` — so no element's text is EVER exactly
    // "Parallel Projection" once it's toggled on). Substring match is
    // unambiguous — no other Camera-menu entry contains this text.
    await page.getByText('Parallel Projection').click()
    await page.waitForTimeout(100)

    const perspectiveGapAgain = await depthPairScreenXGap(page)
    expect(perspectiveGapAgain).toBeGreaterThan(5)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const canvas = await page.locator('canvas').first().boundingBox()
    if (canvas === null) throw new Error('no canvas')
    const cx = canvas.x + canvas.width / 2
    const cy = canvas.y + canvas.height / 2
    // A real middle-drag orbit (OrbitControls' MIDDLE = ROTATE binding).
    await page.mouse.move(cx, cy)
    await page.mouse.down({ button: 'middle' })
    await page.mouse.move(cx + 120, cy + 40, { steps: 8 })
    await page.mouse.up({ button: 'middle' })
    await page.waitForTimeout(100)
    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(
      Math.hypot(
        after.position[0] - before.position[0],
        after.position[1] - before.position[1],
        after.position[2] - before.position[2],
      ),
    ).toBeGreaterThan(0.05)
  })

  test('setCamera (the harness path every e2e/pixel test drives) rebinds OrbitControls when called while parallel projection is active', async ({ page }) => {
    await setup(page)

    // Get into parallel projection via the real menu FIRST. The prior test
    // above only ever calls `setCamera` on a fresh, still-perspective page —
    // it never exercises the branch inside `setCamera` that fires when
    // `rig.projection === 'parallel'` (the force-back-to-perspective branch
    // that must also rebuild OrbitControls). This test starts from parallel
    // so that branch actually runs.
    await openCameraMenu(page)
    await page.getByText('Parallel Projection').click()
    await page.waitForTimeout(100)

    // The harness path every e2e/pixel test uses to pin a deterministic pose.
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [0, -10, 0], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100)

    // setCamera always forces perspective: true foreshortening, not
    // depth-invariant orthographic projection, should be back.
    const gap = await depthPairScreenXGap(page)
    expect(gap).toBeGreaterThan(5)

    // The actual discriminator: the render-loop's `camera` binding is
    // correctly reassigned to the new perspective camera by
    // `rig.toggleProjection` regardless of the fix, so the projection
    // assertion above passes either way. Only OrbitControls' own `.object`
    // — rebuilt by `rebindControlsForProjectionChange`, which the pre-fix
    // `setCamera` never called — determines whether a real orbit drag can
    // still move the camera afterward.
    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const canvas = await page.locator('canvas').first().boundingBox()
    if (canvas === null) throw new Error('no canvas')
    const cx = canvas.x + canvas.width / 2
    const cy = canvas.y + canvas.height / 2
    await page.mouse.move(cx, cy)
    await page.mouse.down({ button: 'middle' })
    await page.mouse.move(cx + 120, cy + 40, { steps: 8 })
    await page.mouse.up({ button: 'middle' })
    await page.waitForTimeout(100)
    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(
      Math.hypot(
        after.position[0] - before.position[0],
        after.position[1] - before.position[1],
        after.position[2] - before.position[2],
      ),
    ).toBeGreaterThan(0.05)
  })
})

test.describe('Camera ▸ Zoom Window (menu-driven, real drag)', () => {
  test('dragging a rectangle reframes the camera onto it', async ({ page }) => {
    await setup(page)

    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [0, -10, 0], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100)
    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const beforeDistance = Math.hypot(
      before.position[0] - before.target[0],
      before.position[1] - before.target[1],
      before.position[2] - before.target[2],
    )

    await openCameraMenu(page)
    await page.getByText('Zoom Window', { exact: true }).click()

    const canvas = await page.locator('canvas').first().boundingBox()
    if (canvas === null) throw new Error('no canvas')
    const cx = canvas.x + canvas.width / 2
    const cy = canvas.y + canvas.height / 2
    const halfW = canvas.width * 0.2
    const halfH = canvas.height * 0.2

    await page.mouse.move(cx - halfW, cy - halfH)
    await page.mouse.down()
    await page.mouse.move(cx + halfW, cy + halfH, { steps: 10 })
    await page.mouse.up()
    await page.waitForTimeout(100)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    const afterDistance = Math.hypot(
      after.position[0] - after.target[0],
      after.position[1] - after.target[1],
      after.position[2] - after.target[2],
    )
    // A rectangle smaller than the viewport zooms IN (distance shrinks) —
    // the reframe the design's §3 formula predicts
    // (`scale = max(rectW/vpW, rectH/vpH)` applied to distance).
    expect(afterDistance).toBeLessThan(beforeDistance * 0.9)
    expect(afterDistance).toBeGreaterThan(beforeDistance * 0.2)
  })

  test('Escape cancels an in-flight drag without reframing, and Zoom Window stays armed for a fresh attempt (one-shot semantics)', async ({ page }) => {
    await setup(page)

    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [0, -10, 0], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100)
    const before = await page.evaluate(() => window.__hew_test!.getCamera())

    await openCameraMenu(page)
    await page.getByText('Zoom Window', { exact: true }).click()

    const canvas = await page.locator('canvas').first().boundingBox()
    if (canvas === null) throw new Error('no canvas')
    const cx = canvas.x + canvas.width / 2
    const cy = canvas.y + canvas.height / 2
    const halfW = canvas.width * 0.2
    const halfH = canvas.height * 0.2

    // Press, drag past the threshold, then Escape BEFORE releasing.
    await page.mouse.move(cx - halfW, cy - halfH)
    await page.mouse.down()
    await page.mouse.move(cx + halfW, cy + halfH, { steps: 10 })
    await page.keyboard.press('Escape')
    await page.mouse.up() // the aborted drag must treat this as a no-op
    await page.waitForTimeout(100)

    const afterEscape = await page.evaluate(() => window.__hew_test!.getCamera())
    // Component-wise toBeCloseTo, not toEqual — OrbitControls' damping
    // recomputes position every idle frame via a spherical round trip, which
    // isn't perfectly bit-stable even at rest; the abort itself touches
    // nothing, so this is about ruling out a REFRAME, not bit-exactness.
    for (let i = 0; i < 3; i++) {
      expect(afterEscape.position[i]).toBeCloseTo(before.position[i], 6)
      expect(afterEscape.target[i]).toBeCloseTo(before.target[i], 6)
    }
    expect(afterEscape.fovDeg).toBe(before.fovDeg)

    // Still in Zoom Window mode (one-shot semantics: only the RECTANGLE was
    // cancelled, not the whole gesture) — a fresh drag now DOES reframe.
    await page.mouse.move(cx - halfW, cy - halfH)
    await page.mouse.down()
    await page.mouse.move(cx + halfW, cy + halfH, { steps: 10 })
    await page.mouse.up()
    await page.waitForTimeout(100)

    const afterRetry = await page.evaluate(() => window.__hew_test!.getCamera())
    const beforeDistance = Math.hypot(
      before.position[0] - before.target[0],
      before.position[1] - before.target[1],
      before.position[2] - before.target[2],
    )
    const retryDistance = Math.hypot(
      afterRetry.position[0] - afterRetry.target[0],
      afterRetry.position[1] - afterRetry.target[1],
      afterRetry.position[2] - afterRetry.target[2],
    )
    expect(retryDistance).toBeLessThan(beforeDistance * 0.9)
  })
})
