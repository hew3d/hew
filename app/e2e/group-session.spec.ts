import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Group editing sessions — end to end with REAL pointer input
 * (docs/design/group-session.md, the app-layer phase). A group session is
 * the group analog of the existing component explode session: opening one
 * applies the "ungroup posture" (direct members surface to the world top
 * level, the group hides) so the ordinary, unmodified tool set — the six
 * replacing ops included — edits the members directly; closing folds
 * everything back.
 *
 * Two journeys:
 *
 * 1. The reported stress-test bug: `push_pull_through` (and every other
 *    replacing op) used to refuse `GroupedOperand` against a grouped
 *    member, because the op consumes its operand and mints a fresh
 *    TOP-LEVEL result — which a group member, nested under its group, is
 *    not. A group session's ungroup posture makes members genuinely
 *    top-level for the session's duration, so this now succeeds: double-
 *    click into a group, draw a circle on a member face, Push/Pull it
 *    clean through the solid (a through-cut, the same real click-drag-
 *    click PushPullTool gesture explode-session.spec.ts's own mid-session
 *    edit uses) to cut a hole. Escape closes — the result folds back into
 *    the group as its (replaced) member.
 * 2. Entry convergence (the reported Outliner-vs-viewport divergence): the
 *    Outliner's double-click on a node nested two levels deep (group →
 *    object) must open the enclosing group's session AND push an object
 *    context, producing the SAME breadcrumb/"editing"-chip state a
 *    viewport double-click into the group followed by one into the object
 *    would reach — not the old non-contiguous "Model → Object 1" the
 *    Outliner used to produce by pushing the bare clicked node with no
 *    ancestor walk.
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

function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

async function ready(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

async function aim(page: Page, camera: CameraParams): Promise<Ctx> {
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

/** Move first (settles the hover pick before the down), then click — the
 *  same flake-free pattern explode-session.spec.ts/tools.spec.ts use. */
async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 4, p.y - 4)
  await page.waitForTimeout(40)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(120)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

/** A real click-drag-click through a two-click tool gesture (Push/Pull). */
async function dragWorld(
  page: Page,
  ctx: Ctx,
  anchor: [number, number, number],
  target: [number, number, number],
): Promise<void> {
  const a = px(ctx, anchor[0], anchor[1], anchor[2])
  const t = px(ctx, target[0], target[1], target[2])
  await page.mouse.move(a.x, a.y)
  await page.mouse.down() // first click: arm the drag
  await page.mouse.up()
  await page.mouse.move(t.x, t.y, { steps: 10 })
  await page.mouse.down() // second click: commit
  await page.mouse.up()
  await page.waitForTimeout(150)
}

/** A real double-click at a world point — opens a session (group or
 *  component) on whatever it resolves to at the current context depth. */
async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

// Same 3/4 framing of a 2×1×1 m box at the origin that explode-session.spec.ts
// uses for its own box — proven pixel math for exactly this box shape/camera
// pairing, reused here rather than re-derived.
const CAMERA: CameraParams = {
  position: { x: 5, y: -5, z: 5 },
  target: { x: 1.2, y: 0.5, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.1,
  far: 1000,
}

test('group session: double-click in, draw a circle on a member face, Push/Pull it clean through (GroupedOperand fix), close folds the result back in', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const group = h.groupNodes([{ kind: 'object', id: box }])
    return { box, group }
  })
  const { group } = setup

  const membersBefore = await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), group)
  expect(membersBefore).toEqual([{ kind: 'object', id: setup.box }])

  // ---- 1. Double-click into the group — a plain double-click on a
  // top-level group opens a group session by default now (the group analog
  // of the existing component explode session).
  const beforeShot = await page.locator('canvas').first().screenshot()
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(200)

  // The member is now a genuinely top-level world object — the ungroup
  // posture, no bake/pose involved (unlike a component session).
  expect(await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), setup.box))
    .toEqual([0, 0, 0, 2, 1, 1])
  const hashAfterOpen = await page.evaluate(() => window.__hew_test!.getStateHash())

  // ---- 2. Draw a circle on the TOP member face — a real two-click Circle
  // tool gesture (center, then a rim point), the same pattern
  // explode-session.spec.ts's own mid-session Rectangle draw uses.
  await page.keyboard.press('c')
  await clickWorld(page, ctx, 1, 0.5, 1) // center
  await clickWorld(page, ctx, 1.3, 0.5, 1) // rim point (radius 0.3)
  const hashAfterCircle = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfterCircle).not.toBe(hashAfterOpen)

  // ---- 3. Push/Pull the circle's imprinted sub-face DOWN, clean through
  // the 1 m-thick box (a real drag past the opposite wall — the same
  // click-drag-click PushPullTool gesture, this time deep enough to trip
  // the through-cut path: this is exactly the operation the stress test
  // found refusing `GroupedOperand` against a grouped member before group
  // sessions existed — a replacing op that consumes its operand and mints
  // a fresh TOP-LEVEL result, which only works because the member is
  // genuinely top-level for the session's duration.
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [1, 0.5, 1], [1, 0.5, -0.5])
  const hashAfterThroughCut = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfterThroughCut).not.toBe(hashAfterCircle)
  // The through-cut consumed the original object and minted a fresh one in
  // its place — still exactly one live world object (a hole punched clean
  // through does not split the box into separate pieces).
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)

  // ---- 4. Escape closes the session — the through-cut's result (a NEW
  // object id; the source was consumed) folds back in as the group's
  // (replaced) member, not left stranded at the top level.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  const membersAfter = await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), group)
  expect(membersAfter).toHaveLength(1)
  expect(membersAfter[0].kind).toBe('object')
  expect(membersAfter[0].id).not.toBe(setup.box) // the through-cut replaced it
  const afterCloseShot = await page.locator('canvas').first().screenshot()
  expect(afterCloseShot.equals(beforeShot)).toBe(false)
})

