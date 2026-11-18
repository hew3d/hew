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
 *      and the unrelated box both prove out: only A hides kernel-side
 *      (`instances_of` keeps B — a LIVE sibling placement that renders the
 *      session's world geometry, ARCHITECTURE.md 2.11; live-siblings.spec.ts
 *      drives that rendering), the unrelated box is merely dimmed (still a
 *      live, visible, un-hidden world object — just faded).
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

  // Only the ENTERED instance A hides kernel-side: `instances_of` (which the
  // harness's getInstancesOf wraps) filters hidden instances out, and the
  // sibling B stays live — a live sibling placement drawing the session's
  // world member (ARCHITECTURE.md 2.11).
  expect(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).toEqual([setup.instanceB])
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(setup.instance)
  expect(await page.evaluate(() => window.__hew_test!.getSessionPlacements())).toEqual([setup.instanceB])
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceExpandedMembers(i), setup.instanceB)).toEqual([memberId])
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
  const hashAfterOpen = await page.evaluate(() => window.__hew_test!.getModelStateHash())

  // ---- 2. Push/Pull a member face outward — a REAL drag, reliable here
  // (unlike component-edit.spec.ts's K1/K2 case) because the member is a
  // plain world object for the edit's duration, not reached through any
  // instance pose.
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [1, 0.5, 1], [1, 0.5, 2])
  const boundsAfterPush = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(boundsAfterPush[5]).toBeGreaterThan(1.1) // maxZ grew past the original 1
  const hashAfterTopPush = await page.evaluate(() => window.__hew_test!.getModelStateHash())
  expect(hashAfterTopPush).not.toBe(hashAfterOpen)

  // ---- 3. Draw a rectangle on the FRONT member face and Push/Pull the new
  // sub-face outward — brand-new mid-session geometry, expected to fold
  // into the definition at close.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0.9, 0, 0.4)
  await clickWorld(page, ctx, 1.1, 0, 0.6)
  // Fail fast, with a clear signal, if the rectangle imprint didn't commit —
  // rather than a confusing mismatch several steps later at the undo walk.
  const hashAfterRectangle = await page.evaluate(() => window.__hew_test!.getModelStateHash())
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
    (await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())) !== null

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
  expect(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).toEqual([setup.instanceB])
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
  const hashBefore = await page.evaluate(() => window.__hew_test!.getModelStateHash())

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
  const hashAfter = await page.evaluate(() => window.__hew_test!.getModelStateHash())
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

