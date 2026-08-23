import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * The explode-session component-editing model — end to end with REAL
 * pointer input (feat(viewport): make the explode session a plain
 * double-click; docs/agents/ARCHITECTURE.md 2.11). A plain double-click on a
 * uniformly-posed component instance now bakes its definition's members
 * into world-owned geometry (same ObjectIds, a move not a copy) so the
 * entirely unmodified tool set edits them directly — no `_in_instance`
 * kernel calls, no pose mapping, and (per component-edit.spec.ts's own
 * DEFERRED FINDING) none of that spec's real-drag Push/Pull fragility,
 * since a session member IS a plain world object for the edit's duration.
 *
 * The main journey drives one 2×1×1 box, folded into a component with TWO
 * instances (so "did the edit reach every placement" has a real second
 * instance to check against) plus one entirely UNRELATED box elsewhere (so
 * "is the rest of the model actually dimmed" has something to dim):
 *
 *   1. Double-click instance A open — sibling instance B (same definition)
 *      and the unrelated box both prove out: B is hidden kernel-side
 *      (`instances_of` excludes it), the unrelated box is merely dimmed
 *      (still a live, visible, un-hidden world object — just faded).
 *   2. Push/Pull a member face outward (a REAL drag — a plain world-object
 *      push, the same mechanics tools.spec.ts's own Push/Pull drags use).
 *   3. Draw a rectangle on another member face and Push/Pull the new
 *      sub-face outward (the "draw, then fold in" case: brand-new geometry
 *      created mid-session becomes part of the definition at close).
 *   4. Escape closes the session — both instances render both edits.
 *   5. Undo walks the granular steps in reverse (close → extrude → push/pull
 *      → open), each one an ordinary history entry; undoing past the close
 *      visually re-enters the session (scoping/dimming/status all resync).
 *   6. Redo walks forward again.
 *
 * A second journey covers the "emptied definition" edge: open a session,
 * delete every member, close — the definition (and every instance of it)
 * is deleted outright; undo restores it.
 *
 * Ground truth is mostly the harness's structural queries
 * (`getInstancesOf`/`getComponentMemberObjects`/`getObjectCount`/
 * `getObjectBounds`), matching component-edit.spec.ts's own pyramid-shaped
 * approach; the dimming check is the one place this file reaches for a
 * screenshot diff, since opacity has no structural query.
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
 *  same flake-free pattern component-edit.spec.ts and follow-me-face-
 *  pointer.spec.ts already use. */
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

/** A real click-drag-click through a two-click tool gesture (Push/Pull): a
 *  first click at `anchor` arms the drag, a real mouse move to `target`
 *  drags it, a second click at `target` commits — tools.spec.ts's own
 *  proven Push/Pull drag pattern. Reliable here (unlike component-edit.spec.ts's
 *  K1/K2 attempts) because a session member is a PLAIN WORLD OBJECT for the
 *  edit's duration — no instance pose is involved in resolving the drag. */
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

/** A real double-click at a world point — opens an explode session on a
 *  uniformly-posed component instance (the default double-click gesture). */
async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

// A 3/4 view of a 2×1×1 m box at the origin, with instance B and the
// unrelated dimming-check box both comfortably out of frame (never clicked,
// only used for structural/screenshot checks) and empty ground (x≈3) for
// the idle-lock-free rectangle draw on the box's own top-adjacent face.
const CAMERA: CameraParams = {
  position: { x: 5, y: -5, z: 5 },
  target: { x: 1.2, y: 0.5, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.1,
  far: 1000,
}

test('explode session: open, edit two member faces, close folds both instances, then undo/redo the granular walk', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // ---- setup: one box folded into a component with TWO instances, plus one
  // UNRELATED box elsewhere (never part of the component) to prove dimming
  // reaches "everything else", not just sibling instances.
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const instanceB = h.placeInstance(component, 5, 0, 0)
    const unrelated = h.drawBox([-3, -3, 0], [-1, -2, 0], 1)
    return { instance, component, instanceB, unrelated }
  })
  const { component } = setup

  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)))
    .toEqual(new Set([setup.instance, setup.instanceB]))
  const members0 = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  expect(members0).toHaveLength(1)
  const memberId = members0[0]

  // ---- 1. Double-click instance A open — a plain double-click on a
  // uniformly-posed instance opens a session by default now.
  const beforeShot = await page.locator('canvas').first().screenshot()
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(200)

  // Sibling instance B is hidden kernel-side: `instances_of` (which the
  // harness's getInstancesOf wraps) filters hidden instances out entirely.
  expect(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).toEqual([])
  // The member is now a WORLD-owned object (baked, same id) — object_ids()
  // (`getObjectCount`) counts only world-tree objects, so it now includes
  // both the unrelated box AND the newly-baked member.
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(2)
  expect(await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId))
    .toEqual([0, 0, 0, 2, 1, 1])

  // Dimming: the UNRELATED box (never part of the session) fades; a
  // screenshot diff is the only observable signal opacity has (no
  // structural query for render state) — the same "something visibly
  // changed" proof component-edit.spec.ts's circle-imprint check uses.
  const afterOpenShot = await page.locator('canvas').first().screenshot()
  expect(afterOpenShot.equals(beforeShot)).toBe(false)
  const hashAfterOpen = await page.evaluate(() => window.__hew_test!.getStateHash())

  // ---- 2. Push/Pull a member face outward — a REAL drag, reliable here
  // (unlike component-edit.spec.ts's K1/K2 case) because the member is a
  // plain world object for the edit's duration, not reached through any
  // instance pose.
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [1, 0.5, 1], [1, 0.5, 2])
  const boundsAfterPush = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(boundsAfterPush[5]).toBeGreaterThan(1.1) // maxZ grew past the original 1
  const hashAfterTopPush = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfterTopPush).not.toBe(hashAfterOpen)

  // ---- 3. Draw a rectangle on the FRONT member face and Push/Pull the new
  // sub-face outward — brand-new mid-session geometry, expected to fold
  // into the definition at close.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0.9, 0, 0.4)
  await clickWorld(page, ctx, 1.1, 0, 0.6)
  // Fail fast, with a clear signal, if the rectangle imprint didn't commit —
  // rather than a confusing mismatch several steps later at the undo walk.
  const hashAfterRectangle = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfterRectangle).not.toBe(hashAfterTopPush)
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [1.0, 0, 0.5], [1.0, -2, 0.5])
  const boundsAfterExtrude = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(boundsAfterExtrude[1]).toBeLessThan(-0.1) // minY protrudes past the front face

  // ---- 4. Escape closes the session — both instances render both edits.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)))
    .toEqual(new Set([setup.instance, setup.instanceB]))
  const membersAfterClose = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  // BOTH edits act on the SAME member (a push/pull and a sub-face extrude
  // are per-object ops, not births), so membership is EXACTLY the original
  // one member — a second entry would mean an edit escaped into a separate
  // object and got folded in, which is not what this journey does. The
  // closing screenshot must differ from the original, on BOTH instances'
  // renders (the shared definition, so any placement shows it).
  expect(membersAfterClose).toHaveLength(1)
  const afterCloseShot = await page.locator('canvas').first().screenshot()
  expect(afterCloseShot.equals(beforeShot)).toBe(false)

  /** Undo in a loop until the document satisfies `reached` — a STRUCTURAL
   *  checkpoint, not a `state_hash` one, and deliberately so: undo of a
   *  translate-and-build push/pull reverses the build with collapse surgery
   *  whose vertex arithmetic is not guaranteed BIT-identical to the
   *  pre-push state for every drag geometry (the same territory as the
   *  roadmap's deferred `UnbuildPushPull` notes), so pinning hash equality
   *  across an undo of arbitrary geometry ops flakes with the drag's exact
   *  pixels. Each predicate below is chosen to be FALSE at every state the
   *  walk passes through before its target, so "first state satisfying it"
   *  is exactly the checkpoint. The loop still exists because a drawing
   *  gesture's commit can bracket into more than one undo step, and this
   *  test has no business pinning that internal count. */
  async function undoUntil(reached: () => Promise<boolean>, what: string): Promise<void> {
    for (let i = 0; i < 25; i++) {
      if (await reached()) return
      expect(await page.evaluate(() => window.__hew_test!.canUndo()), `ran out of undo before: ${what}`).toBe(true)
      await page.evaluate(() => window.__hew_test!.undo())
      // A short settle: runUndo's post-history reconciliation (tag visibility,
      // explode-session rescope/dimming) is real React state, batched onto a
      // later tick — give it a moment before the next read/undo.
      await page.waitForTimeout(30)
    }
    expect(await reached(), `never reached: ${what}`).toBe(true)
  }
  const memberBounds = () =>
    page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  const sessionOpen = async () =>
    (await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).length === 0

  // ---- 5. Undo the granular walk in reverse: close -> extrude -> push/pull
  // -> open. Undoing past the close visually RE-ENTERS the session — the
  // scoping/dimming/status resync (Viewport.tsx's applyHistoryChange).
  // Undoes the close: back in the session (instances hidden) with BOTH
  // edits still present (front boss protrudes, top boss raised).
  await undoUntil(async () => {
    if (!(await sessionOpen())) return false
    const b = await memberBounds()
    return b[1] < -0.1 && b[5] > 1.1
  }, 'session reopened with both edits')
  await page.waitForTimeout(150)
  expect(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).toEqual([])
  const afterUndoCloseShot = await page.locator('canvas').first().screenshot()
  // Re-entering the session re-dims the unrelated box again — visually
  // distinct from the (undimmed) post-close render just captured.
  expect(afterUndoCloseShot.equals(afterCloseShot)).toBe(false)

  // Undoes the front-face extrude and its rectangle imprint: front boss
  // retracted, top boss still present, still in the session.
  await undoUntil(async () => {
    if (!(await sessionOpen())) return false
    const b = await memberBounds()
    return b[1] > -0.001 && b[5] > 1.1
  }, 'front boss retracted, top boss standing')
  await page.waitForTimeout(150)
  const afterUndoExtrude = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(afterUndoExtrude[1]).toBeCloseTo(0, 6) // front-face boss retracted
  expect(afterUndoExtrude[5]).toBeGreaterThan(1.1) // top push/pull's boss is still there

  // Undoes the top push/pull: the pristine baked box, still in the session.
  await undoUntil(async () => {
    if (!(await sessionOpen())) return false
    const b = await memberBounds()
    return Math.abs(b[5] - 1) < 0.001
  }, 'pristine member, session still open')
  await page.waitForTimeout(150)
  const afterUndoPush = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  // Element-wise near-equality, not toEqual: the un-build of a push/pull is
  // not guaranteed BIT-identical (see undoUntil's doc), only geometrically
  // identical.
  for (const [i, v] of [0, 0, 0, 2, 1, 1].entries()) {
    expect(afterUndoPush[i]).toBeCloseTo(v, 9)
  }

  // Undoes the open itself: both instances visible again.
  await undoUntil(async () => !(await sessionOpen()), 'session closed by undoing the open')
  await page.waitForTimeout(150)
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)))
    .toEqual(new Set([setup.instance, setup.instanceB]))

  // ---- 6. Redo walks forward again, back to the fully-closed, both-edits state.
  let redoIterations = 0
  while (redoIterations < 30 && (await page.evaluate(() => window.__hew_test!.canRedo()))) {
    await page.evaluate(() => window.__hew_test!.redo())
    await page.waitForTimeout(30)
    redoIterations++
  }
  await page.waitForTimeout(150)
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)))
    .toEqual(new Set([setup.instance, setup.instanceB]))
  const membersAfterRedo = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  expect(membersAfterRedo.length).toBe(membersAfterClose.length)
  // No hash pin here for the same reason undoUntil documents: the redo
  // replays from an undo base that is geometrically — not bitwise —
  // identical. Structure above (instances, membership) plus the kernel's
  // own explode_session_specs (which assert the redo chain end-state
  // structurally, spec 2) carry this.
  expect(await page.evaluate(() => window.__hew_test!.canRedo())).toBe(false)
})