// Adversarial-review finding 1, repro (a): `enterNode` used to walk
// `node_parent` to build the target's container chain, but an OPEN session
// erases exactly the parent links of its own direct members (the ungroup
// posture) — so a target already reachable through an open session produced
// a chain with no open frame in it at all, and the entry-convergence diff
// read that as "outside every open frame", closing the whole stack. An
// Outliner double-click on a session member WHILE its session is open must
// keep the session open and just push the member as an object context on
// top of it — not close it and land the object at the top level.
test('entry convergence: an Outliner double-click on a session MEMBER while its session is open pushes an object context instead of closing the session', async ({
  page,
}) => {
  await ready(page)

  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const group = h.groupNodes([{ kind: 'object', id: box }])
    return { box, group }
  })

  const ctx = await aim(page, CAMERA)
  // Open the group session via a real viewport double-click first — the
  // member becomes a genuinely top-level world object for the session's
  // duration (the ungroup posture), which is exactly the state that used to
  // fool `enterNode`'s ancestor walk.
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(200)
  await expect(page.getByText('editing', { exact: true })).toHaveCount(1)
  expect(await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), setup.group)).toEqual([])

  // Now double-click the member's OWN row in the Outliner — the session
  // header nests it as "Object 1" (its positional label as the group's
  // only, unnamed member).
  const memberRow = page.getByText('Object 1').first()
  await expect(memberRow).toBeVisible()
  await memberRow.dblclick()
  await page.waitForTimeout(200)

  // The session stayed open (still exactly one "editing" chip for the
  // group frame) AND an object context pushed on top of it (a second chip
  // for the object) — the bug this fixes closed the session outright,
  // dropping back to zero chips and a bare top-level object context.
  await expect(page.getByText('editing', { exact: true })).toHaveCount(2)
  await expect(page.getByRole('button', { name: 'Model' })).toBeVisible()
  // The kernel agrees: the group session is still open (its member reads
  // as hidden/empty from the outside, the same assertion the sibling
  // convergence test above uses).
  expect(await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), setup.group)).toEqual([])

  // Escape twice: pops the object context, then closes the group session —
  // unchanged layering, proving the stack is genuinely still intact (a
  // single Escape from the CLOSED-session bug's landing state would have
  // had nothing left to pop).
  await page.keyboard.press('Escape')
  await page.waitForTimeout(150)
  await expect(page.getByText('editing', { exact: true })).toHaveCount(1)
  await page.keyboard.press('Escape')
  await page.waitForTimeout(150)
  await expect(page.getByText('editing', { exact: true })).toHaveCount(0)
  expect(await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), setup.group))
    .toEqual([{ kind: 'object', id: setup.box }])
})

