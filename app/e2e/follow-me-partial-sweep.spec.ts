import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Follow Me — E4, the drag-to-partial-sweep gesture, cross-checked against
 * the REAL kernel with REAL pointer input.
 *
 * After a path is picked, a plain click on a valid profile still commits the
 * FULL sweep; pressing and then MOVING the cursor along the path before
 * releasing previews a live "swept so far" station and a length readout in
 * the VCB (`MeasurementBox.tsx`, label "Swept length"), and the release
 * commits a PARTIAL sweep of the dragged length (`followMeDrag.ts`'s seam
 * walk + `FollowMeTool`'s `dragging` stage). Escape mid-drag backs up to the
 * profile-pick stage without dropping the picked path. Typing a length and
 * pressing Enter while hovering a profile commits a partial sweep directly,
 * without a drag.
 *
 * GESTURE MODEL (verified against the real app): Follow Me is one of the few
 * tools that implements the optional `Tool.onPointerUp` hook (`types.ts`) —
 * every OTHER tool here is click-move-click (a press arms, a SECOND press
 * commits), but this gesture genuinely needs a real release: a plain click
 * and a press-drag-release must commit DIFFERENT things from the same first
 * press. Concretely: the profile PRESS arms the gesture (`_armOrCommit`
 * enters the `dragging` stage whenever a seam walk can be built, true for
 * virtually any real path + profile) — the button stays DOWN; real pointer
 * MOVEs while still held update the live preview; the RELEASE — at the
 * arming position for a "plain click" full sweep, or after a real move for a
 * partial one — commits. So a plain click is exactly `mouse.down()` +
 * `mouse.up()` with no move in between (unchanged from before E4); a drag is
 * `mouse.down()`, several `mouse.move()`s with the button still down, then
 * `mouse.up()` — never a second, separate `down()`/`up()` pair.
 *
 * Path: a straight 3 m ground segment (0,0,0)->(3,0,0). Profile: a small
 * square attached exactly at the path's near end (plane x = 0, on the path),
 * so every hover reads a clean "starts cleanly on the path" — the start
 * legality itself is covered by `follow-me-start-cue.spec.ts`; this suite is
 * about the DRAG, not the legality cue.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 1.5, y: -3.6, z: 2.4 },
  target: { x: 1.5, y: 0, z: 0.1 },
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

async function setup(page: Page): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await page.evaluate(() => window.__hew_test!.setLengthUnit('m')) // deterministic VCB/typed-length format
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

/** The straight 3 m path + the attached profile at its near end (x = 0),
 *  built through the harness (deterministic scaffolding, NOT what is under
 *  test — see follow-me-start-cue.spec.ts for the legality cue itself). */
async function buildScene(page: Page): Promise<void> {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawLineChain([[0, 0, 0], [3, 0, 0]])
    const r = h.drawRectangle([-0.1, -0.1, 0], [0.1, 0.1, 0])
    h.rotateSketch(r.sketch, 90, [0, 1, 0], [0, 0, 0])
  })
}

/** Pick the path with a real click, then activate Follow Me — gated on the
 *  tool being live before any further canvas interaction. */
async function pickPathAndActivate(page: Page, ctx: Ctx, alongX: number): Promise<void> {
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, alongX, 0, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
}

async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

/** A fresh move (offset first so it isn't coalesced away by the browser),
 *  settled for the snap/hover pass to run. */
async function moveTo(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 3, p.y - 3)
  await page.waitForTimeout(40)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(120)
}

/** The MeasurementBox's own text ("" when not showing) — read by DOM
 *  structure (label span "Swept length" + its sibling value span), not by
 *  CSS, since the box has no test id. */
async function vcbText(page: Page): Promise<string> {
  return page.evaluate(() => {
    const label = Array.from(document.querySelectorAll('span')).find(
      (el) => el.textContent === 'Swept length',
    )
    return label?.parentElement?.textContent ?? ''
  })
}

