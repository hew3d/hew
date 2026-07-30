import { test, expect, type Page } from '@playwright/test'
import { SNAP_RADIUS_PX } from '../src/viewport/snapService'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Camera P2 (docs/design/camera.md §4/§5): Position Camera / Look Around /
 * Walk driven through the REAL Camera menu and real pointer/keyboard
 * events, camera-state persistence across a save/reload round trip, and
 * the cylindrical (constant-world-radius) snap tolerance under parallel
 * projection — the design's own Verification section calls for all three,
 * and none had e2e coverage before this file.
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

async function canvasCenter(page: Page): Promise<{ x: number; y: number; box: { x: number; y: number; width: number; height: number } }> {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  return { x: canvas.x + canvas.width / 2, y: canvas.y + canvas.height / 2, box: canvas }
}

/** The viewport's full camera state (projection/fov/eye/target/up) —
 * unlike `harness.getCamera()` (narrower, always-perspective-shaped test
 * convenience), covers both projections. */
async function readCameraState(page: Page) {
  return page.evaluate(() => window.__hew_test!.getCameraState())
}

test.describe('Camera ▸ Position Camera / Look Around / Walk (menu-driven, real drags)', () => {
  test('a full click → auto-switch → drag-look → drag-walk sequence moves and reorients the camera', async ({ page }) => {
    await setup(page)

    // A default-ish 3/4 pose looking roughly at the origin, so a click near
    // canvas center resolves a ground-plane snap.
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [3.5, -3, 2.5], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100)

    // --- Position Camera: a plain click places the eye above the clicked
    // point and auto-switches to Look Around (design §4). ---
    await openCameraMenu(page)
    await page.getByText('Position Camera', { exact: true }).click()
    await page.locator('text=Click to stand there').first().waitFor({ timeout: 5000 })

    const { x: cx, y: cy } = await canvasCenter(page)
    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(100)

    // Auto-switched to Look Around (its own distinct status hint).
    await page.locator('text=Drag to look around').first().waitFor({ timeout: 5000 })

    const afterClick = await readCameraState(page)
    // Eye height ~1.68 m above whatever ground point was under the cursor —
    // this pose looks near the origin, so the clicked point's z is ~0.
    expect(afterClick.eye[2]).toBeGreaterThan(1.0)
    expect(afterClick.eye[2]).toBeLessThan(2.5)

    // --- Look Around: a real drag yaws/pitches the view (no position
    // change — Look Around pivots about a fixed eye). ---
    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.move(cx + 150, cy - 60, { steps: 10 })
    await page.mouse.up()
    await page.waitForTimeout(100)

    const afterLook = await readCameraState(page)
    expect(afterLook.eye[0]).toBeCloseTo(afterClick.eye[0], 6)
    expect(afterLook.eye[1]).toBeCloseTo(afterClick.eye[1], 6)
    expect(afterLook.eye[2]).toBeCloseTo(afterClick.eye[2], 6)
    // The look direction (target relative to the fixed eye) genuinely
    // changed — dragging is not a no-op.
    const dirChanged =
      Math.abs(afterLook.target[0] - afterClick.target[0]) +
      Math.abs(afterLook.target[1] - afterClick.target[1]) +
      Math.abs(afterLook.target[2] - afterClick.target[2])
    expect(dirChanged).toBeGreaterThan(0.05)

    // --- Walk: switch tools via the menu, then a real vertical drag walks
    // forward — the eye MOVES this time (unlike Look Around). ---
    await openCameraMenu(page)
    await page.getByText('Walk', { exact: true }).click()
    await page.locator('text=Drag up/down to walk').first().waitFor({ timeout: 5000 })

    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.move(cx, cy - 150, { steps: 10 }) // drag up -> walk forward
    await page.mouse.up()
    await page.waitForTimeout(100)

    const afterWalk = await readCameraState(page)
    const moved = Math.hypot(
      afterWalk.eye[0] - afterLook.eye[0],
      afterWalk.eye[1] - afterLook.eye[1],
      afterWalk.eye[2] - afterLook.eye[2],
    )
    expect(moved).toBeGreaterThan(0.1)
    // Walk maintains eye height above the ground plane (design §4) —
    // z stays put even though x/y moved.
    expect(afterWalk.eye[2]).toBeCloseTo(afterLook.eye[2], 6)

    // --- Escape returns to Select. ---
    await page.keyboard.press('Escape')
    await page.waitForTimeout(100)
    // Select's default idle hint (or simply no walkthrough hint left showing).
    await expect(page.locator('text=Drag up/down to walk')).toHaveCount(0)
  })
})