test('explode session: a line folded into a component stays drawn when the component is opened or exploded', async ({
  page,
}) => {
  // A loose line drawn on the ground inside a session, outside the part,
  // folds into the definition at close: it renders through the instance
  // from then on. Opening the session again and exploding the instance each
  // move that drawing back to a world sketch while the instance hides, and
  // the viewport must redraw the world sketches at that moment: they used
  // to be redrawn only on the next drawing change, so in a freshly opened
  // file the line vanished from view while it was still in the document.
  let ctx = await ready(page).then(() => aim(page, CAMERA))
  const { component } = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    return h.makeComponent([box])
  })
  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).not.toBeNull()
  await page.keyboard.press('l')
  await clickWorld(page, ctx, 2, 0.1, 0)
  await clickWorld(page, ctx, 2, 0.9, 0)
  await page.keyboard.press('Escape') // end the line chain
  await page.keyboard.press('Space')
  await page.keyboard.press('Escape') // close the session
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  // The line folded in: a definition sketch, no world sketch.
  expect(await page.evaluate((c) => window.__hew_test!.getComponentMemberSketches(c), component)).toHaveLength(1)
  expect(await page.evaluate(() => window.__hew_test!.getSketchIds())).toHaveLength(0)
  // Where the line landed (the clicks snap); the instance's pose is the
  // identity, so its definition coordinates are world coordinates.
  const [sketch] = await page.evaluate((c) => window.__hew_test!.getComponentMemberSketches(c), component)
  const seg = await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch)
  const mid: [number, number, number] = [(seg[0] + seg[3]) / 2, (seg[1] + seg[4]) / 2, (seg[2] + seg[5]) / 2]
  const ground: [number, number, number] = [mid[0] + 0.3, mid[1], mid[2]]
  // The line is drawn when the pixel on it differs clearly from the bare
  // ground beside it.
  const lineDrawn = async (): Promise<boolean> =>
    page.evaluate(
      ([on, off]) => {
        const t = window.__hew_test!
        const a = t.pixelColorAt(on)
        const b = t.pixelColorAt(off)
        if (a === null || b === null) return false
        return Math.abs(a.r - b.r) + Math.abs(a.g - b.g) + Math.abs(a.b - b.b) > 60
      },
      [mid, ground] as const,
    )
  expect(await lineDrawn()).toBe(true)

  // Save and reopen, so nothing drawn during the session lingers on screen.
  const bytes = await page.evaluate(() => window.__hew_test!.save())
  await page.evaluate((b) => window.__hew_test!.load(b), bytes)
  ctx = await aim(page, CAMERA)
  await expect.poll(lineDrawn).toBe(true)

  // Opening the session bakes it to a world sketch: still drawn.
  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).not.toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getSketchIds())).toHaveLength(1)
  await expect.poll(lineDrawn).toBe(true)

  await page.keyboard.press('Escape')
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  await expect.poll(lineDrawn).toBe(true)

  // Exploding copies it out as a world sketch: still drawn.
  await clickWorld(page, ctx, 0.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.kind))).toEqual(['instance'])
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object', exact: true }).click()
  await page.getByTestId('menu-bar').getByText('Explode', { exact: true }).click()
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getSketchIds())).toHaveLength(1)
  await expect.poll(lineDrawn).toBe(true)
})

test('a line folded into a component selects the component: hover names it, a click selects it, a crossing sweep takes it', async ({
  page,
}) => {
  // A line drawn inside a component renders through the instance, but no
  // snap names it (the inference index leaves definition drawing out), so
  // hovering it read as bare ground and clicking it selected nothing. As in
  // SketchUp, a click on any entity of a closed component selects the
  // component, and the component's box takes the line in.
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const { component } = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    return h.makeComponent([box])
  })
  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).not.toBeNull()
  await page.keyboard.press('l')
  await clickWorld(page, ctx, 2, 0.1, 0)
  await clickWorld(page, ctx, 2, 0.9, 0)
  await page.keyboard.press('Escape') // end the line chain
  await page.keyboard.press('Space')
  await page.keyboard.press('Escape') // close the session
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  const [sketch] = await page.evaluate((c) => window.__hew_test!.getComponentMemberSketches(c), component)
  const seg = await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch)
  const mid: [number, number, number] = [(seg[0] + seg[3]) / 2, (seg[1] + seg[4]) / 2, (seg[2] + seg[5]) / 2]
  await page.evaluate(() => window.__hew_test!.selectNodes([]))

  // Hover: the tip's second line names the component, not just the ground.
  const at = px(ctx, mid[0], mid[1], mid[2])
  await page.mouse.move(at.x - 4, at.y - 4)
  await page.mouse.move(at.x, at.y)
  await expect(page.getByTestId('inference-tip-detail')).toContainText(/Component/, { timeout: 3000 })

  // Click: the instance is selected.
  await clickWorld(page, ctx, mid[0], mid[1], mid[2])
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.kind))).toEqual(['instance'])

  // Its box encloses the line, which lies outside the cube.
  const b = await page.evaluate(() => window.__hew_test!.getSelectionBounds())
  expect(b).not.toBeNull()
  for (const k of [0, 1, 2]) {
    expect(b![k]).toBeLessThanOrEqual(Math.min(seg[k], seg[k + 3]) + 1e-6)
    expect(b![k + 3]).toBeGreaterThanOrEqual(Math.max(seg[k], seg[k + 3]) - 1e-6)
  }

  // A right-to-left crossing sweep over the line alone takes the instance.
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  const right = px(ctx, mid[0] + 0.15, mid[1] + 0.05, mid[2])
  const left = px(ctx, mid[0] - 0.15, mid[1] - 0.05, mid[2])
  const sweep = {
    minX: Math.min(left.x, right.x) - 20, maxX: Math.max(left.x, right.x) + 20,
    minY: Math.min(left.y, right.y) - 20, maxY: Math.max(left.y, right.y) + 20,
  }
  // The sweep stays clear of the cube, so only the line can take it in.
  for (const c of [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]]) {
    const q = px(ctx, c[0], c[1], c[2])
    expect(q.x < sweep.minX || q.x > sweep.maxX || q.y < sweep.minY || q.y > sweep.maxY).toBe(true)
  }
  await page.mouse.move(right.x + 20, right.y - 20)
  await page.mouse.down()
  await page.mouse.move(left.x - 20, left.y + 20, { steps: 12 })
  await page.mouse.up()
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.kind))).toEqual(['instance'])
})

