import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Component-edit parity — end to end with REAL pointer input
 * (component-edit-parity.md). No E2E existed for editing INSIDE a component
 * instance's own definition before this phase; every other modeling tool has
 * its own real-input spec (sketch-planes.spec.ts, follow-me-face-pointer.spec.ts),
 * but "draw/push-pull/sweep while double-clicked into a component" had no
 * end-to-end coverage at all.
 *
 * The journey drives one 2×1×1 box, folded into a component with TWO
 * instances (so "does the edit reach every instance" has a real second
 * instance to check against, not just the one the tool clicked through):
 *
 *   1. Double-click into instance A's own editing context (real dblclick).
 *   2. Draw a small rectangle on the FRONT member face with the REAL
 *      Rectangle tool (face mode: `split_face_inner_in_instance`).
 *   3. Push/Pull that new sub-face outward with the REAL Push/Pull tool
 *      (`push_pull_in_component`, already-existing K1 capability — proven
 *      undisturbed by the A1/A2 channel refactor).
 *   4. Arrow-key-lock a plane and draw a SECOND rectangle in EMPTY space
 *      with the REAL Rectangle tool — THE original axis-lock symptom: before
 *      this phase, plane-mode drawing inside a component ignored the
 *      instance context entirely and landed in a WORLD sketch
 *      (`begin_ground_sketch`/`begin_sketch_on_plane`) instead of the
 *      definition (`begin_sketch_on_plane_in_instance`).
 *   5. Draw a small standing profile as its own def-owned sketch (another
 *      idle-locked plane, same gesture as step 4) straddling the box's own
 *      TOP-face rim.
 *   6. Follow Me that profile around the TOP face's loop with the REAL
 *      Follow Me tool (`follow_me_around_face_in_instance` — a sketch-region
 *      profile around a member's face-loop path). Before this phase Follow
 *      Me refused every face interaction inside a component's definition
 *      wholesale.
 *   7. Undo the whole chain and confirm the definition (and so BOTH
 *      instances) is back to its starting shape.
 *   8. Exit the context (Escape) and prove a draw AFTER exiting lands back
 *      in the world, not the definition.
 *
 * Ground truth throughout is the harness's component/object queries
 * (`getComponentMemberObjects`/`getComponentMemberSketches`/
 * `getInstancesOf`/`getObjectBounds`) — logical, not pixel, per the test
 * pyramid (docs/DEVELOPMENT.md): a member is SHARED storage, so proving an
 * edit landed on the definition's member list is exactly proving every
 * instance sees it — there is no separate per-instance copy to diverge.
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

/** Move first (so tools see a pointermove — snap resolve + preview — before
 *  the down, the same order a human hand produces), then click. The initial
 *  move lands a few px off-target first — a fresh move that isn't coalesced
 *  away — and each step gets a short settle wait, matching the pattern
 *  proven flake-free in follow-me-face-pointer.spec.ts's moveTo/clickAt: the
 *  hover pick (snap resolve, face-loop preview) runs off a rAF, and a
 *  same-tick move+down races it. */
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

/** A real double-click at a world point — the app's "enter this node's
 *  editing context" gesture (Viewport's default `dblclick` fallback; no
 *  tool consumes it while idle). */
async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

// A 3/4 view of a 2×1×1 m box: the front face (y=0), top face (z=1), and
// right face (x=2) are all clear on screen, with empty space to the right
// (x≈3) for the idle-locked empty-space draw.
const CAMERA: CameraParams = {
  position: { x: 5, y: -5, z: 5 },
  target: { x: 1.2, y: 0.5, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.1,
  far: 1000,
}

test('component-edit parity: draw, push/pull, idle-lock draw, and Follow Me inside a component instance — undo/exit round-trips both instances', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // ---- setup: one box, folded into a component with TWO instances -------
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const instanceB = h.placeInstance(component, 5, 0, 0)
    return { instance, component, instanceB }
  })
  const { component } = setup
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)))
    .toEqual(new Set([setup.instance, setup.instanceB]))

  const members0 = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  expect(members0).toHaveLength(1)
  const memberId = members0[0]
  const boundsBefore = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    memberId,
  )
  // The exact-match ground truth for the undo round-trip (step 7) — a
  // `state_hash` comparison sidesteps needing to know how many
  // gesture-bracketed kernel actions the five in-context edits actually
  // expand to (drawing tools may bracket a multi-segment commit into more
  // than one undo step): undoing back to THIS hash is undoing back to
  // exactly this state, full stop, regardless of that internal granularity.
  const hashBeforeContext = await page.evaluate(() => window.__hew_test!.getStateHash())

  // ---- 1. Double-click into instance A's own editing context ------------
  // A point on the box's top face — instance A sits at the identity pose
  // make_component gave it, so this is instance A's own member.
  await dblClickWorld(page, ctx, 1, 0.5, 1)

  // ---- 2. Rectangle tool, REAL clicks, on the FRONT member face ---------
  // A small patch dead center of the 2×1 front face (y=0) — deliberately
  // small so most of the face stays untouched, wide open ground for the
  // Follow Me profile click in step 5 (a click too close to ANY edge —
  // the face's own boundary or this patch's — reads as "on edge" rather
  // than "on face", too ambiguous to land reliably).
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0.9, 0, 0.4)
  await clickWorld(page, ctx, 1.1, 0, 0.6)

  const afterImprint = await page.evaluate(
    (c) => ({
      lastError: window.__hew_test!.getLastError(),
      objectCount: window.__hew_test!.getObjectCount(),
      members: window.__hew_test!.getComponentMemberObjects(c),
    }),
    component,
  )
  expect(afterImprint.lastError).toBeNull()
  // A face-mode imprint splits an existing face — no new Object is born, and
  // the definition's member LIST is unchanged (still the same one member).
  // `getObjectCount` counts only WORLD-tree objects (`visible_object_ids`) —
  // a definition member is reached solely through its component, never that
  // list, so it reads 0 throughout this whole in-component session.
  expect(afterImprint.objectCount).toBe(0)
  expect(afterImprint.members).toEqual(members0)

  // ---- 3. Push/Pull the new sub-face outward (push_pull_in_component) ---
  // A click on the sub-face's center arms the drag; a typed distance with
  // no drag commits OUTWARD along the face normal (front face normal −Y),
  // extending the member's bounding box past its original y=0 boundary.
  await page.keyboard.press('p')
  await clickWorld(page, ctx, 1.0, 0, 0.5)
  await page.keyboard.type('0.15')
  await expect(page.getByText('Push depth')).toBeVisible()
  await page.keyboard.press('Enter')

  const afterPushPull = await page.evaluate(
    (id) => ({
      lastError: window.__hew_test!.getLastError(),
      bounds: window.__hew_test!.getObjectBounds(id),
    }),
    memberId,
  )
  expect(afterPushPull.lastError).toBeNull()
  // minY moved from 0 to roughly −0.15 — the boss protrudes past the
  // original front face, on the SAME shared member both instances place.
  expect(afterPushPull.bounds[1]).toBeLessThan(-0.1)

  // ---- 4. Idle-lock a plane and draw in EMPTY space — THE axis-lock
  // symptom. ArrowRight locks the future plane's normal to X; the two click
  // points (x=3) sit well clear of the box, in empty space still inside the
  // instance's editing context.
  const sketchesBeforeLockDraw = await page.evaluate(
    (c) => window.__hew_test!.getComponentMemberSketches(c),
    component,
  )
  expect(sketchesBeforeLockDraw).toHaveLength(0)

  await page.keyboard.press('r')
  await page.keyboard.press('ArrowRight')
  // The FIRST click of an idle-locked gesture has no constraint plane yet
  // (`snapConstraint` only returns one once something is anchored) — it
  // resolves through the ordinary ground-plane snap, so it must actually
  // SIT on the ground (z=0) for the ray to land there at all. THAT click
  // is what anchors the locked plane (x=3) through it; the second click is
  // then constrained to that real plane and can be off the ground.
  await clickWorld(page, ctx, 3, 0.3, 0)
  await clickWorld(page, ctx, 3, 0.8, 0.7)

  const afterLockDraw = await page.evaluate(
    (c) => ({
      lastError: window.__hew_test!.getLastError(),
      // `getSketchIds` (`Document::sketch_ids`) deliberately excludes
      // definition-owned sketches — the world-tree analog of
      // `getObjectCount` excluding definition members — so a WORLD sketch
      // (the symptom this step exists to catch) would show up here, but a
      // correctly-routed def-owned one never will.
      worldSketchIds: window.__hew_test!.getSketchIds(),
      memberSketches: window.__hew_test!.getComponentMemberSketches(c),
    }),
    component,
  )
  expect(afterLockDraw.lastError).toBeNull()
  // The lock-drawn rectangle landed in THIS component's own definition —
  // never a world sketch (`begin_ground_sketch`/`begin_sketch_on_plane`),
  // which is what the symptom produced.
  expect(afterLockDraw.worldSketchIds).toHaveLength(0)
  expect(afterLockDraw.memberSketches).toHaveLength(1)

  // ---- 5. A small standing profile, drawn as its OWN def-owned sketch on
  // an X-normal plane through x=1 — squarely inside the box's own x∈[0,2]
  // span, so the TOP face's loop path genuinely crosses the profile's plane
  // (`PathDetachedFromProfile` otherwise), straddling the loop's z=1 rim
  // (from the ground up to just above it) just outside the box's y=0 edge.
  // Same idle-lock gesture as step 4, anchored at a different x.
  await page.keyboard.press('r')
  await page.keyboard.press('ArrowRight')
  await clickWorld(page, ctx, 1, -0.3, 0)
  await clickWorld(page, ctx, 1, -0.05, 1.15)

  const afterProfileDraw = await page.evaluate(
    (c) => ({
      lastError: window.__hew_test!.getLastError(),
      memberSketches: window.__hew_test!.getComponentMemberSketches(c),
    }),
    component,
  )
  expect(afterProfileDraw.lastError).toBeNull()
  expect(afterProfileDraw.memberSketches).toHaveLength(2)

  // ---- 6. Follow Me: that profile swept around the TOP face's loop path
  // (a plain sketch-region profile around another member's face-loop path —
  // `follow_me_around_face_in_instance` — always births a SEPARATE member;
  // only a solid-FACE profile on the SAME object auto-merges). Before this
  // phase Follow Me refused every face interaction inside a component's
  // definition wholesale.
  const hashBeforeFollowMe = await page.evaluate(() => window.__hew_test!.getStateHash())
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  await expect(page.getByText('Click the path to follow')).toBeVisible()
  // Path: click the top face (z=1) — a face-loop path around its boundary.
  await clickWorld(page, ctx, 1.6, 0.5, 1)
  // Profile: the sketch region just drawn.
  await clickWorld(page, ctx, 1, -0.175, 0.6)

  const afterFollowMe = await page.evaluate(
    (c) => ({
      lastError: window.__hew_test!.getLastError(),
      members: window.__hew_test!.getComponentMemberObjects(c),
      instances: window.__hew_test!.getInstancesOf(c),
      stateHash: window.__hew_test!.getStateHash(),
    }),
    component,
  )
  expect(afterFollowMe.lastError).toBeNull()
  // A SECOND member is born (the swept molding) — the original member is
  // untouched (non-merging: profile and path are different kinds of pick) —
  // and BOTH instances still place the SAME (now two-member) definition.
  // The state hash moving is the general proof real geometry landed.
  expect(afterFollowMe.members).toHaveLength(2)
  expect(new Set(afterFollowMe.instances)).toEqual(new Set([setup.instance, setup.instanceB]))
  expect(afterFollowMe.stateHash).not.toBe(hashBeforeFollowMe)

  // ---- 7. Undo the whole chain: Follow Me → profile-sketch draw →
  // lock-draw → push/pull → front-face imprint. The definition (and so
  // BOTH instances) returns to its starting single-member shape. A drawing
  // gesture's multi-segment commit can bracket into more than one undo step
  // (`Document`'s own granularity, not this test's business), so rather than
  // pin a literal undo count, undo in a loop until the document's
  // `state_hash` is back to EXACTLY what it was before entering the context
  // — the least ambiguous ground truth for "is this really the same state" —
  // bounded so a genuine regression fails loudly instead of hanging.
  let undoIterations = 0
  let currentHash = await page.evaluate(() => window.__hew_test!.getStateHash())
  while (currentHash !== hashBeforeContext) {
    if (undoIterations >= 30 || !(await page.evaluate(() => window.__hew_test!.canUndo()))) break
    await page.evaluate(() => window.__hew_test!.undo())
    undoIterations++
    currentHash = await page.evaluate(() => window.__hew_test!.getStateHash())
  }
  expect(currentHash).toBe(hashBeforeContext)

  // The KERNEL'S own `def_members`/`def_member_sketches` list a definition's
  // members the same way a Group lists ITS members: hiding one (what
  // undoing a creation does — never a true delete, so redo can bring it
  // straight back) leaves its id and listing in place, only its visibility
  // changes (`Document::commit_region_object_owned`'s own doc says so
  // explicitly) — a tombstone, not a removal. The wasm boundary
  // (`Scene::component_member_objects`/`component_member_sketches`) already
  // filters that kernel-side list through `Document::object`/`Document::sketch`
  // before it ever reaches JS, so `getComponentMemberObjects` here already
  // answers live-only — the `isObjectSolid` (`Document::object_solid`:
  // live/visible AND watertight) filter below is a redundant, defensive
  // re-check, not load-bearing. Watertightness is along for the ride rather
  // than the point of this particular filter — every surviving member in
  // this spec is a plain watertight box, and the exact state_hash equality
  // above already fully constrains the state — but a genuinely live,
  // currently-non-watertight member (mid-boolean, say) would be wrongly
  // excluded by this same filter if this pattern is reused for less trivial
  // geometry without that redundant exact-hash backstop.
  const afterUndo = await page.evaluate(
    (c) => ({
      members: window.__hew_test!.getComponentMemberObjects(c),
      instances: window.__hew_test!.getInstancesOf(c),
    }),
    component,
  )
  const liveMembers: string[] = []
  for (const id of afterUndo.members) {
    if (await page.evaluate((oid) => window.__hew_test!.isObjectSolid(oid), id)) liveMembers.push(id)
  }
  expect(liveMembers).toEqual(members0)
  expect(new Set(afterUndo.instances)).toEqual(new Set([setup.instance, setup.instanceB]))
  const boundsAfterUndo = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    memberId,
  )
  for (let i = 0; i < 6; i++) {
    expect(boundsAfterUndo[i]).toBeCloseTo(boundsBefore[i], 6)
  }

  // ---- 8. Exit the context (Escape) and prove a draw AFTERWARD lands in
  // the world, not the definition — the mirror-image check of step 4.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(120)
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0.5, -2, 0)
  await clickWorld(page, ctx, 1, -1.5, 0)

  const afterExitDraw = await page.evaluate(
    (c) => ({
      lastError: window.__hew_test!.getLastError(),
      allSketchIds: window.__hew_test!.getSketchIds(),
      memberSketches: window.__hew_test!.getComponentMemberSketches(c),
    }),
    component,
  )
  expect(afterExitDraw.lastError).toBeNull()
  expect(afterExitDraw.allSketchIds).toHaveLength(1)
  // The post-exit sketch is a WORLD sketch — never listed as a member of the
  // definition. `getComponentMemberSketches` is already filtered to LIVE
  // members (see the step 7 comment above: the kernel's own list keeps this
  // session's now-hidden def-owned sketches tombstoned, but the wasm
  // boundary filters them out before this call ever sees them) — so an
  // empty-looking list here would be equally consistent with "the filter
  // dropped them" and "the definition legitimately has none live". The
  // precise check is therefore that the NEW (world) sketch specifically
  // isn't among them, not that the list is empty.
  expect(afterExitDraw.memberSketches).not.toContain(afterExitDraw.allSketchIds[0])
})

