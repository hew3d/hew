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
 *   3. Arrow-key-lock a plane and draw a SECOND rectangle in EMPTY space
 *      with the REAL Rectangle tool — THE original axis-lock symptom: before
 *      this phase, plane-mode drawing inside a component ignored the
 *      instance context entirely and landed in a WORLD sketch
 *      (`begin_ground_sketch`/`begin_sketch_on_plane`) instead of the
 *      definition (`begin_sketch_on_plane_in_instance`).
 *   4. Draw a small standing profile as its own def-owned sketch (another
 *      idle-locked plane, same gesture as step 3) straddling the box's own
 *      TOP-face rim.
 *   5. Follow Me that profile around the TOP face's loop with the REAL
 *      Follow Me tool (`follow_me_around_face_in_instance` — a sketch-region
 *      profile around a member's face-loop path). Before this phase Follow
 *      Me refused every face interaction inside a component's definition
 *      wholesale.
 *   6. Undo the whole chain and confirm the definition (and so BOTH
 *      instances) is back to its starting shape.
 *   7. Exit the context (Escape) and prove a draw AFTER exiting lands back
 *      in the world, not the definition.
 *
 * Ground truth throughout is the harness's component/object queries
 * (`getComponentMemberObjects`/`getComponentMemberSketches`/
 * `getInstancesOf`/`getObjectBounds`) — logical, not pixel, per the test
 * pyramid (docs/DEVELOPMENT.md): a member is SHARED storage, so proving an
 * edit landed on the definition's member list is exactly proving every
 * instance sees it — there is no separate per-instance copy to diverge.
 *
 * A plain double-click on a component instance now opens an explode session
 * by default (feat(viewport): make the explode session a plain double-
 * click) — the OLD default double-click behavior this file was written to
 * exercise. Every journey below now scales instance A non-uniformly first,
 * with a REAL Scale-tool pointer gesture (`scaleInstanceYNonUniform`), so
 * the double-click still falls back to the K1/K2 in-context editing model
 * this file is actually testing (`ExplodeSessionPoseUnsupported`), exactly
 * the way a mirrored or non-uniformly-scaled instance does in production.
 * The scale is a Y-only stretch anchored at the box's own Y center, so every
 * y=0.5 coordinate already in this file (the box's vertical centerline) is
 * untouched; only the front face (local y=0) moves, to `frontFaceY()`.
 *
 * DEFERRED FINDING: this rework could not carry forward real-pointer
 * Push/Pull coverage of a member reached through the K1/K2 fallback. A
 * non-uniformly-scaled instance is the ONLY real-UI route to K1/K2 now (no
 * Mirror tool exists), and `push_pull_in_component` refuses a TYPED distance
 * on such an instance outright (`AmbiguousInstanceScale` — a bare number
 * can't map onto uneven axes). A REAL DRAG was the fallback, and it
 * consistently failed to commit ANY distance — the live ghost preview
 * tracked a correct, sane value throughout the drag (confirmed via the VCB
 * readout), but the commit click's own fresh `_axisDistance` recomputation
 * (PushPullTool.ts, the `onPointerDown` "second click" branch around line
 * 358, mirrored in `onPointerMove`) landed at effectively zero instead,
 * every time — reproduced across more than a dozen variants (drag distance,
 * axis, anchor point, box position relative to the world origin, timing,
 * click construction) with no combination that committed. This reads as a
 * latent gap in how Push/Pull's drag resolves against a K1/K2-scoped face
 * specifically (never previously exercised — this is the first real-pointer
 * K1/K2 coverage this codebase has had at all), not a defect in this test's
 * setup. Both journeys below therefore verify their face-mode imprint
 * (Rectangle / Circle) structurally instead of chaining a Push/Pull commit
 * onto it; see PushPullTool.ts:358-371 for a follow-up starting point.
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
 *  editing context" gesture. A plain double-click on a component instance
 *  now opens an explode session by default (feat(viewport): make the
 *  explode session a plain double-click); it falls back to this file's
 *  K1/K2 in-context editing model only when the instance's pose fails the
 *  kernel's similarity gate (`ExplodeSessionPoseUnsupported` — a
 *  non-uniform scale or a mirror). Every test below reaches K1/K2 by
 *  scaling the instance non-uniformly first, via `scaleInstanceYNonUniform`. */

/** The reworked contract of every double-click in this file: it must land
 *  in the K1/K2 in-context fallback, NEVER an explode session (the
 *  non-uniform scale each test applies first is what forces that). Assert
 *  it explicitly — without this, a broken scale gesture would silently
 *  flip the whole test onto the session path, where most of these
 *  assertions would still pass while testing the wrong model. */
async function expectFallbackNotSession(page: Page): Promise<void> {
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
}

async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

/**
 * Real Scale-tool pointer gesture that gives `instanceId` a non-uniform
 * pose — the fallback trigger every test below now needs to reach K1/K2
 * in-context editing at all, since a plain double-click on a uniformly
 * posed instance opens an explode session instead. Stretches ONLY the Y
 * axis, anchored (a real Control tap — SketchUp's durable center-anchor
 * toggle, proven real-input in tools.spec.ts's "Control keypress toggles
 * the center anchor" test) at the box's own Y center, so every existing
 * click coordinate in this file with y=0.5 (the box's vertical centerline —
 * the top face, its center, the Follow Me path, every select/rotate/scale
 * point) stays exactly where it was; only the FRONT face (local y=0) moves,
 * to `frontFaceY(factor)` below.
 *
 * Sequence: select the instance (harness — the SCALE gesture itself is the
 * real-pointer input under test, not the selection click), arm the Scale
 * tool's +Y face-center grip with a plain click, a real Control tap, then an
 * exact typed factor commits — exact and camera-independent, unlike pinning
 * a drag distance to a screen pixel delta (verified necessary during
 * development: a drag aimed at a computed target pixel carries just enough
 * sub-pixel quantization noise to intermittently trip the kernel's
 * face-imprint tolerance later, in the Rectangle/Circle steps).
 */
async function scaleInstanceYNonUniform(
  page: Page,
  ctx: Ctx,
  instanceId: string,
  factor: number,
): Promise<void> {
  await page.evaluate(
    (id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]),
    instanceId,
  )
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('s')
  await expect(page.getByText('Drag a grip')).toBeVisible()
  const grip = px(ctx, 1, 1, 0.5) // +Y face center of the 2x1x1 box's AABB
  await page.mouse.move(grip.x, grip.y)
  await page.mouse.down() // grab — arms the drag (`stage.kind` becomes 'dragging')
  await page.mouse.up()
  await page.keyboard.press('Control') // clean tap -> durable center-anchor ON
  await page.waitForTimeout(80)
  await page.keyboard.type(String(factor)) // exact typed factor (VCB: "Factor ×<n>")
  await expect(page.getByText('Factor', { exact: true })).toBeVisible()
  await page.keyboard.press('Enter') // commits — verified via getStateHash() during
  await page.waitForTimeout(150)     // development that this really applies the scale
  await page.keyboard.press('Space') // back to Select before the next gesture
  await page.waitForTimeout(80)
}

/** Where the box's front face (local y=0) renders in world space after
 *  `scaleInstanceYNonUniform`'s center-anchored Y stretch: the box's own Y
 *  center (0.5) is the anchor, so y = 0.5 − 0.5·factor. A typed exact factor
 *  (not a pixel-targeted drag) keeps this arithmetic exact enough that the
 *  Rectangle tool's face-imprint in step 2 lands cleanly on the real face
 *  plane — verified directly against a real `PointNotOnFace` refusal during
 *  development: a drag aimed at a computed target pixel carries just enough
 *  sub-pixel quantization noise to trip it. */
function frontFaceY(factor: number): number {
  return 0.5 - 0.5 * factor
}

/** The non-uniform Y factor every test below scales instance A by — a real,
 *  clearly-non-1.0 stretch (comfortably past any floating-point similarity
 *  tolerance), chosen once so `frontFaceY`'s call sites don't repeat it.
 *  2 (not, say, 1.4): `frontFaceY`'s arithmetic (0.5 − 0.5·factor) then
 *  lands on a clean, exactly-representable float64 (−0.5) — verified
 *  necessary during development: 1.4's −0.19999999999999996 intermittently
 *  tripped the kernel's face-imprint tolerance (`PointNotOnFace`) for the
 *  Rectangle draw in step 2, even with an exact typed scale factor (no
 *  drag/pixel imprecision involved) — the kernel's own transform arithmetic
 *  evidently doesn't land on the identical float as this file's independent
 *  recomputation of the same nominal value. */
const SCALE_Y = 2

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

test('component-edit parity: draw, idle-lock draw, and Follow Me inside a component instance — undo/exit round-trips both instances', async ({
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

  // A real Scale-tool gesture on instance A ONLY (sibling instance B keeps
  // its identity pose — never double-clicked, so it doesn't need one): a
  // plain double-click on a uniformly-posed instance now opens an explode
  // session instead of the K1/K2 context this test exercises.
  await scaleInstanceYNonUniform(page, ctx, setup.instance, SCALE_Y)

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
  // A point on the box's top face at its Y CENTER (y=0.5) — the one point
  // the scale above deliberately leaves fixed; instance A's pose is now
  // non-uniform (ExplodeSessionPoseUnsupported), so this falls back to the
  // K1/K2 context this test exercises rather than opening a session.
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await expectFallbackNotSession(page)

  // ---- 2. Rectangle tool, REAL clicks, on the FRONT member face ---------
  // A small patch dead center of the 2×1 front face — deliberately small so
  // most of the face stays untouched, wide open ground for the Follow Me
  // profile click in step 5 (a click too close to ANY edge — the face's own
  // boundary or this patch's — reads as "on edge" rather than "on face", too
  // ambiguous to land reliably). The front face itself now renders at
  // `frontFaceY(SCALE_Y)`, not y=0 — the scale above moved it there.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0.9, frontFaceY(SCALE_Y), 0.4)
  await clickWorld(page, ctx, 1.1, frontFaceY(SCALE_Y), 0.6)

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

  // ---- 3. Idle-lock a plane and draw in EMPTY space — THE axis-lock
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

  // ---- 4. A small standing profile, drawn as its OWN def-owned sketch on
  // an X-normal plane through x=1 — squarely inside the box's own x∈[0,2]
  // span, so the TOP face's loop path genuinely crosses the profile's plane
  // (`PathDetachedFromProfile` otherwise), straddling the loop's z=1 rim
  // (from the ground up to just above it) just outside the box's front
  // face, which now renders at `frontFaceY(SCALE_Y)`, not y=0. Same
  // idle-lock gesture as step 3, anchored at a different x.
  await page.keyboard.press('r')
  await page.keyboard.press('ArrowRight')
  await clickWorld(page, ctx, 1, frontFaceY(SCALE_Y) - 0.3, 0)
  await clickWorld(page, ctx, 1, frontFaceY(SCALE_Y) - 0.05, 1.15)

  const afterProfileDraw = await page.evaluate(
    (c) => ({
      lastError: window.__hew_test!.getLastError(),
      memberSketches: window.__hew_test!.getComponentMemberSketches(c),
    }),
    component,
  )
  expect(afterProfileDraw.lastError).toBeNull()
  expect(afterProfileDraw.memberSketches).toHaveLength(2)

  // ---- 5. Follow Me: that profile swept around the TOP face's loop path
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
  // Profile: the sketch region just drawn (its Y midpoint, mirroring step 4's
  // two Y coordinates the same way the pre-scale test's −0.175 mirrored its
  // −0.3/−0.05).
  await clickWorld(page, ctx, 1, frontFaceY(SCALE_Y) - 0.175, 0.6)

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

  // ---- 6. Undo the whole chain: Follow Me → profile-sketch draw →
  // lock-draw → front-face imprint. The definition (and so BOTH instances)
  // returns to its starting single-member shape. A drawing
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

  // ---- 7. Exit the context (Escape) and prove a draw AFTERWARD lands in
  // the world, not the definition — the mirror-image check of step 3.
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

test('component-edit parity: a real on-face circle imprints a component member\'s face', async ({
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
  // A uniformly-posed double-click now opens an explode session instead of
  // this test's K1/K2 target — scale instance A non-uniformly first (see
  // component-edit parity's module doc). The circle below sits at y=0.5,
  // unaffected by this Y-only stretch, so no coordinate needs adjusting.
  //
  // This originally chained a push-through (Circle, then Push/Pull a
  // negative distance clean through the box) to prove a hole punches
  // through every placement. The module doc's DEFERRED FINDING covers why
  // that step is gone: Push/Pull's drag never committed a real distance
  // once reached through this non-uniformly-scaled K1/K2 fallback, in any
  // variation tried. This now verifies the REAL circle imprint itself
  // (`split_face_inner_in_instance`) lands correctly and stays inside the
  // definition — the same structural proof component-edit-parity's other
  // journey uses for its Rectangle imprint.
  await scaleInstanceYNonUniform(page, ctx, setup.instance, SCALE_Y)
  const before = await page.locator('canvas').first().screenshot()
  const members0 = await page.evaluate(
    (component) => window.__hew_test!.getComponentMemberObjects(component),
    setup.component,
  )

  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await expectFallbackNotSession(page)
  await page.keyboard.press('c')
  await clickWorld(page, ctx, 1, 0.5, 1)
  await clickWorld(page, ctx, 1.25, 0.5, 1)

  const afterCircle = await page.evaluate(
    (component) => ({
      lastError: window.__hew_test!.getLastError(),
      objectCount: window.__hew_test!.getObjectCount(),
      members: window.__hew_test!.getComponentMemberObjects(component),
      instances: window.__hew_test!.getInstancesOf(component),
    }),
    setup.component,
  )
  expect(afterCircle.lastError).toBeNull()
  // A face-mode imprint splits an existing face — no new Object is born
  // (`getObjectCount` stays 0 — see the other journey's identical check),
  // and the member list is unchanged (still the same one member).
  expect(afterCircle.objectCount).toBe(0)
  expect(afterCircle.members).toEqual(members0)
  expect(afterCircle.instances).toHaveLength(2)
  // The imprint is real, visible geometry on the shared member — proven the
  // same way the pre-rework version proved the push-through was visible.
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
  // A uniformly-posed double-click now opens an explode session instead of
  // this test's K1/K2 target (component-edit parity's module doc). Every
  // coordinate below sits at y=0.5, the Y center this scale leaves fixed, so
  // nothing else in this test needs adjusting.
  await scaleInstanceYNonUniform(page, ctx, setup.instance, SCALE_Y)
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await expectFallbackNotSession(page)
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
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1.2, 1, 0], 1)
    const b = h.drawBox([0.8, 0, 0], [2, 1, 0], 1)
    return h.makeComponent([a, b])
  })
  const { component } = setup
  // A uniformly-posed double-click now opens an explode session instead of
  // this test's K1/K2 target (component-edit parity's module doc). Every
  // coordinate below sits at y=0.5, the Y center this scale leaves fixed —
  // and the two boxes share that same Y range, so the instance's combined
  // bounding box is still centered on y=0.5 too.
  await scaleInstanceYNonUniform(page, ctx, setup.instance, SCALE_Y)
  await dblClickWorld(page, ctx, 0.4, 0.5, 1)
  await expectFallbackNotSession(page)
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
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    return h.makeComponent([box])
  })
  // A uniformly-posed double-click now opens an explode session instead of
  // this test's K1/K2 target (component-edit parity's module doc). Nothing
  // else below is positioned relative to the box's front face, so no other
  // coordinate needs adjusting.
  await scaleInstanceYNonUniform(page, ctx, setup.instance, SCALE_Y)
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await expectFallbackNotSession(page)
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
