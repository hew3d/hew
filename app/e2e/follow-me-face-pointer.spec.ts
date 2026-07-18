import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Follow Me — SOLID-FACE path, driven end to end with REAL pointer input.
 *
 * The maintainer's report was that molding around a solid's face (a profile
 * run around a tabletop's top) was flaky where a drawn-path sweep worked. The
 * kernel sweep is correct for both (pinned by document_specs.rs and the
 * API-level follow-me-face-path.spec.ts); the gap was the TOOL: a solid's
 * face cannot be preselected, so it must be clicked directly, and a click
 * that missed the face — or landed on the standing profile — was a silent
 * no-op or swept the wrong rim. This suite drives the real pointer through
 * the three things a user actually experiences:
 *
 *   1. MISS FEEDBACK — a path-stage click on nothing says what to aim at
 *      instead of doing nothing silently.
 *   2. WRONG-FACE REFUSAL — picking a face parallel to the profile is refused
 *      with FACE-specific copy ("that face is parallel to the profile"), not
 *      the generic drawn-path wording.
 *   3. SUCCESS — hovering previews the face-loop that will be swept, and a
 *      direct top-face click + profile click commits a watertight molding.
 *
 * The scene is built through the harness at meter scale (deterministic, and
 * NOT what is under test — a solid's face clicks the same whatever built it);
 * only the Follow Me interaction uses synthesized pointer events. Pixel
 * picking on the tiny 0.1 m maintainer fixture is not deterministic (its
 * File→Open path re-frames the camera on a rAF, and a 6 cm profile is a
 * handful of pixels), which is exactly why the fixture is covered at the
 * kernel/API layers; here the geometry is scaled up so a real click lands.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

interface Ctx {
  vp: Mat4
  rect: { left: number; top: number; width: number; height: number }
}

// A 3/4 view of a 2×1×0.2 m tabletop: the whole top face is clear on screen,
// the standing profile straddles the right (x = 2) rim, the front (y = 0) face
// is visible for the wrong-face pick, and there is empty ground to miss into.
const CAMERA: CameraParams = {
  position: { x: 4, y: -3, z: 3 },
  target: { x: 1, y: 0.5, z: 0.1 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

/** Wait for the harness and pin the camera (no file load → no rAF re-frame,
 *  so the pinned pose sticks and projected pixels are exact). */
async function aim(page: Page): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(
    (cam) =>
      window.__hew_test!.setCamera({
        position: [cam.position.x, cam.position.y, cam.position.z],
        target: [cam.target.x, cam.target.y, cam.target.z],
        up: [cam.up.x, cam.up.y, cam.up.z],
        fovDeg: cam.fovDeg,
      }),
    CAMERA,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(CAMERA, rect.width / rect.height), rect }
}

/**
 * Build the tabletop + a standing profile square. The flat square is folded
 * up 90° about the X line at (y = 0.5, z = 0.2) so its plane is y = 0.3
 * (normal +Y), z ∈ [0.1, 0.3], x ∈ [1.9, 2.1] — square across the top face's
 * x = 2 rim (which runs in Y) and hanging proud of it. Returns nothing kernel:
 * the whole point is that the tool finds the face and region by pointer.
 */
async function buildScene(page: Page): Promise<void> {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [2, 1, 0], 0.2)
    const prof = h.drawRectangle([1.9, 0.4, 0], [2.1, 0.6, 0])
    h.rotateSketch(prof.sketch, -90, [1, 0, 0], [0, 0.5, 0.2])
  })
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
}

async function activateFollowMe(page: Page): Promise<void> {
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  // Gate on the tool being live before touching the canvas — a click racing
  // the tool switch would drive the outgoing tool.
  await expect(page.getByText('Click the path to follow')).toBeVisible()
}

async function moveTo(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 4, p.y - 4) // a fresh move that isn't coalesced away
  await page.waitForTimeout(40)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(120)
}

async function clickAt(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  await moveTo(page, ctx, x, y, z)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

test('Follow Me: a path-stage click on nothing says what to aim at (no silent no-op)', async ({
  page,
}) => {
  const ctx = await aim(page)
  await buildScene(page)
  await activateFollowMe(page)

  // Hover the flat top face first — the sweep target is previewed before any
  // click (the face-loop highlight; its geometry is unit-tested). Then click
  // empty ground: the tool must not sit silent.
  await moveTo(page, ctx, 0.5, 0.5, 0.2)
  await clickAt(page, ctx, 0.5, -1, 0) // empty ground in front of the tabletop

  await expect(page.getByText('Click the flat face to run the profile around it', { exact: false })).toBeVisible()
  // Nothing was built — a miss builds nothing.
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
})

test('Follow Me: a face parallel to the profile is refused with face-specific guidance', async ({
  page,
}) => {
  const ctx = await aim(page)
  await buildScene(page)
  await activateFollowMe(page)

  // Pick the FRONT face (y = 0) — parallel to the profile's plane, so no rim of
  // it is perpendicular to the profile. Then click the profile to attempt the
  // sweep.
  await clickAt(page, ctx, 1, 0, 0.1)
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
  await clickAt(page, ctx, 2, 0.3, 0.2)

  // Face-context copy, not the generic drawn-path perpendicularity message.
  await expect(page.getByText('That face is parallel to the profile', { exact: false })).toBeVisible()
  await expect(page.getByText('[ProfileNotPerpendicular]')).toBeVisible()
  // Refused → the tabletop stands alone, untouched.
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
})

test("Follow Me refuses a component instance's face (frame guard, real pointer)", async ({
  page,
}) => {
  const ctx = await aim(page)
  // A plain box folded into a component: its top face is now instanced
  // (definition-local geometry + a separate placement pose), which
  // face_boundary/follow_me_around_face — taking only (object, face) — cannot
  // place. The Viewport wires faceDrawEligible into Follow Me, so this must be
  // refused rather than swept in the wrong coordinate frame.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([0, 0, 0], [2, 1, 0], 0.2)
    h.makeComponent([id])
  })
  await activateFollowMe(page)

  await clickAt(page, ctx, 0.5, 0.5, 0.2) // click the instanced top face
  await expect(page.getByText('belongs to a component', { exact: false })).toBeVisible()
  // No path was locked — the tool is still choosing a path, nothing swept.
  await expect(page.getByText('Click the path to follow')).toBeVisible()
})

test('Follow Me: hovering then clicking the top face molds a watertight solid around it', async ({
  page,
}) => {
  const ctx = await aim(page)
  await buildScene(page)
  await activateFollowMe(page)

  // Hover the top face — the face-loop that will be swept previews under the
  // cursor — then click it directly to pick the path.
  await moveTo(page, ctx, 0.5, 0.5, 0.2)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()

  // Click the standing profile → immediate commit.
  await clickAt(page, ctx, 2, 0.3, 0.2)
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2)

  // A separate, watertight molding; the tabletop is left untouched.
  const after = await page.evaluate(() => {
    const h = window.__hew_test!
    return { solids: h.getObjectIds().map((id) => h.isObjectSolid(id)) }
  })
  expect(after.solids).toEqual([true, true])
  // No refusal toast on the successful path.
  await expect(page.getByText('[ProfileNotPerpendicular]')).toHaveCount(0)
})