test('entry convergence: an Outliner double-click on an object nested inside a group opens the group session AND pushes the object context, matching the viewport path', async ({
  page,
}) => {
  await ready(page)

  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const group = h.groupNodes([{ kind: 'object', id: box }])
    return { box, group }
  })

  // Select the nested object directly (not the group) — the Outliner
  // auto-expands every ancestor of a selected node, revealing its row
  // without a manual chevron click.
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), setup.box)

  // Positional label: the object is this group's only, unnamed member, so
  // it reads "Object 1" — same fallback `resolveLabel` uses everywhere else.
  const objectRow = page.getByText('Object 1').first()
  await expect(objectRow).toBeVisible()
  await objectRow.dblclick()
  await page.waitForTimeout(200)

  // Both the group frame AND the pushed object context are "editing" —
  // design: every entry on the combined path gets the chip, not just the
  // deepest (the bug this fixes: the Outliner used to push the bare
  // clicked object with no ancestor walk, producing a non-contiguous
  // "Model → Object 1" breadcrumb with no chip on the group at all).
  // Exact match: the chip text is exactly "editing"; a substring match
  // would also catch the viewport's "Editing <name>" status overlay.
  await expect(page.getByText('editing', { exact: true })).toHaveCount(2)
  // The root "Model" crumb is still a clickable button — the combined path
  // is non-empty (Group → Object), not collapsed back to the top level.
  await expect(page.getByRole('button', { name: 'Model' })).toBeVisible()

  // The kernel agrees: a group session is open on exactly this group (its
  // member is a genuinely top-level world object right now, the ungroup
  // posture — `getGroupMembers` on a hidden/session-open group answers
  // `undefined`/empty from the kernel's own hidden filter, which the
  // harness surfaces as an empty array here).
  expect(await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), setup.group)).toEqual([])
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)

  // Escape twice: pops the object context first, then closes the group
  // session — the design's layering (one gesture per level).
  await page.keyboard.press('Escape')
  await page.waitForTimeout(150)
  // Exact match throughout — see above.
  await expect(page.getByText('editing', { exact: true })).toHaveCount(1) // only the group frame remains
  await page.keyboard.press('Escape')
  await page.waitForTimeout(150)
  await expect(page.getByText('editing', { exact: true })).toHaveCount(0)
  expect(await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), setup.group))
    .toEqual([{ kind: 'object', id: setup.box }])
})

// Tape Measure scoped rescale (docs/design/group-session.md, "Tape Measure
// scoped rescale"): with a group session open, typing a length after
// measuring two real member points arms a confirmation NAMING the group
// (rather than the old blanket refusal/toast) and, on Confirm, calls
// `rescale_session` — resizing only the group's contents about the
// measurement's first point. Unlike the whole-model path
// (tools.spec.ts's own rescale coverage), this must NOT touch geometry
// outside the group, and must NOT companion-scale the camera/grid.
test('group session: Tape Measure rescale is scoped to the open group — member resizes about the measured point, outside geometry and the camera are untouched', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1) // x:[0,2] y:[0,1] z:[0,1]
    const group = h.groupNodes([{ kind: 'object', id: box }])
    // An unrelated top-level box, well clear of the group — the scoped
    // rescale's "outside geometry stays put" half of the contract.
    const outside = h.drawBox([10, 10, 0], [11, 11, 0], 1)
    return { box, group, outside }
  })

  const outsideBoundsBefore = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    setup.outside,
  )
  const cameraBefore = await page.evaluate(() => window.__hew_test!.getCamera())

  // ---- Double-click into the group — opens the group session (the
  // ungroup posture: the member is genuinely top-level for the duration).
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(200)

  // ---- Tape Measure: two real clicks on the member's own top-front edge
  // corners — (0,0,1)→(2,0,1), a real 2 m distance, both resting on real
  // geometry (the rescale arm requires both ends to) — then type a
  // DIFFERENT length to arm the confirmation. Mirrors tools.spec.ts's own
  // rescale journey, just aimed at a grouped member mid-session instead of
  // a bare top-level object.
  await page.keyboard.press('t')
  await page.locator('text=Click a point to measure from').first().waitFor({ timeout: 5000 })

  const cornerA = px(ctx, 0, 0, 1)
  const cornerB = px(ctx, 2, 0, 1)
  await page.mouse.move(cornerA.x, cornerA.y)
  await page.waitForTimeout(40)
  await page.mouse.down() // first point (the rescale anchor)
  await page.mouse.up()
  await page.mouse.move(cornerB.x, cornerB.y, { steps: 8 }) // hover the second point live
  await page.keyboard.type('4') // real 2 m vs. typed 4 m -> factor 2, arms the SCOPED confirm
  await page.keyboard.press('Enter')

  // The dialog names the group, not "the model" — the group is unnamed, so
  // it reads "Group 1" (the same `resolveLabel` fallback the breadcrumb and
  // Outliner already use).
  const dialog = page.getByRole('dialog', { name: 'Resize Group 1' })
  await dialog.waitFor({ timeout: 5000 })
  await expect(page.getByRole('dialog', { name: 'Resize the model' })).toHaveCount(0)
  await page.getByRole('button', { name: 'Resize' }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)

  // ---- 1. The member's bounds doubled about the measured anchor (0,0,1):
  // a' = anchor + factor*(a - anchor), factor = 2, anchor = (0,0,1).
  //   x: [0,2]   -> [0,4]
  //   y: [0,1]   -> [0,2]
  //   z: [0,1]   -> [-1,1]
  const memberBounds = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    setup.box,
  )
  const expected = [0, 0, -1, 4, 2, 1]
  for (let i = 0; i < 6; i++) expect(memberBounds[i]).toBeCloseTo(expected[i], 4)

  // ---- 2. The outside object is untouched — a scoped rescale only
  // touches the innermost session's contents.
  const outsideBoundsAfter = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    setup.outside,
  )
  expect(outsideBoundsAfter).toEqual(outsideBoundsBefore)

  // ---- 3. No camera/grid companion scaling — unlike the whole-model path,
  // the world outside the session didn't change size, so the view must not
  // move (`applyRescaleToView` must not have run). Per-component
  // `toBeCloseTo` rather than `toEqual`: OrbitControls re-normalizes its
  // internal spherical coordinates on every `controls.update()` (harmless
  // mid-session scene refreshes call it too), which can perturb the last
  // couple of float64 ULPs even with no actual re-pose — this asserts "no
  // camera move happened", not float-bit-identity.
  const cameraAfter = await page.evaluate(() => window.__hew_test!.getCamera())
  for (let i = 0; i < 3; i++) {
    expect(cameraAfter.position[i]).toBeCloseTo(cameraBefore.position[i], 6)
    expect(cameraAfter.target[i]).toBeCloseTo(cameraBefore.target[i], 6)
  }
  expect(cameraAfter.fovDeg).toBe(cameraBefore.fovDeg)

  // ---- 4. Escape closes the session — the member folds back into the
  // group carrying its new size (same object id: a rescale transforms in
  // place, it doesn't consume/replace like a boolean/push-pull-through).
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  const membersAfter = await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), setup.group)
  expect(membersAfter).toEqual([{ kind: 'object', id: setup.box }])
  const memberBoundsAfterClose = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    setup.box,
  )
  for (let i = 0; i < 6; i++) expect(memberBoundsAfterClose[i]).toBeCloseTo(expected[i], 4)
})