test('component-edit parity: a real on-face circle pushes through a component box as a hole', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    h.placeInstance(component, 5, 0, 0)
    return { instance, component }
  })
  const before = await page.locator('canvas').first().screenshot()
  const source = (await page.evaluate(
    (component) => window.__hew_test!.getComponentMemberObjects(component),
    setup.component,
  ))[0]

  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.keyboard.press('c')
  await clickWorld(page, ctx, 1, 0.5, 1)
  await clickWorld(page, ctx, 1.25, 0.5, 1)

  await page.keyboard.press('p')
  await clickWorld(page, ctx, 1, 0.5, 1)
  await page.keyboard.type('-1.5')
  await expect(page.getByText('Push depth')).toBeVisible()
  await page.keyboard.press('Enter')

  await expect.poll(async () => page.evaluate(
    ({ component, source }) =>
      !window.__hew_test!.getComponentMemberObjects(component).includes(source),
    { component: setup.component, source },
  )).toBe(true)
  const afterState = await page.evaluate(
    (component) => ({
      members: window.__hew_test!.getComponentMemberObjects(component),
      instances: window.__hew_test!.getInstancesOf(component),
    }),
    setup.component,
  )
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  expect(afterState.members).toHaveLength(1)
  expect(afterState.instances).toHaveLength(2)
  const after = await page.locator('canvas').first().screenshot()
  expect(after.equals(before)).toBe(false)
})