/** Folds a line into a fresh component of one unit box at (x0, 0): opens its
 *  session by a real double-click, draws a line along x = x0 + 2 (outside the
 *  cube), and closes. Returns the instance, definition and the line's
 *  endpoints in world space (the instance sits at the identity pose). */
async function componentWithLine(
  page: Page,
  ctx: Ctx,
  x0: number,
): Promise<{ instance: string; component: string; mid: [number, number, number] }> {
  const { instance, component } = await page.evaluate((x) => {
    const h = window.__hew_test!
    const box = h.drawBox([x, 0, 0], [x + 1, 1, 0], 1)
    return h.makeComponent([box])
  }, x0)
  await dblClickWorld(page, ctx, x0 + 0.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(instance)
  await page.keyboard.press('l')
  await clickWorld(page, ctx, x0 + 2, 0.1, 0)
  await clickWorld(page, ctx, x0 + 2, 0.9, 0)
  await page.keyboard.press('Escape') // end the line chain
  await page.keyboard.press('Space')
  await page.keyboard.press('Escape') // close the session
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  const [sketch] = await page.evaluate((c) => window.__hew_test!.getComponentMemberSketches(c), component)
  const seg = await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch)
  const mid: [number, number, number] = [(seg[0] + seg[3]) / 2, (seg[1] + seg[4]) / 2, (seg[2] + seg[5]) / 2]
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  return { instance, component, mid }
}

const selectionKinds = async (page: Page): Promise<string[]> =>
  page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.kind))

test('double-clicking a component\'s line opens the component, even over a solid face the line lies on', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const { instance, mid } = await componentWithLine(page, ctx, 0)
  // A solid whose top face lies in the line's own plane, under the line: the
  // face is under the cursor too, and must not take the double-click.
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([1.5, -0.5, 0], [2.5, 1.5, 0], -1)
  })
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  const at = px(ctx, mid[0], mid[1], mid[2])
  await page.mouse.move(at.x - 4, at.y - 4)
  await page.mouse.move(at.x, at.y)
  await page.waitForTimeout(150)
  await page.mouse.click(at.x, at.y, { clickCount: 2 })
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(instance)
})

test('a component\'s line under the cursor beats a guide lying along it', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const { mid } = await componentWithLine(page, ctx, 0)
  // A guide along the line, as guides are often placed on edges.
  await page.evaluate((m) => window.__hew_test!.addGuideLine(m[0], 0, m[2], 0, 1, 0), mid)
  await clickWorld(page, ctx, mid[0], mid[1], mid[2])
  await expect.poll(() => selectionKinds(page)).toEqual(['instance'])
  expect(await page.evaluate(() => window.__hew_test!.getSelectedGuide())).toBeNull()
  // Off the line the guide is still selectable.
  await clickWorld(page, ctx, mid[0], 3, mid[2])
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getSelectedGuide())).not.toBeNull()
})

