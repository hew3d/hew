import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Section-plane E2E — the real-wiring checks the SceneRenderer/tool unit
 * tests can't reach, driven the way a user does: activate the tool from the
 * rail, click the WebGL canvas to place, press real keys. Two of these guard
 * regressions that were live bugs (a real browser was required to confirm the
 * fix): a component instance snapping back to unclipped on select (BLOCKER 1),
 * and section-tool Delete also destroying the document selection (BLOCKER 2).
 *
 * Observation is through `window.__hew_test`'s section accessors
 * (`getSectionState` / `getSectionRenderInfo`) — material/session state, not
 * pixels — plus `getObjectCount` for the destructive-Delete guard. Pixel
 * placement uses the pinned-camera projection helper, exactly like
 * input-pipeline.spec.ts.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

// Pinned pose framing the box the specs draw (centred on ~(1,1,1)).
const CAMERA: CameraParams = {
  position: { x: 8, y: 6, z: 8 },
  target: { x: 1, y: 1, z: 1 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

interface Ctx {
  vp: Mat4
  rect: { left: number; top: number; width: number; height: number }
}

function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

async function setup(page: Page): Promise<Ctx> {
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

/** Click the canvas at the pixel where `world` renders — move first so the
 * tool sees a hover (snap resolve) before the down, as a real hand produces. */
async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

function sectionRadio(page: Page) {
  return page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Section Plane' })
}

/** Activate the Section Plane tool via its rail button, then drop focus so a
 * later `keyboard.press` dispatches as a bare window key (not to the button). */
async function activateSectionTool(page: Page): Promise<void> {
  const radio = sectionRadio(page)
  await radio.click()
  await expect(radio).toHaveAttribute('aria-checked', 'true')
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
}

test('Section Plane is reachable from the Inspect rail group', async ({ page }) => {
  await setup(page)
  await expect(sectionRadio(page)).toBeVisible()
})

// ---------------------------------------------------------------------------
// BLOCKER 2 (destructive) — Delete removes ONLY the section, not the object.
// ---------------------------------------------------------------------------
test('Delete with the Section Plane tool active removes the section, never the selected object', async ({
  page,
}) => {
  const ctx = await setup(page)

  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))
  await page.evaluate((b) => window.__hew_test!.selectObjects([b]), box)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  // Place a section on the top face. The tool STAYS active (no spring-back),
  // and the box stays selected.
  await activateSectionTool(page)
  await clickWorld(page, ctx, 1, 1, 2)
  await page.waitForFunction(() => window.__hew_test!.getSectionState() !== null)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  // Delete must remove ONLY the section (the tool is active, so its
  // capturesKey guard makes the App-level Delete handler stand down).
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Delete')

  await page.waitForFunction(() => window.__hew_test!.getSectionState() === null)
  // The destructive regression: the previously-selected object survives.
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
})

// ---------------------------------------------------------------------------
// BLOCKER 1 — a component instance stays clipped through selection.
// ---------------------------------------------------------------------------
test('a component instance keeps the section clip when selected (does not render whole again)', async ({
  page,
}) => {
  const ctx = await setup(page)

  const instance = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 2, 0], 2)
    return h.makeComponent([box]).instance
  })

  // Place a horizontal section on empty ground — the clip is applied to
  // every solid material regardless of which side geometry sits on.
  await activateSectionTool(page)
  await clickWorld(page, ctx, 3, 3, 0)
  await page.waitForFunction(() => window.__hew_test!.getSectionState()?.active === true)

  // Batched (unselected) instance carries the clip; the widget overlay never does.
  const before = await page.evaluate(
    (id) => window.__hew_test!.getSectionRenderInfo('instance', id),
    instance,
  )
  expect(before.widget).toBe(true)
  expect(before.widgetClipCount).toBe(0)
  expect(before.nodeClipCount).toBe(1)

  // Select the instance → it materializes out of its batch. Regression: the
  // freshly built materials must STILL carry the clip (before the fix they
  // rendered whole/unclipped).
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), instance)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  const after = await page.evaluate(
    (id) => window.__hew_test!.getSectionRenderInfo('instance', id),
    instance,
  )
  expect(after.nodeClipCount).toBe(1)
  expect(after.widgetClipCount).toBe(0)
})