// Adversarial-review finding 6: inference/snapping stays deliberately
// unscoped while a session is open (ARCHITECTURE.md 2.4), so a Tape Measure
// gesture can freely measure geometry OUTSIDE the open session even though a
// scoped `rescale_session` only ever touches what's INSIDE it. Arming on
// such a measurement would translate the session's contents by a wrong
// anchor while the measured distance itself never changes size. This must
// decline the arm outright (toast, no dialog) rather than reach the
// confirmation the sibling scoped-rescale test above exercises.
test('group session: measuring geometry OUTSIDE the open session declines the scoped-rescale arm — no dialog, outside geometry never resizes', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const group = h.groupNodes([{ kind: 'object', id: box }])
    // A separate, unrelated box right next to the group (disjoint footprint,
    // but still inside the test camera's framing so it's actually clickable)
    // — the geometry this test measures, deliberately outside the open
    // session's scope.
    const outside = h.drawBox([3, 0, 0], [5, 1, 0], 1)
    return { box, group, outside }
  })

  const outsideBoundsBefore = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    setup.outside,
  )

  // ---- Open the group session.
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(200)
  await expect(page.getByText('editing', { exact: true })).toHaveCount(1)

  // ---- Tape Measure: two real clicks on the OUTSIDE box's own top-front
  // edge corners (a real 2 m distance) — snapping is unscoped, so this
  // measures cleanly even though the box is outside the open session.
  await page.keyboard.press('t')
  await page.locator('text=Click a point to measure from').first().waitFor({ timeout: 5000 })

  const cornerA = px(ctx, 3, 0, 1)
  const cornerB = px(ctx, 5, 0, 1)
  await page.mouse.move(cornerA.x, cornerA.y)
  await page.waitForTimeout(40)
  await page.mouse.down()
  await page.mouse.up()
  await page.mouse.move(cornerB.x, cornerB.y, { steps: 8 })
  await page.keyboard.type('4') // real 2 m vs. typed 4 m -> would arm factor 2 if it armed at all
  await page.keyboard.press('Enter')

  // No confirmation dialog of ANY kind reaches the screen — declined before
  // arming, not shown-then-cancelled.
  await page.waitForTimeout(200)
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByText(/outside the open group or component/i)).toBeVisible()

  // The outside geometry never moved — the bug this prevents would have
  // translated it (a scoped rescale anchored at a point ON it, but scoped
  // to a session that does NOT contain it, is a no-op on the anchor object
  // itself only by coincidence of the specific case; the general contract
  // this test pins is simpler and unconditional: no rescale ever applied).
  const outsideBoundsAfter = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    setup.outside,
  )
  expect(outsideBoundsAfter).toEqual(outsideBoundsBefore)

  // The member's own size is untouched too — nothing applied at all.
  const memberBounds = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), setup.box)
  expect(memberBounds).toEqual([0, 0, 0, 2, 1, 1])

  await page.keyboard.press('Escape') // Tape Measure is idle (declined, not armed) — this closes the group session
  await page.waitForTimeout(150)
  expect(await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), setup.group))
    .toEqual([{ kind: 'object', id: setup.box }])
})