test('a window marquee counts a component\'s outlying line in its extent', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  await componentWithLine(page, ctx, 0)
  const drag = async (from: { x: number; y: number }, to: { x: number; y: number }): Promise<void> => {
    await page.mouse.move(from.x, from.y)
    await page.mouse.down()
    await page.mouse.move(to.x, to.y, { steps: 12 })
    await page.mouse.up()
    await page.waitForTimeout(150)
  }
  const corners = (pts: [number, number, number][], pad: number) => {
    const ps = pts.map((p) => px(ctx, p[0], p[1], p[2]))
    return {
      min: { x: Math.min(...ps.map((p) => p.x)) - pad, y: Math.min(...ps.map((p) => p.y)) - pad },
      max: { x: Math.max(...ps.map((p) => p.x)) + pad, y: Math.max(...ps.map((p) => p.y)) + pad },
    }
  }
  const cube: [number, number, number][] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]]
  // Left to right around the cube alone: the line two metres out is a drawn
  // part of the component, so the window neither encloses nor covers it.
  const tight = corners(cube, 12)
  await drag(tight.min, tight.max)
  expect(await selectionKinds(page)).toEqual([])
  // Around the cube and the line: enclosed.
  const all = corners([...cube, [2, 0.1, 0], [2, 0.9, 0]], 14)
  await drag(all.min, all.max)
  await expect.poll(() => selectionKinds(page)).toEqual(['instance'])
})

test('in a group, a line inside a component outside the group is out of scope and masks nothing', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const a = await componentWithLine(page, ctx, 0)
  const b = await componentWithLine(page, ctx, 3)
  // A holds a group with another box; B stays at the top level.
  const filler = await page.evaluate(() => window.__hew_test!.drawBox([0, 3, 0], [1, 4, 0], 1))
  const group = await page.evaluate(
    ([inst, obj]) => window.__hew_test!.groupNodes([{ kind: 'instance', id: inst }, { kind: 'object', id: obj }]),
    [a.instance, filler] as const,
  )
  await dblClickWorld(page, ctx, 0.5, 3.5, 1) // enter the group
  await page.waitForTimeout(250)
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  // B's line (outside the group): nothing.
  await clickWorld(page, ctx, b.mid[0], b.mid[1], b.mid[2])
  expect(await selectionKinds(page)).toEqual([])
  // A's line (inside it): the component.
  await clickWorld(page, ctx, a.mid[0], a.mid[1], a.mid[2])
  await expect.poll(() => selectionKinds(page)).toEqual(['instance'])
  expect(group).toBeTruthy()
})

test('a line drawn in a nested component draws, and picks, through the outer component', async ({ page }) => {
  const camera: CameraParams = {
    position: { x: 8, y: -8, z: 8 }, target: { x: 2.5, y: 1.5, z: 0 }, up: { x: 0, y: 0, z: 1 }, fovDeg: 50, near: 0.1, far: 1000,
  }
  const ctx = await ready(page).then(() => aim(page, camera))
  const part = await componentWithLine(page, ctx, 0)
  const { assembly, assemblyInstance } = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    const r = h.makeComponent([box])
    return { assembly: r.component, assemblyInstance: r.instance }
  })
  // Fold a placed part (with its line) into the assembly as a nested member.
  await dblClickWorld(page, ctx, 3.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(assemblyInstance)
  await page.evaluate(([c]) => window.__hew_test!.placeInstance(c, 3, 3, 0), [part.component] as const)
  await page.keyboard.press('Escape')
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  expect(assembly).toBeTruthy()
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  // The nested part's line sits at world (5, 3.5) through the assembly's
  // nested placement (+3, +3): clicking there selects the assembly.
  const lineMid: [number, number, number] = [part.mid[0] + 3, part.mid[1] + 3, part.mid[2]]
  const lineDrawn = async (): Promise<boolean> =>
    page.evaluate(
      ([on, off]) => {
        const t = window.__hew_test!
        const a = t.pixelColorAt(on)
        const b = t.pixelColorAt(off)
        if (a === null || b === null) return false
        return Math.abs(a.r - b.r) + Math.abs(a.g - b.g) + Math.abs(a.b - b.b) > 60
      },
      [lineMid, [lineMid[0] + 0.3, lineMid[1], lineMid[2]]] as const,
    )
  await expect.poll(lineDrawn).toBe(true)
  await clickWorld(page, ctx, lineMid[0], lineMid[1], lineMid[2])
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.id))).toEqual([assemblyInstance])
})