test('Follow Me partial sweep: a real drag previews a live length and commits a shorter solid than a full sweep', async ({
  page,
}) => {
  const pageErrors: string[] = []
  page.on('pageerror', (err) => pageErrors.push(err.message))

  const ctx = await setup(page)
  await buildScene(page)
  await pickPathAndActivate(page, ctx, 1.5)

  // Hover the attached profile — a clean start, not a corner/detached case.
  await moveTo(page, ctx, 0, 0, 0)
  await expect(page.getByText('starts cleanly on the path')).toBeVisible()
  expect(await vcbText(page)).toBe('') // no VCB before any press

  // ARM the drag: press (and hold — no up yet) at the profile, entering the
  // `dragging` stage.
  await page.mouse.down()
  await expect(page.getByText('Drag along the path for a partial sweep', { exact: false })).toBeVisible()

  // Real intermediate moves along the path's own screen-space direction —
  // several steps, not one jump, so intermediate onPointerMove events land
  // and the live preview + VCB genuinely update as the cursor travels.
  await moveTo(page, ctx, 0.4, 0, 0)
  const readingA = await vcbText(page)
  expect(readingA).toContain('Swept length')
  expect(readingA).toMatch(/\d/)

  await moveTo(page, ctx, 0.8, 0, 0)
  const readingB = await vcbText(page)
  expect(readingB).not.toBe(readingA) // the readout tracked the cursor

  await moveTo(page, ctx, 1.2, 0, 0)
  const readingC = await vcbText(page)
  expect(readingC).not.toBe(readingB)

  // Commit: the RELEASE, at the dragged position (button was held since arm).
  await page.mouse.up()
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  const partialId = await page.evaluate(() => window.__hew_test!.getObjectIds()[0])
  const partial = await page.evaluate((id) => {
    const h = window.__hew_test!
    return { solid: h.isObjectSolid(id), bounds: h.getObjectBounds(id) }
  }, partialId)
  expect(partial.solid).toBe(true)
  const partialLength = partial.bounds[3] - partial.bounds[0] // max X − min X
  // Dragged to ~1.2 m of a 3 m path — comfortably short of the full length,
  // and in the right ballpark for where the cursor was released (a few cm
  // of pixel-projection slack either way).
  expect(partialLength).toBeGreaterThan(0.8)
  expect(partialLength).toBeLessThan(1.6)

  // Control: re-pick the SAME path (the commit returned the tool to
  // pick-path; the path sketch is left standing) and commit a FULL sweep —
  // a plain click, arm then release at the same spot, no drag in between.
  // The FIRST profile's region was consumed by its own commit (exactly like
  // an extrusion's — `FollowMeTool.ts`'s module doc), so this needs a FRESH
  // profile sketch at the same attached position.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const r = h.drawRectangle([-0.1, -0.1, 0], [0.1, 0.1, 0])
    h.rotateSketch(r.sketch, 90, [0, 1, 0], [0, 0, 0])
  })
  await clickWorld(page, ctx, 2.5, 0, 0)
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
  await moveTo(page, ctx, 0, 0, 0)
  await page.mouse.down()
  await page.mouse.up() // one press-release, no move in between: a plain click
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2)

  const full = await page.evaluate((skipId) => {
    const h = window.__hew_test!
    const newId = h.getObjectIds().find((i) => i !== skipId)!
    return { bounds: h.getObjectBounds(newId) }
  }, partialId)
  const fullLength = full.bounds[3] - full.bounds[0]
  expect(fullLength).toBeGreaterThan(2.8) // the whole 3 m path
  expect(partialLength).toBeLessThan(fullLength)

  expect(pageErrors, `unexpected page errors during the drag:\n${pageErrors.join('\n')}`).toEqual([])
})

test('Follow Me partial sweep: Escape mid-drag backs up to profile-pick without committing; the path stays pickable', async ({
  page,
}) => {
  const ctx = await setup(page)
  await buildScene(page)
  await pickPathAndActivate(page, ctx, 1.5)

  await moveTo(page, ctx, 0, 0, 0)
  await expect(page.getByText('starts cleanly on the path')).toBeVisible()

  // Arm (hold — no up yet), drag partway…
  await page.mouse.down()
  await moveTo(page, ctx, 0.6, 0, 0)
  await expect(page.getByText('Drag along the path for a partial sweep', { exact: false })).toBeVisible()

  // …then Escape, still mid-press. Nothing commits, and the tool backs up to
  // profile-pick — NOT path-pick: the path is still highlighted/selected,
  // ready to click again, exactly as the module doc promises ("cancels back
  // to the PROFILE stage… the path is still picked and highlighted"). The
  // status hint still shows the last hover's verdict badge copy (Escape
  // doesn't clear `profileVerdict`) — "starts cleanly", not the generic
  // pick-profile hint — since the cursor never left the (still valid)
  // profile. Release the button afterward (a real user abandoning the drag
  // would let go too) — the tool's `onPointerUp` sees `pick-profile`, not
  // `dragging`, so this is a harmless no-op, not a second commit.
  await page.keyboard.press('Escape')
  await page.mouse.up()
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)
  expect(await vcbText(page)).toBe('')
  await expect(page.getByText('starts cleanly on the path')).toBeVisible()

  // A subsequent plain click (press + release, no drag) on the SAME still-
  // picked path still works and sweeps the FULL path — the cancel did not
  // silently drop the pick.
  await moveTo(page, ctx, 0, 0, 0)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  const info = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getObjectIds()[0]
    return { solid: h.isObjectSolid(id), bounds: h.getObjectBounds(id) }
  })
  expect(info.solid).toBe(true)
  expect(info.bounds[3] - info.bounds[0]).toBeGreaterThan(2.8) // the whole 3 m path
})

test('Follow Me partial sweep: typing a length + Enter at the profile stage commits a partial sweep without a drag', async ({
  page,
}) => {
  const ctx = await setup(page)
  await buildScene(page)
  await pickPathAndActivate(page, ctx, 1.5)

  // Hover the profile — no press, no drag — then type a length and Enter.
  await moveTo(page, ctx, 0, 0, 0)
  await expect(page.getByText('starts cleanly on the path')).toBeVisible()

  await page.keyboard.type('1')
  await expect(page.getByText('Swept length')).toBeVisible()
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  const info = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getObjectIds()[0]
    return { solid: h.isObjectSolid(id), bounds: h.getObjectBounds(id) }
  })
  expect(info.solid).toBe(true)
  const length = info.bounds[3] - info.bounds[0]
  // A typed 1 m sweep of a 3 m path — short of the full length, and close
  // to the typed value (exact, since this commits the literal parsed
  // length rather than a ray-projected drag position).
  expect(length).toBeGreaterThan(0.9)
  expect(length).toBeLessThan(1.1)
})