// ---------------------------------------------------------------------------
// Core clip: solids clip, overlays do not.
// ---------------------------------------------------------------------------
test('an active section clips the solid but not the widget overlay', async ({ page }) => {
  const ctx = await setup(page)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))

  await activateSectionTool(page)
  await clickWorld(page, ctx, 1, 1, 2)
  await page.waitForFunction(() => window.__hew_test!.getSectionState()?.active === true)

  const info = await page.evaluate((b) => window.__hew_test!.getSectionRenderInfo('object', b), box)
  expect(info.nodeClipCount).toBe(1) // the solid is clipped
  expect(info.widget).toBe(true)
  expect(info.widgetClipCount).toBe(0) // the widget overlay is not

  // A plain capture for the human eyeball (widget frame + arrow vs the cut
  // solid) — NOT a golden comparison, so it never gates the suite.
  await page.screenshot({ path: 'test-results/section-active.png' })
})

// ---------------------------------------------------------------------------
// Clip SIDE — the section must REMOVE the near/outer (+normal) side and KEEP
// the interior. Placed on a TOP face (outward normal +Z), material ABOVE is
// cut away and the interior BELOW is revealed. (The "flood waters backwards"
// regression: it used to hide below and keep above.)
// ---------------------------------------------------------------------------
test('a section on a top face removes the material ABOVE and keeps the interior below', async ({
  page,
}) => {
  const ctx = await setup(page)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))

  await activateSectionTool(page)
  await clickWorld(page, ctx, 1, 1, 2) // place on the top face (outward normal +Z)
  await page.waitForFunction(() => window.__hew_test!.getSectionState()?.active === true)

  // three.js keeps `normal·p + constant >= 0`, discards `< 0`. Assert the
  // clip plane cuts away points ABOVE the box (the +Z / near side) and keeps
  // points BELOW inside it (the interior).
  const dist = (info: { clipPlane: { normal: number[]; constant: number } | null }, p: number[]) => {
    const c = info.clipPlane!
    return c.normal[0] * p[0] + c.normal[1] * p[1] + c.normal[2] * p[2] + c.constant
  }
  const placed = await page.evaluate((b) => window.__hew_test!.getSectionRenderInfo('object', b), box)
  expect(placed.clipPlane).not.toBeNull()
  expect(dist(placed, [1, 1, 3])).toBeLessThan(0) // above the cut → removed
  expect(dist(placed, [1, 1, 1])).toBeGreaterThan(0) // inside/below → kept

  // Sweep the cut DOWN into the solid (drag the widget toward the interior):
  // progressively more of the top is removed. Grab the widget centre and
  // move down along the normal.
  const grab = px(ctx, 1, 1, 2)
  const to = px(ctx, 1, 1, 1)
  await page.mouse.move(grab.x, grab.y)
  await page.mouse.down()
  await page.mouse.up() // arm
  await page.mouse.move(to.x, to.y)
  await page.mouse.move(to.x, to.y)
  await page.mouse.down()
  await page.mouse.up() // commit

  const swept = await page.evaluate((b) => window.__hew_test!.getSectionRenderInfo('object', b), box)
  const cutZ = await page.evaluate(() => window.__hew_test!.getSectionState()!.origin[2])
  expect(cutZ).toBeLessThan(1.9) // the cut moved down
  // Still removing the top: a point just above the new cut is gone, just
  // below is kept.
  expect(dist(swept, [1, 1, cutZ + 0.3])).toBeLessThan(0)
  expect(dist(swept, [1, 1, cutZ - 0.3])).toBeGreaterThan(0)

  // Capture for the human eyeball — the top removed, the cross-section
  // exposed. Not a golden; never gates the suite.
  await page.screenshot({ path: 'test-results/section-clip-side.png' })
})