test('explode session: deleting every member closes and deletes the component; undo restores it', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    return h.makeComponent([box])
  })
  const { component } = setup
  const hashBefore = await page.evaluate(() => window.__hew_test!.getStateHash())

  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)

  // Select the (now world-owned) member with a real click, delete it with a
  // real Delete keypress — the app's ordinary object-delete gesture.
  await page.keyboard.press('Space')
  await clickWorld(page, ctx, 1, 0.5, 1)
  await expect.poll(async () => page.evaluate(
    () => window.__hew_test!.getSelection().map((n) => n.kind),
  )).toEqual(['object'])
  await page.keyboard.press('Delete')
  await page.waitForTimeout(150)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(0)

  // Close the now-emptied session — the definition has nothing left to fold
  // back into, so it (and every instance of it) is deleted outright.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  const nameAfterClose = await page.evaluate(
    (c) => {
      try {
        return window.__hew_test!.getComponentName(c)
      } catch {
        return 'THREW'
      }
    },
    component,
  )
  expect(nameAfterClose === null || nameAfterClose === 'THREW').toBe(true)
  expect(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).toEqual([])

  // Undo restores the component, its instance, and its one member.
  await page.evaluate(() => window.__hew_test!.undo()) // undoes the close/definition-delete
  await page.waitForTimeout(150)
  await page.evaluate(() => window.__hew_test!.undo()) // undoes the member delete
  await page.waitForTimeout(150)
  await page.evaluate(() => window.__hew_test!.undo()) // undoes the open
  await page.waitForTimeout(150)
  const hashAfter = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfter).toBe(hashBefore)
  const membersRestored = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  expect(membersRestored).toHaveLength(1)
})

