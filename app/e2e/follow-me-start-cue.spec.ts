import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Follow Me's START AFFORDANCE, cross-checked against the REAL kernel.
 *
 * The affordance predicts, while the cursor is still hovering, whether the
 * profile under it can start a sweep on the picked path. The only failure mode
 * that actually matters is a prediction that DISAGREES with the kernel — a cue
 * saying "legal here" where the sweep then refuses is worse than no cue. So
 * both tests below assert the prediction AND then let the kernel answer the
 * same question for real, in the same scene, with real pointer input:
 *
 *   1. Predicted refusal ⟹ actual refusal. A profile circle left lying flat
 *      on the ground beside a ground circle path: the tool warns during the
 *      hover, and the click that follows really does come back
 *      `[ProfileNotPerpendicular]`.
 *   2. Predicted acceptance ⟹ actual acceptance. The same profile stood up
 *      and moved onto the path's rim (the lathe build from
 *      `follow-me.spec.ts`, whose camera and steps this reuses): the tool says
 *      the profile starts cleanly, and the click really does sweep a solid.
 *
 * The unit specs in `src/tools/followMeStart.test.ts` cover the rule itself;
 * these are the ones that would catch it drifting away from the kernel.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/** Close framing: the whole scene lives within ~0.5 m of the origin (the
 *  lathe test's camera, so the same pixels mean the same world points). */
const CAMERA: CameraParams = {
  position: { x: 1.1, y: 0.8, z: 0.9 },
  target: { x: 0.15, y: 0, z: 0 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.01,
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

async function setup(page: Page, camera: CameraParams = CAMERA): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await page.evaluate(
    (cam) =>
      window.__hew_test!.setCamera({
        position: [cam.position.x, cam.position.y, cam.position.z],
        target: [cam.target.x, cam.target.y, cam.target.z],
        up: [cam.up.x, cam.up.y, cam.up.z],
        fovDeg: cam.fovDeg,
      }),
    camera,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(camera, rect.width / rect.height), rect }
}

async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

async function clickWorldSettled(
  page: Page,
  ctx: Ctx,
  x: number,
  y: number,
  z: number,
): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(300)
  await page.mouse.down()
  await page.mouse.up()
}

/** Hover a world point and let the tool's hover pass run. */
async function hoverWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 4, p.y - 4)
  await page.waitForTimeout(60)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(200)
}

/**
 * "Click" a hovered profile to commit — a plain press-release at the SAME
 * pixel, exactly like `clickWorld`.
 *
 * A Follow Me profile press now always goes through `_armOrCommit`
 * (E-app-3, the drag-to-partial-sweep gesture), which ARMS the `dragging`
 * stage whenever a seam walk can be built — true for virtually every real
 * path + profile. Unlike this codebase's other tools (all click-move-click:
 * a press arms, a SECOND press commits), Follow Me implements the optional
 * `Tool.onPointerUp` hook (`types.ts`) specifically so a plain click can
 * still commit on ONE press-release: the release, landing within
 * `MIN_PARTIAL_SWEEP_LEN` of the arming position (no move in between), reads
 * as "no real drag happened" and commits the FULL sweep. Named separately
 * from `clickWorld` only to document that intent at each call site.
 */
async function armAndCommit(page: Page): Promise<void> {
  await page.mouse.down()
  await page.mouse.up()
}

/** The two circles both tests start from: a 10 cm path circle about the origin
 *  (its centre pinned to the red axis, so a facet vertex lands exactly on it)
 *  and a 2 cm profile circle drawn flat beside it, centred on the same axis. */
async function drawTwoCircles(page: Page, ctx: Ctx): Promise<void> {
  await page.keyboard.press('c')
  await clickWorldSettled(page, ctx, 0, 0, 0)
  await page.mouse.move(px(ctx, 0.06, 0, 0).x, px(ctx, 0.06, 0, 0).y)
  await page.waitForTimeout(200)
  await page.keyboard.type('0.1')
  await page.keyboard.press('Enter')

  await page.keyboard.press('c')
  await clickWorldSettled(page, ctx, 0.35, 0, 0)
  await page.mouse.move(px(ctx, 0.4, 0, 0).x, px(ctx, 0.4, 0, 0).y)
  await page.waitForTimeout(200)
  await page.keyboard.type('0.02')
  await page.keyboard.press('Enter')
}

/** Select the path circle and switch to Follow Me, gated on the tool being
 *  live before any canvas click (a click racing the switch drives the old
 *  tool). */
async function pickPathAndActivate(page: Page, ctx: Ctx): Promise<void> {
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0, 0.1, 0) // the path circle's rim, a quarter turn away
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
}