// ---------------------------------------------------------------------------
// Placement: a snap near a raised face's edge keeps its real height (Z).
// ---------------------------------------------------------------------------
test('placing near a raised face corner keeps the section at that height, not Z=0', async ({
  page,
}) => {
  const ctx = await setup(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))

  await activateSectionTool(page)
  // The top-face corner (2,2,2): inference offers an edge/vertex snap here,
  // which used to collapse the placement to the floor.
  await clickWorld(page, ctx, 2, 2, 2)
  await page.waitForFunction(() => window.__hew_test!.getSectionState() !== null)

  const st = await page.evaluate(() => window.__hew_test!.getSectionState())
  expect(st).not.toBeNull()
  expect(st!.origin[2]).toBeGreaterThan(1.5) // at the raised height, not Z=0
})

// ---------------------------------------------------------------------------
// Lifecycle: toggle off (widget kept, clip cleared), toggle on, Delete.
// ---------------------------------------------------------------------------
test('toggle active off/on keeps the widget and swaps the clip; Delete removes both', async ({
  page,
}) => {
  const ctx = await setup(page)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))

  await activateSectionTool(page)
  await clickWorld(page, ctx, 1, 1, 2)
  await page.waitForFunction(() => window.__hew_test!.getSectionState()?.active === true)
  expect(await page.evaluate((b) => window.__hew_test!.getSectionRenderInfo('object', b).nodeClipCount, box)).toBe(1)

  // Toggle OFF via the command — widget stays, clip clears.
  await page.evaluate(() => window.__hew_test!.toggleSectionActive())
  expect(await page.evaluate(() => window.__hew_test!.getSectionState()?.active)).toBe(false)
  const off = await page.evaluate((b) => window.__hew_test!.getSectionRenderInfo('object', b), box)
  expect(off.widget).toBe(true)
  expect(off.nodeClipCount).toBe(0)

  // Toggle back ON — clip returns.
  await page.evaluate(() => window.__hew_test!.toggleSectionActive())
  expect(await page.evaluate((b) => window.__hew_test!.getSectionRenderInfo('object', b).nodeClipCount, box)).toBe(1)

  // Delete (tool still active) → widget + clip both gone, model whole.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Delete')
  await page.waitForFunction(() => window.__hew_test!.getSectionState() === null)
  const gone = await page.evaluate((b) => window.__hew_test!.getSectionRenderInfo('object', b), box)
  expect(gone.widget).toBe(false)
  expect(gone.nodeClipCount).toBe(0)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
})

// ---------------------------------------------------------------------------
// Sweep: drag the widget along its normal to move the cut through the model.
// ---------------------------------------------------------------------------
test('dragging the widget sweeps the section along its normal', async ({ page }) => {
  const ctx = await setup(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))

  await activateSectionTool(page)
  await clickWorld(page, ctx, 1, 1, 2) // place on the top face (z=2)
  await page.waitForFunction(() => window.__hew_test!.getSectionState() !== null)
  // Tool stays active after placing — grab the widget directly.

  // Arm on the widget centre, move down along the normal, click to set.
  const grab = px(ctx, 1, 1, 2)
  const sweepTo = px(ctx, 1, 1, 0.5)
  await page.mouse.move(grab.x, grab.y)
  await page.mouse.down()
  await page.mouse.up() // arms the offset (click-move-click; no pointer-up in the Tool API)
  await page.mouse.move(sweepTo.x, sweepTo.y)
  await page.mouse.move(sweepTo.x, sweepTo.y) // second move settles the preview delta
  await page.mouse.down()
  await page.mouse.up() // commit the swept offset

  const st = await page.evaluate(() => window.__hew_test!.getSectionState())
  expect(st).not.toBeNull()
  // The cut moved down the normal from z=2 (a real sweep, not the placement).
  expect(st!.origin[2]).toBeLessThan(1.7)
})
