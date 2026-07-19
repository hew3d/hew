import { test, expect, type Page } from '@playwright/test'
import * as THREE from 'three'
import { worldToPagePixel, type Mat4, type CanvasRect } from './helpers/projectWorldToScreen'

/**
 *  Follow-up: — the hover dock must not get "stuck" showing the
 * Sketch verb row after the hovered sketch is undone. Two mechanisms were
 * suspected (see Viewport.tsx `updateSketchHover`/`reevaluateHoverNow`):
 *
 *   (a) hover is only re-evaluated inside `onPointerMove` — a stationary
 *       cursor + Cmd+Z produces no repick, so the dock is stale until the
 *       mouse happens to move again. CONFIRMED — see the first test below,
 *       which reproduces it with zero pointer movement after `undo()`.
 *   (b) a persistent wedge beyond (a), where even moving the cursor between
 *       ground and other sketches never un-sticks the dock. NOT reproduced —
 *       see the second test: a real `pointermove` after `undo()` already
 *       toggles the dock correctly (both before and after the (a) fix), and
 *       repeated ground/sketch/ground transitions after a second sketch is
 *       drawn keep working. So (a) is the whole mechanism here; there is no
 *       separate wedge to chase.
 *
 * Drives real `pointermove` events (`page.mouse.move`) over projected world
 * points under a pinned top-down camera (docs/DEVELOPMENT.md strategy 2), reading
 * the dock's context from `data-dock-context` (ContextualDock.tsx).
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

// Near-top-down camera over the ground plane (where free-standing sketches
// live) — makes world (x, y, 0) -> screen pixel arithmetic simple and keeps
// every point of interest comfortably inside the frustum (see comment at each
// use). Tilted a hair off straight-down on purpose: a perfectly polar pose
// sits on OrbitControls' pole, where free orbit's clamp (minPolarAngle =
// atan(POLE_TILT), Viewport.tsx) would rotate it ~0.001 rad off-axis on the
// next update — moving position.y by ~0.015 and failing the pin below. This
// tilt keeps the pose well inside the clamp, and pixelFor projects from the
// same pose, so the world→pixel arithmetic stays exact.
const CAMERA = {
  position: [0, -0.1, 15] as [number, number, number],
  target: [0, 0, 0] as [number, number, number],
  up: [0, 1, 0] as [number, number, number],
  fovDeg: 50,
}

function buildViewProjection(aspect: number): Mat4 {
  const camera = new THREE.PerspectiveCamera(CAMERA.fovDeg, aspect, 0.01, 100)
  camera.position.set(...CAMERA.position)
  camera.up.set(...CAMERA.up)
  camera.lookAt(CAMERA.target[0], CAMERA.target[1], CAMERA.target[2])
  camera.updateMatrixWorld(true)
  camera.updateProjectionMatrix()
  return camera.projectionMatrix.clone().multiply(camera.matrixWorldInverse).elements as unknown as Mat4
}

/** Shared per-test rig: pin the camera, project world points to page pixels,
 * and poll the dock's `data-dock-context` attribute. */