test('Follow Me start cue: a warned-about placement is one the kernel really refuses', async ({
  page,
}) => {
  const ctx = await setup(page)
  await drawTwoCircles(page, ctx)
  await pickPathAndActivate(page, ctx)

  // Hover the profile circle while it is still lying FLAT on the ground — the
  // placement that used to teach nothing until the click came back refused.
  await hoverWorld(page, ctx, 0.35, 0, 0)
  await expect(page.getByText('Move it onto a marked quadrant')).toBeVisible()
  // Nothing has been committed, so nothing has been refused yet either.
  await expect(page.getByText('[ProfileNotPerpendicular]')).toHaveCount(0)

  // Now let the kernel answer the same question. The warning is only worth
  // anything if the kernel agrees, so this click MUST refuse.
  await page.mouse.down()
  await page.mouse.up()
  await expect(page.getByText('[ProfileNotPerpendicular]')).toBeVisible()
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)
})

test('Follow Me start cue: a confirmed placement is one the kernel really sweeps', async ({
  page,
}) => {
  const ctx = await setup(page)
  await drawTwoCircles(page, ctx)

  // Stand the profile up: Rotate, X lock, pivot ON the red axis (the axis snap
  // pins the standing plane's y to exactly 0), reference off-axis, typed 90°.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0.37, 0, 0)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('q')
  await page.keyboard.press('ArrowRight')
  await clickWorldSettled(page, ctx, 0.5, 0, 0)
  await clickWorld(page, ctx, 0.5, 0.1, 0)
  await page.keyboard.type('90')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 2)

  // Place it on the rim: Move, X lock, typed 0.25 toward the axis.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0.37, 0, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.keyboard.press('m')
  await clickWorldSettled(page, ctx, 0.37, 0, 0)
  await page.keyboard.press('ArrowRight')
  await page.mouse.move(px(ctx, 0.2, 0, 0).x, px(ctx, 0.2, 0, 0).y)
  await page.keyboard.type('0.25')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().some((s) => {
      const lines = h.getSketchLines(s)
      return lines.length > 0 && lines.every((v, i) => i % 3 !== 0 || v < 0.2)
    })
  })

  await pickPathAndActivate(page, ctx)

  // The profile now stands on a plane containing the circle's axis — the
  // radial family the kernel accepts. The tool says so BEFORE the click.
  await hoverWorld(page, ctx, 0.1, 0, 0.005)
  await expect(page.getByText('starts cleanly on the path')).toBeVisible()

  // And the kernel agrees: the very next click sweeps a closed solid.
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  for (const code of [
    '[ProfileNotPerpendicular]',
    '[PathDetachedFromProfile]',
    '[PathBranches]',
    '[PathDisconnected]',
  ]) {
    await expect(page.getByText(code)).toHaveCount(0)
  }
  expect(
    await page.evaluate(() => {
      const h = window.__hew_test!
      return h.isObjectSolid(h.getObjectIds()[0])
    }),
  ).toBe(true)
})

/**
 * design §2b — the corner seam. A closed path's CORNER is no longer an
 * automatic refusal: it is legal exactly when the hovered profile sits
 * entirely BEYOND the corner along the other (non-perpendicular) flank's own
 * direction, and refused (`PathTooTight`) when the profile decisively hangs
 * back over that flank instead (`followMeStart.ts`'s `cornerFold`).
 *
 * Path: a 2x1 rectangle on the ground, corners at the origin — the exact
 * shape `followMeStart.test.ts`'s `rectPath()` pins the fold math against.
 * At the (0,0,0) corner the incoming flank is (0,1,0)->(0,0,0), so "beyond
 * the corner" is the half-space y <= 0.
 */