test('component-edit parity: selected members rotate and scale through real tool gestures', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    return h.makeComponent([box])
  })
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.keyboard.press('Space')
  await clickWorld(page, ctx, 1, 0.5, 1)
  await expect.poll(async () => page.evaluate(
    () => window.__hew_test!.getSelection().map((n) => n.kind),
  )).toEqual(['object'])

  const beforeRotate = await page.evaluate(() => window.__hew_test!.getStateHash())
  await page.keyboard.press('q')
  await page.keyboard.press('ArrowUp')
  await clickWorld(page, ctx, 1, 0.5, 1)
  await clickWorld(page, ctx, 2, 0.5, 1)
  await page.keyboard.type('30')
  await page.keyboard.press('Enter')
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getStateHash()))
    .not.toBe(beforeRotate)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  await expect.poll(async () => page.evaluate(
    () => window.__hew_test!.getSelection().map((n) => n.kind),
  )).toEqual(['object'])

  const beforeScale = await page.evaluate(() => window.__hew_test!.getStateHash())
  await page.keyboard.press('s')
  await page.waitForTimeout(120)
  await clickWorld(page, ctx, 1, 0.5, 1)
  await clickWorld(page, ctx, 1, 0.5, 1.5)
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getStateHash()))
    .not.toBe(beforeScale)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  expect(await page.evaluate(
    (component) => window.__hew_test!.getComponentMemberObjects(component).length,
    setup.component,
  )).toBe(1)
})