test('ungrouping a hidden group, and undoing it, keeps the lines inside its components in step with the component', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const { instance, mid } = await componentWithLine(page, ctx, 0)
  const differs = (a: [number, number, number], b: [number, number, number]) =>
    page.evaluate(
      ([on, off]) => {
        const t = window.__hew_test!
        const p = t.pixelColorAt(on)
        const q = t.pixelColorAt(off)
        if (p === null || q === null) return false
        return Math.abs(p.r - q.r) + Math.abs(p.g - q.g) + Math.abs(p.b - q.b) > 60
      },
      [a, b] as const,
    )
  const lineDrawn = (): Promise<boolean> => differs(mid, [mid[0] + 0.3, mid[1], mid[2]])
  const cubeDrawn = (): Promise<boolean> => differs([0.5, 0.5, 1], [0.5, 0.5, 1.6])
  // The line is drawn exactly when its component is.
  const inStep = async (): Promise<boolean> => (await lineDrawn()) === (await cubeDrawn())
  const group = await page.evaluate((i) => window.__hew_test!.groupNodes([{ kind: 'instance', id: i }]), instance)
  await expect.poll(lineDrawn).toBe(true)
  await expect.poll(cubeDrawn).toBe(true)
  await page.evaluate((g) => window.__hew_test!.toggleNodeHidden({ kind: 'group', id: g }), group)
  await expect.poll(lineDrawn).toBe(false)
  await expect.poll(cubeDrawn).toBe(false)
  await page.evaluate((g) => window.__hew_test!.selectNodes([{ kind: 'group', id: g }]), group)
  await expect.poll(() => selectionKinds(page)).toEqual(['group'])
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object', exact: true }).click()
  await page.getByTestId('menu-bar').getByText('Ungroup', { exact: true }).click()
  await expect.poll(() => selectionKinds(page)).toEqual([])
  await expect.poll(inStep).toBe(true)
  // Undo brings the group back; the line follows the component again.
  await page.keyboard.press('Control+z')
  await expect.poll(inStep).toBe(true)
  await page.keyboard.press('Control+Shift+Z')
  await expect.poll(inStep).toBe(true)
})

/** A component whose only drawing is one diagonal line from (x0+2, 0.1) to
 *  (x0+3, 0.9): its box is deleted inside the session, so the placement
 *  holds lines and no solid. Returns the instance and the line's endpoints. */
async function lineOnlyComponent(
  page: Page,
  ctx: Ctx,
  x0: number,
): Promise<{ instance: string; component: string; a: [number, number, number]; b: [number, number, number] }> {
  const made = await page.evaluate((x) => {
    const h = window.__hew_test!
    const box = h.drawBox([x, 0, 0], [x + 1, 1, 0], 1)
    return { ...h.makeComponent([box]), box }
  }, x0)
  await dblClickWorld(page, ctx, x0 + 0.5, 0.5, 1)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(made.instance)
  await page.keyboard.press('l')
  await clickWorld(page, ctx, x0 + 2, -1.5, 0)
  await clickWorld(page, ctx, x0 + 3.5, 2.5, 0)
  await page.keyboard.press('Escape')
  await page.keyboard.press('Space')
  await page.evaluate((id) => window.__hew_test!.deleteObject(id), made.box)
  await page.keyboard.press('Escape')
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  const [sketch] = await page.evaluate((c) => window.__hew_test!.getComponentMemberSketches(c), made.component)
  const seg = await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch)
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  return {
    instance: made.instance,
    component: made.component,
    a: [seg[0], seg[1], seg[2]],
    b: [seg[3], seg[4], seg[5]],
  }
}

/** A real left-to-right (window) or right-to-left (crossing) drag between two
 *  screen points. */
async function marquee(page: Page, from: { x: number; y: number }, to: { x: number; y: number }): Promise<void> {
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps: 12 })
  await page.mouse.up()
  await page.waitForTimeout(150)
}