async function setupHoverRig(page: Page) {
  const canvas = page.locator('canvas').first()
  const box = await canvas.boundingBox()
  if (box === null) throw new Error('canvas has no bounding box')
  const rect: CanvasRect = { left: box.x, top: box.y, width: box.width, height: box.height }
  const vp = buildViewProjection(rect.width / rect.height)

  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)

  function pixelFor(world: [number, number, number]): { x: number; y: number } {
    const p = worldToPagePixel({ x: world[0], y: world[1], z: world[2] }, vp, rect)
    if (p === null) throw new Error(`world point ${world} projects behind the pinned camera`)
    return p
  }

  // Real pointermove events, spaced past SketchHoverGate's 100ms poll
  // throttle so each move is guaranteed to trigger a fresh wasm re-pick
  // rather than being silently swallowed by the throttle window.
  async function moveTo(pt: { x: number; y: number }): Promise<void> {
    await page.mouse.move(pt.x, pt.y)
    await page.waitForTimeout(150)
    await page.mouse.move(pt.x + 1, pt.y)
  }

  async function expectDockContext(expected: string): Promise<void> {
    await page.waitForFunction(
      (want) =>
        document.querySelector('[data-testid="contextual-dock"]')?.getAttribute('data-dock-context') === want,
      expected,
      { timeout: 3_000 },
    )
  }

  async function dockContextNow(): Promise<string | null> {
    return page.evaluate(
      () => document.querySelector('[data-testid="contextual-dock"]')?.getAttribute('data-dock-context') ?? null,
    )
  }

  /** Assert the pinned camera has NOT been reframed behind the rig's back.
   * Drawing a sketch — even the first one into an empty document — must
   * leave the camera alone (the first-sketch auto-zoom was removed by
   * maintainer decision: the welcome unit choice sets the initial framing),
   * so `pixelFor`'s projections stay valid without any re-pinning. */
  async function expectCameraStillPinned(): Promise<void> {
    // Two frames: were a reframe still scheduled (the old behavior queued
    // it on the next animation frame), it would have landed by now.
    await page.evaluate(
      () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
    )
    const cam = await page.evaluate(() => window.__hew_test!.getCamera())
    // Sub-millimeter slack: the setCamera→getCamera round-trip carries a
    // little controls noise, while the removed auto-zoom moved the camera
    // by whole meters (re-targeting the sketch center).
    for (let i = 0; i < 3; i++) {
      expect(cam.position[i]).toBeCloseTo(CAMERA.position[i], 3)
      expect(cam.target[i]).toBeCloseTo(CAMERA.target[i], 3)
    }
  }

  return { pixelFor, moveTo, expectDockContext, dockContextNow, expectCameraStillPinned }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
})

test('hover dock updates immediately after undo even with NO further pointer movement (mechanism a)', async ({ page }) => {
  const { pixelFor, moveTo, expectDockContext, dockContextNow } = await setupHoverRig(page)

  // A free-standing sketch, drawn straight through the kernel (harness
  // `drawRectangle`, like the other tool specs) so it carries no selection
  // side effect — the dock context here is driven purely by hover, matching
  // the user's repro ("hover it," not "select it").
  await page.evaluate(() => window.__hew_test!.drawRectangle([-1, -1, 0], [1, 1, 0]))

  const centerA = pixelFor([0, 0, 0])
  await moveTo(centerA)
  await expectDockContext('sketch')

  // Cmd+Z the sketch (same wasm path App.tsx's keyboard shortcut drives) —
  // deliberately WITHOUT moving the mouse afterward.
  await page.evaluate(() => window.__hew_test!.undo())

  // Before the fix this stayed 'sketch' indefinitely (no pointermove -> no
  // repoll). It must now flip back to 'empty' as soon as the document
  // mutation itself re-evaluates hover against the cursor's last position.
  await expectDockContext('empty')
  expect(await dockContextNow()).toBe('empty')
})

test('hover dock keeps toggling between ground and sketches after an undo (no wedge beyond mechanism a)', async ({ page }) => {
  const { pixelFor, moveTo, expectDockContext, expectCameraStillPinned } = await setupHoverRig(page)

  await page.evaluate(() => window.__hew_test!.drawRectangle([-1, -1, 0], [1, 1, 0]))

  const centerA = pixelFor([0, 0, 0])
  const ground = pixelFor([3, 3, 0])
  const centerB = pixelFor([3, -3, 0])

  await moveTo(centerA)
  await expectDockContext('sketch')

  await page.evaluate(() => window.__hew_test!.undo())

  // Move to open ground, away from any sketch.
  await moveTo(ground)
  await expectDockContext('empty')

  // Draw a second sketch elsewhere and confirm hover toggling still works.
  // The undo left the document visibly empty, and this draw used to fire
  // the (since-removed) first-sketch auto-zoom — the camera must now stay
  // exactly where the rig pinned it, keeping every projection valid.
  await page.evaluate(() => window.__hew_test!.drawRectangle([2, -4, 0], [4, -2, 0]))
  await expectCameraStillPinned()
  await moveTo(centerB)
  await expectDockContext('sketch')

  // ...and back to ground once more.
  await moveTo(ground)
  await expectDockContext('empty')
})