const CORNER_CAMERA: CameraParams = {
  position: { x: 1.6, y: -2.3, z: 1.9 },
  target: { x: 0.6, y: 0.05, z: 0 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.01,
  far: 1000,
}

test('Follow Me start cue: a corner seam beyond the corner sweeps clean; straddling it is refused PathTooTight', async ({
  page,
}) => {
  const ctx = await setup(page, CORNER_CAMERA)

  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawRectangle([0, 0, 0], [2, 1, 0])
  })

  // Legal corner-seam profile: folded onto the x = 0 plane (through the
  // corner), sitting entirely at y <= 0 — fully beyond the corner along the
  // incoming flank's own direction, the mitred picture-frame band the
  // kernel's own spec accepts.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const r = h.drawRectangle([-0.1, -0.5, 0], [0.1, -0.1, 0])
    h.rotateSketch(r.sketch, 90, [0, 1, 0], [0, 0, 0])
  })

  // Overhang profile: same plane, but straddling y = 0 — part of it hangs
  // back over the corner's wrong flank, the fold the kernel's advance check
  // refuses as PathTooTight.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const r = h.drawRectangle([-0.15, -0.2, 0], [0.15, 0.2, 0])
    h.rotateSketch(r.sketch, 90, [0, 1, 0], [0, 0, 0])
  })

  // Pick the path with a real click on its bottom edge, then activate Follow Me.
  await clickWorld(page, ctx, 1, 0, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()

  // Hover the LEGAL corner-seam profile — the cue reads clean, no refusal
  // hint, before any click.
  await hoverWorld(page, ctx, 0, -0.3, 0)
  await expect(page.getByText('starts cleanly on the path')).toBeVisible()
  await expect(page.getByText('[PathTooTight]')).toHaveCount(0)

  // The kernel agrees: the click sweeps a watertight solid.
  await armAndCommit(page)
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  expect(
    await page.evaluate(() => {
      const h = window.__hew_test!
      return h.isObjectSolid(h.getObjectIds()[0])
    }),
  ).toBe(true)
  await expect(page.getByText('[PathTooTight]')).toHaveCount(0)

  // Re-pick the SAME path (the commit above returned the tool to pick-path;
  // the path sketch itself is left standing) for the overhang placement.
  await clickWorld(page, ctx, 1, 0, 0)
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()

  // Hover the OVERHANG profile — the cue names the corner and the fold
  // BEFORE any click.
  await hoverWorld(page, ctx, 0, 0, 0)
  await expect(page.getByText(/hangs back over the corner/)).toBeVisible()
  await expect(page.getByText('[PathTooTight]')).toHaveCount(0) // nothing clicked yet

  // The kernel agrees: the click really is refused, and nothing new is built.
  await armAndCommit(page)
  await expect(page.getByText('[PathTooTight]')).toBeVisible()
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
})

/**
 * design §2a — a detached-but-perpendicular open-path end is CARRIED
 * (rigidly translated) to the profile rather than refused for not touching
 * it. Path: a single ground segment (0,0,0)->(1,0,0); profile: perpendicular
 * to it, standing on the plane x = 5 — a real 4 m gap from the near end. The
 * near end (distance 4) is closer than the far end (distance 5), so it is
 * the one carried; the whole 1 m path is translated +4 in x, landing the
 * swept solid around x ∈ [4, 5] — nowhere near where the path itself was
 * drawn (x ∈ [0, 1]).
 */
const DETACHED_CAMERA: CameraParams = {
  position: { x: 2.5, y: -6, z: 3.5 },
  target: { x: 2.5, y: 0, z: 0.2 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.01,
  far: 1000,
}

test('Follow Me start cue: a perpendicular-but-detached open end is CARRIED to the profile, not refused', async ({
  page,
}) => {
  const ctx = await setup(page, DETACHED_CAMERA)

  await page.evaluate(() => window.__hew_test!.drawLineChain([[0, 0, 0], [1, 0, 0]]))

  await page.evaluate(() => {
    const h = window.__hew_test!
    const r = h.drawRectangle([4.9, -0.1, 0], [5.1, 0.1, 0])
    h.rotateSketch(r.sketch, 90, [0, 1, 0], [5, 0, 0])
  })

  // Pick the path with a real click, then activate Follow Me.
  await clickWorld(page, ctx, 0.5, 0, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()

  // Hover the detached profile — the cue reads "ok, carried", NOT refused,
  // and says the sweep follows the path's shape from the profile.
  await hoverWorld(page, ctx, 5, 0, 0)
  await expect(page.getByText('The sweep starts at the profile and follows the path’s shape')).toBeVisible()
  for (const code of ['[PathDetachedFromProfile]', '[ProfileNotPerpendicular]']) {
    await expect(page.getByText(code)).toHaveCount(0)
  }

  // The kernel agrees: the click commits a watertight solid.
  await armAndCommit(page)
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  for (const code of ['[PathDetachedFromProfile]', '[ProfileNotPerpendicular]']) {
    await expect(page.getByText(code)).toHaveCount(0)
  }

  const info = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getObjectIds()[0]
    return { solid: h.isObjectSolid(id), bounds: h.getObjectBounds(id) }
  })
  expect(info.solid).toBe(true)
  // The path was drawn at x ∈ [0, 1] — a carried sweep lands well beyond
  // that, close to the profile at x = 5, not where the path itself sits.
  const [minX, , , maxX] = info.bounds
  expect(minX).toBeGreaterThan(3)
  expect(maxX).toBeGreaterThan(4.5)
})