test('a lone diagonal line: a window must enclose all of it, a crossing needs only a touch', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const { instance, component, a, b } = await lineOnlyComponent(page, ctx, 0)
  expect(instance).toBeTruthy()
  // Lines and no solid: the placement is a lines-only instance.
  expect(await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)).toEqual([])
  expect(await page.evaluate((c) => window.__hew_test!.getComponentMemberSketches(c).length, component)).toBe(1)
  const pa = px(ctx, a[0], a[1], a[2])
  const pb = px(ctx, b[0], b[1], b[2])
  const lo = { x: Math.min(pa.x, pb.x), y: Math.min(pa.y, pb.y) }
  const hi = { x: Math.max(pa.x, pb.x), y: Math.max(pa.y, pb.y) }
  // A window around the whole bounding rect but one corner: it swallows 94%
  // of the line's rect yet leaves one endpoint outside, and a line has no
  // screen area to "cover": not selected.
  const w = hi.x - lo.x
  const h = hi.y - lo.y
  // The line must read as a true diagonal on screen for this to mean anything.
  expect(Math.min(w, h)).toBeGreaterThan(60)
  await marquee(page, { x: lo.x - 20, y: lo.y - 20 }, { x: lo.x + w * 0.97, y: lo.y + h * 0.97 })
  expect(await selectionKinds(page)).toEqual([])
  // Enclosing both endpoints: selected.
  await marquee(page, { x: lo.x - 20, y: lo.y - 20 }, { x: hi.x + 20, y: hi.y + 20 })
  await expect.poll(() => selectionKinds(page)).toEqual(['instance'])
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  // Crossing whose far corner just passes the line's midpoint: selected.
  const mid = { x: (pa.x + pb.x) / 2, y: (pa.y + pb.y) / 2 }
  const len = Math.hypot(pb.x - pa.x, pb.y - pa.y)
  // 40 px off the line, on the side that makes the drag run right to left.
  const k = pb.y > pa.y ? -1 : 1
  const side = { x: (k * -(pb.y - pa.y) / len) * 40, y: (k * (pb.x - pa.x) / len) * 40 }
  await marquee(page, { x: mid.x + side.x, y: mid.y + side.y }, { x: mid.x - side.x * 0.2, y: mid.y - side.y * 0.2 })
  await expect.poll(() => selectionKinds(page)).toEqual(['instance'])
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  // Crossing that stays clear of the line (beside its far end): nothing.
  await marquee(page, { x: hi.x + 60, y: lo.y - 20 }, { x: hi.x + 30, y: lo.y + 10 })
  expect(await selectionKinds(page)).toEqual([])
})

test('a component with lines: crossing takes it by a touch on the line alone, and a window covering most of its extent takes it', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  await componentWithLine(page, ctx, 0)
  const cube: [number, number, number][] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]]
  const all = [...cube, [2, 0.1, 0], [2, 0.9, 0]] as [number, number, number][]
  const ps = all.map((p) => px(ctx, p[0], p[1], p[2]))
  const minX = Math.min(...ps.map((p) => p.x))
  const maxX = Math.max(...ps.map((p) => p.x))
  const minY = Math.min(...ps.map((p) => p.y))
  const maxY = Math.max(...ps.map((p) => p.y))
  // Crossing across the outlying line only (clear of the cube): selected.
  const l = px(ctx, 2, 0.5, 0)
  await marquee(page, { x: l.x + 15, y: l.y + 15 }, { x: l.x - 3, y: l.y - 3 })
  await expect.poll(() => selectionKinds(page)).toEqual(['instance'])
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  // A window over ~90% of the extent (cube plus line) leaves the line's far
  // end out yet covers most of it: taken by coverage.
  const w = maxX - minX
  const h = maxY - minY
  await marquee(page, { x: minX - 20, y: minY - 20 }, { x: minX + w * 0.97, y: minY + h * 0.97 })
  await expect.poll(() => selectionKinds(page)).toEqual(['instance'])
  await page.evaluate(() => window.__hew_test!.selectNodes([]))
  // A window over only half: not enough.
  await marquee(page, { x: minX - 20, y: minY - 20 }, { x: minX + w * 0.5, y: minY + h * 0.5 })
  expect(await selectionKinds(page)).toEqual([])
})