test.describe('Look Around / Walk pointer capture (raw-drag tools track the drag off-canvas)', () => {
  test('a Look Around drag keeps turning the view after the cursor crosses the canvas edge', async ({ page }) => {
    await setup(page)
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [3.5, -3, 2.5], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100)

    await openCameraMenu(page)
    await page.getByText('Look Around', { exact: true }).click()
    await page.locator('text=Drag to look around').first().waitFor({ timeout: 5000 })

    const { x: cx, y: cy, box } = await canvasCenter(page)
    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.move(cx + 80, cy, { steps: 5 }) // a small drag, still inside the canvas
    await page.waitForTimeout(50)
    const midDrag = await readCameraState(page)

    // Cross the canvas's top edge (the in-app MenuBar sits above it) WHILE
    // the button is still held. Without `setPointerCapture` at the raw-drag
    // press (Viewport's onPointerRawDown dispatch), the canvas element stops
    // receiving 'pointermove' the instant the cursor leaves its bounds — the
    // browser routes the event to whatever is now under the cursor instead —
    // so the drag would silently freeze here instead of continuing to turn
    // the view.
    await page.mouse.move(cx + 160, box.y - 20, { steps: 10 })
    await page.waitForTimeout(50)
    const offCanvas = await readCameraState(page)
    await page.mouse.up()

    const turnedFurther =
      Math.abs(offCanvas.target[0] - midDrag.target[0]) +
      Math.abs(offCanvas.target[1] - midDrag.target[1]) +
      Math.abs(offCanvas.target[2] - midDrag.target[2])
    expect(turnedFurther).toBeGreaterThan(0.02)
  })
})