test('explode session: an edit undone inside a reopened session stays undone after a fresh close', async ({
  page,
}) => {
  // The playtest sequence: rotate a member, close, undo back in, undo the
  // rotate, Escape out again — the definition must be back to the original
  // geometry (the reconstructed session inherits the ORIGINAL pristine
  // snapshots; re-snapshotting current geometry made this close resurrect
  // the undone rotate).
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const instanceB = h.placeInstance(component, 3, 0, 0)
    return { instance, component, instanceB }
  })
  const { component } = setup
  const members = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  const memberId = members[0]

  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await page.waitForTimeout(200)
  const boundsPristine = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)

  // Rotate the member -45° about Z (the harness path RotateTool commits).
  await page.evaluate((id) => window.__hew_test!.rotateObject(id, -45), memberId)
  await page.waitForTimeout(100)
  const boundsRotated = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(boundsRotated).not.toEqual(boundsPristine)

  await page.keyboard.press('Escape') // close — both instances rotated
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()

  await page.evaluate(() => window.__hew_test!.undo()) // back into the session
  await page.waitForTimeout(150)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).not.toBeNull()
  await page.evaluate(() => window.__hew_test!.undo()) // undo the rotate
  await page.waitForTimeout(150)

  await page.keyboard.press('Escape') // fresh close
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()

  // Definition geometry is back to the pristine unit box: re-open and check
  // the member's world bounds match the original session's.
  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await page.waitForTimeout(200)
  const boundsAfter = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  for (const [i, v] of boundsPristine.entries()) {
    expect(boundsAfter[i]).toBeCloseTo(v, 9)
  }
  await page.keyboard.press('Escape')
})

test('explode session: a solid created mid-session is immediately click-selectable', async ({
  page,
}) => {
  // Delta-review finding: the viewport's pick scope refreshed only at
  // open/close/undo boundaries, so a solid drawn mid-session (which folds
  // into the definition at close, i.e. IS session geometry from birth) had
  // an Outliner row but was invisible to viewport click/marquee until the
  // next boundary. The scope now re-derives at the commit choke point.
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.makeComponent([box])
  })
  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).not.toBeNull()

  // A brand-new, disconnected solid born mid-session (harness commit runs
  // the same shared commit path as any tool).
  await page.evaluate(() => window.__hew_test!.drawBox([2, 0, 0], [3, 1, 0], 1))
  await page.waitForTimeout(150)

  // Real click on the new solid selects it — no session boundary between.
  await page.keyboard.press('Space')
  await clickWorld(page, ctx, 2.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(
    () => window.__hew_test!.getSelection().map((n) => n.kind),
  )).toEqual(['object'])

  await page.keyboard.press('Escape')
})