test('component-edit parity: Shift-selected members boolean through the Edit menu', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const component = await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1.2, 1, 0], 1)
    const b = h.drawBox([0.8, 0, 0], [2, 1, 0], 1)
    return h.makeComponent([a, b]).component
  })
  await dblClickWorld(page, ctx, 0.4, 0.5, 1)
  await page.keyboard.press('Space')
  await clickWorld(page, ctx, 0.4, 0.5, 1)
  await page.keyboard.down('Shift')
  await clickWorld(page, ctx, 1.6, 0.5, 1)
  await page.keyboard.up('Shift')
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getSelection().length))
    .toBe(2)

  await page.getByRole('button', { name: 'Edit' }).click()
  await page.getByText('Union', { exact: true }).click()
  await expect.poll(async () => page.evaluate(
    (id) => window.__hew_test!.getComponentMemberObjects(id).length,
    component,
  )).toBe(1)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
})

test('component-edit parity: definition-sketch inference feeds selection, Rotate, and Scale', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    h.makeComponent([box])
  })
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 3, 0, 0)
  await clickWorld(page, ctx, 4, 1, 0)

  await page.keyboard.press('Space')
  const center = px(ctx, 3.5, 0.5, 0)
  await page.mouse.move(center.x, center.y)
  await expect(page.getByText(/On Face|Endpoint|Midpoint|On Edge/)).toBeVisible()
  await clickWorld(page, ctx, 3.5, 0.5, 0)
  await expect.poll(async () => page.evaluate(
    () => window.__hew_test!.getSelection().map((n) => n.kind),
  )).toEqual(['sketch-island'])

  const beforeScale = await page.evaluate(() => window.__hew_test!.getStateHash())
  await page.keyboard.press('s')
  await page.waitForTimeout(120)
  await clickWorld(page, ctx, 4, 1, 0)
  await page.keyboard.type('1.2')
  await page.keyboard.press('Enter')
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getStateHash()))
    .not.toBe(beforeScale)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  const beforeRotate = await page.evaluate(() => window.__hew_test!.getStateHash())
  await page.keyboard.press('q')
  await page.keyboard.press('ArrowUp')
  await clickWorld(page, ctx, 3.5, 0.5, 0)
  await clickWorld(page, ctx, 4, 0.5, 0)
  await page.keyboard.type('30')
  await page.keyboard.press('Enter')
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getStateHash()))
    .not.toBe(beforeRotate)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  await expect.poll(async () => page.evaluate(
    () => window.__hew_test!.getSelection().map((n) => n.kind),
  )).toEqual(['sketch-island'])
})