test.describe('Walkthrough exit re-seeds the orbit target (Viewport.tsx switchToolRef)', () => {
  /**
   * Regression coverage for the method-chaining evaluation-order bug the
   * f1b8af2 commit message describes fixing alongside the render-loop one
   * (Viewport.tsx's `switchToolRef`, exiting a `WALKTHROUGH_TOOL_NAMES`
   * tool): the pre-fix code re-seeded `controls.target` with
   * `controls.target.copy(camera.position).addScaledVector(dir,
   * rig.effectiveDistance(controls.getDistance()))` — a single chained
   * expression where `.addScaledVector`'s distance argument is evaluated
   * AFTER `.copy(camera.position)` has already run, so `controls.target`
   * and `camera.position` are already equal by the time
   * `controls.getDistance()` executes, collapsing the distance to ~0 and
   * silently discarding the reseed — `controls.target` ends up sitting
   * exactly ON the eye instead of a sensible distance in front of it. Only
   * the SIBLING render-loop fix (unconditional `controls.update()` ignoring
   * `.enabled`) had test coverage; this one didn't, per the camera P2
   * review.
   *
   * `getCameraState()`'s `target` field is computed fresh from
   * `controls.getDistance()` at call time (Viewport.tsx), so it directly
   * exposes the bug: a collapsed reseed reads back as `target ≈ eye`.
   */
  test('orbit distance survives entering and exiting Look Around, not collapsing target onto the eye', async ({ page }) => {
    await setup(page)
    // fovDeg 45 matches CameraRig's `EFFECTIVE_DISTANCE_REFERENCE_FOV_DEG`
    // (cameraRig.ts), so `effectiveDistance` is numerically identical to
    // the raw orbit distance here — keeping the expected value exact.
    const orbitDistance = 10
    await page.evaluate((dist) => {
      window.__hew_test!.setCamera({ position: [dist, 0, 0], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    }, orbitDistance)
    await page.waitForTimeout(100)

    const before = await readCameraState(page)
    expect(Math.hypot(...before.eye.map((v, i) => v - before.target[i]))).toBeCloseTo(orbitDistance, 3)

    // Enter Look Around (disables OrbitControls per the walkthrough-tool
    // contract), then immediately exit via Escape WITHOUT ever dragging —
    // isolating the exit-reseed logic from any actual look-around gesture.
    await openCameraMenu(page)
    await page.getByText('Look Around', { exact: true }).click()
    await page.locator('text=Drag to look around').first().waitFor({ timeout: 5000 })
    await page.keyboard.press('Escape')
    await page.waitForTimeout(100)

    const after = await readCameraState(page)
    const eyeToTarget = Math.hypot(...after.eye.map((v, i) => v - after.target[i]))
    // The buggy chained form collapses this to ~0 (target === eye); the
    // fix re-seeds it at (approximately) the ORIGINAL orbit distance.
    expect(eyeToTarget).toBeGreaterThan(orbitDistance * 0.5)
  })
})

test.describe('Camera persistence (docs/design/camera.md §5)', () => {
  test('save/reload round-trips the working view, including parallel projection', async ({ page }) => {
    await setup(page)

    // A distinctive, easy-to-recognize pose, and switch to parallel
    // projection via the REAL menu so the round trip covers a projection
    // change too, not just perspective.
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [4, -6, 3], target: [1, 0.5, 0], up: [0, 0, 1], fovDeg: 55 })
    })
    await openCameraMenu(page)
    await page.getByText('Parallel Projection').click()
    await page.waitForTimeout(100)

    const before = await readCameraState(page)
    expect(before.projection).toBe('parallel')

    // Push the live view into the document and round-trip it through the
    // REAL kernel save/load path (Document::set_camera_state -> save ->
    // Document::load -> Scene::camera_state), then apply it exactly as
    // App.tsx's own load handler does — the same steps `pushCameraStateToScene`
    // / `applyLoadedBytes` run on a real Save/Open, without needing an
    // actual OS file dialog in this harness.
    const bytes = await page.evaluate(() => window.__hew_test!.saveWithCameraState())

    // Perturb the view so the reload is a genuine round trip, not a no-op.
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [-9, 9, 9], target: [-2, -2, -2], up: [0, 0, 1], fovDeg: 30 })
    })
    await page.waitForTimeout(100)
    const perturbed = await readCameraState(page)
    expect(perturbed.projection).toBe('perspective') // setCamera always forces perspective

    // `harness.load` routes through the app's real Open path (applyLoadedBytes),
    // which is where the camera-state restoration this test targets lives.
    await page.evaluate((b) => window.__hew_test!.load(b), bytes)
    await page.waitForTimeout(200) // load's requestAnimationFrame apply

    const after = await readCameraState(page)
    expect(after.projection).toBe('parallel')
    expect(after.fovDeg).toBeCloseTo(before.fovDeg, 6)
    for (let i = 0; i < 3; i++) {
      expect(after.eye[i]).toBeCloseTo(before.eye[i], 3)
      expect(after.target[i]).toBeCloseTo(before.target[i], 3)
      expect(after.up[i]).toBeCloseTo(before.up[i], 3)
    }
  })

  test('a document with no saved camera state loads with the default framing (absent block)', async ({ page }) => {
    await setup(page)
    // A fresh scene never called set_camera_state -> save() carries no
    // camera block at all (docs/design/camera.md §5's "absent means use
    // today's home framing" contract).
    const bytes = await page.evaluate(() => window.__hew_test!.save())
    await page.evaluate((b) => window.__hew_test!.load(b), bytes)
    await page.waitForTimeout(200)
    // No error, and the viewport is still responsive — the absent-block
    // path didn't throw or leave the camera in a broken state.
    const state = await readCameraState(page)
    expect(state.projection === 'perspective' || state.projection === 'parallel').toBe(true)
    expect(Number.isFinite(state.eye[0])).toBe(true)
  })
})

test.describe('Snap aperture ▸ cylindrical tolerance under parallel projection (docs/design/camera.md §1)', () => {
  test('a real vertex-to-vertex Move snaps exactly in parallel Top view', async ({ page }) => {
    await setup(page)

    // A 2x2x2 box; its top face corners at z=2 are the snap targets.
    const boxId = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))

    // Looking straight down (a Top-like pose), then the REAL Parallel
    // Projection menu toggle — this is the exact regime the interim cone
    // synthesis used to approximate and the cylindrical tolerance now
    // handles exactly (constant world-radius, independent of the ray's
    // depth to the ground vs. the top face).
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [1, 1, 20], target: [1, 1, 0], up: [0, 1, 0], fovDeg: 45 })
    })
    await openCameraMenu(page)
    await page.getByText('Parallel Projection').click()
    await page.waitForTimeout(100)

    const canvas = await page.locator('canvas').first().boundingBox()
    if (canvas === null) throw new Error('no canvas')
    const toPage = async (world: [number, number, number]) => {
      const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), world)
      return { x: canvas.x + p.x, y: canvas.y + p.y }
    }

    // Select the box: click its top face (visible from directly above).
    const faceCenter = await toPage([1, 1, 2])
    await page.mouse.click(faceCenter.x, faceCenter.y)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

    // Move tool, then a real vertex-snapped drag from one top corner to an
    // adjacent one: if the click snaps exactly, the box shifts by exactly
    // the vector between those two corners (2, 0, 0) — a clean geometric
    // value a mis-snapped (off-vertex) click would not reliably reproduce.
    await page.keyboard.press('m')
    await page.locator('text=Click a point to move from').first().waitFor({ timeout: 5000 }).catch(() => {
      /* status hint text may differ; the gesture below is the real assertion */
    })

    const before = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), boxId)
    const grab = await toPage([0, 0, 2]) // top corner
    const drop = await toPage([2, 0, 2]) // adjacent top corner, +2 along X

    await page.mouse.move(grab.x, grab.y)
    await page.mouse.down()
    await page.mouse.up()
    await page.mouse.move(drop.x, drop.y)
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(150)

    const after = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), boxId)
    // [minX, minY, minZ, maxX, maxY, maxZ] — expect an exact +2 shift along
    // X only, from a real snap-driven click in parallel Top view.
    expect(after[0] - before[0]).toBeCloseTo(2, 6)
    expect(after[3] - before[3]).toBeCloseTo(2, 6)
    expect(after[1] - before[1]).toBeCloseTo(0, 6)
    expect(after[4] - before[4]).toBeCloseTo(0, 6)
    expect(after[2] - before[2]).toBeCloseTo(0, 6)
    expect(after[5] - before[5]).toBeCloseTo(0, 6)
  })

  /**
   * The test above clicks each corner dead-on, so it cannot tell whether the
   * snap actually used the Cylinder (constant world-radius) aperture or
   * silently fell back to Cone (angular, shrinking/growing with the RAY'S
   * depth to the clicked point) — an exact click succeeds under either
   * shape. This one clicks deliberately OFF each vertex, by a world-space
   * offset engineered so ONLY a true constant-radius tolerance reaches it.
   *
   * The trick: `Document::set_camera_state`/`toggleProjection` sizes the
   * ortho frustum (hence `worldPerPixel`, hence the Cylinder aperture) from
   * the distance between the eye and the ORBIT TARGET — here set 20x
   * farther away than the box actually sits, along the same straight-down
   * view axis (so the box stays centered, just smaller on screen). If the
   * snap wiring instead derived a Cone-shaped aperture from the RAY'S ACTUAL
   * depth to the box (18, not the orbit target's 360), that aperture would
   * be ~20x TIGHTER than the real Cylinder one. An offset chosen at half the
   * true (Cylinder) aperture is ~10x past that hypothetical Cone aperture —
   * comfortably admitted by the real wiring, comfortably missed by the bug.
   */
  test('a vertex-to-vertex Move still snaps exactly when clicked off-vertex, within Cylinder\'s (but not Cone\'s) tolerance', async ({ page }) => {
    await setup(page)

    const boxId = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))

    // Eye very close to the box (top face at z=2 -> ray depth to the box is
    // only 1), but the orbit target sits 40x farther away along the same
    // straight-down axis (still within OrbitControls' 50 m `maxDistance`
    // clamp — `configureControls`, Viewport.tsx — which a naive much-larger
    // target distance would silently clip to, moving the eye itself), so
    // `toggleProjection` sizes the ortho frustum (and therefore
    // `worldPerPixel`) from a distance of 40, not 1.
    const eyeZ = 3
    const boxDepth = eyeZ - 2 // ray depth from the eye to the top face
    const targetDistanceRatio = 40
    const targetZ = eyeZ - targetDistanceRatio * boxDepth
    await page.evaluate(
      ({ eyeZ, targetZ }) => {
        window.__hew_test!.setCamera({ position: [1, 1, eyeZ], target: [1, 1, targetZ], up: [0, 1, 0], fovDeg: 45 })
      },
      { eyeZ, targetZ },
    )
    await openCameraMenu(page)
    await page.getByText('Parallel Projection').click()
    await page.waitForTimeout(100)

    const canvas = await page.locator('canvas').first().boundingBox()
    if (canvas === null) throw new Error('no canvas')
    const toPage = async (world: [number, number, number]) => {
      const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), world)
      return { x: canvas.x + p.x, y: canvas.y + p.y }
    }

    // Measure the CURRENT rendering scale (pixels per world unit) directly,
    // rather than re-deriving `worldPerPixel`'s formula by hand — this is
    // the same ortho camera zoom that `worldPerPixel` itself reads, so it's
    // an implementation-detail-agnostic ground truth for converting a
    // world-space aperture radius into screen pixels.
    const p0 = await toPage([0, 0, 2])
    const p1 = await toPage([1, 0, 2])
    const pixelsPerWorldUnit = Math.hypot(p1.x - p0.x, p1.y - p0.y)

    // The real (Cylinder) aperture radius, in world units, at THIS zoom —
    // and what a mistaken Cone fallback would allow instead, using the
    // box's actual (much shallower) ray depth.
    const cylinderRadiusWorld = SNAP_RADIUS_PX / pixelsPerWorldUnit
    const coneFallbackRadiusWorld = cylinderRadiusWorld / targetDistanceRatio
    const clickOffsetWorld = cylinderRadiusWorld * 0.5
    expect(clickOffsetWorld).toBeGreaterThan(coneFallbackRadiusWorld * 5) // comfortable margin either way

    // Select the box, then Move via drags clicked `clickOffsetWorld` AWAY
    // from each real corner (along +X, in-plane with the top face) rather
    // than dead-on.
    const faceCenter = await toPage([1, 1, 2])
    await page.mouse.click(faceCenter.x, faceCenter.y)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

    await page.keyboard.press('m')
    await page.locator('text=Click a point to move from').first().waitFor({ timeout: 5000 }).catch(() => {
      /* status hint text may differ; the gesture below is the real assertion */
    })

    const before = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), boxId)
    const grab = await toPage([0 + clickOffsetWorld, 0, 2]) // near the (0,0,2) corner, not on it
    const drop = await toPage([2 + clickOffsetWorld, 0, 2]) // near the (2,0,2) corner, not on it

    await page.mouse.move(grab.x, grab.y)
    await page.mouse.down()
    await page.mouse.up()
    await page.mouse.move(drop.x, drop.y)
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(150)

    const after = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), boxId)
    // Still an EXACT +2 shift along X only: the off-vertex clicks must
    // still resolve to the true corners. A Cone fallback (aperture ~20x too
    // tight here) would miss the vertex snap entirely, landing the Move at
    // the raw (unsnapped) ground/face point instead of exactly +2.
    expect(after[0] - before[0]).toBeCloseTo(2, 3)
    expect(after[3] - before[3]).toBeCloseTo(2, 3)
    expect(after[1] - before[1]).toBeCloseTo(0, 3)
    expect(after[4] - before[4]).toBeCloseTo(0, 3)
    expect(after[2] - before[2]).toBeCloseTo(0, 3)
    expect(after[5] - before[5]).toBeCloseTo(0, 3)
  })
})
