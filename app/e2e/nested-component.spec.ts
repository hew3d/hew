import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Nested components — end to end with REAL pointer input (feat(kernel):
 * edit nested components in place; docs/design/nested-components.md). A
 * component definition can now contain instances of OTHER definitions, and
 * a session opened on one stacks LIFO: double-clicking a member instance
 * surfaced by an open session opens its OWN session on top, Escape pops one
 * frame at a time. This is the maintainer's acceptance case for the
 * feature — the kernel's own `sessions_stack_for_drill_down` spec
 * (crates/kernel/tests/nested_component_specs.rs) proves the same shape
 * headless; this file proves it through the real browser app.
 *
 * Setup builds the nested assembly the same way a person would
 * interactively (the commit's own words: "the interactive route to
 * building assemblies") — the app doesn't yet gate "Make Component" open
 * for a mixed object+instance selection (`canMakeComponent` in
 * treeModel.ts still says "no instances (nested defs are deferred)", a
 * comment this feature has outrun), so the real route today is: open a
 * session on the assembly (a real double-click) and place an instance of
 * the part definition while it's open (`placeInstance`, the same harness
 * call every other component-edit spec already uses for sibling
 * placements) — it folds in as a nested member when the session closes.
 *
 * The journey:
 *   1. Build: a part component (one box) and an assembly component (a
 *      second, unrelated box) — then fold a placed part instance into the
 *      assembly as a nested member via the route above. A second,
 *      untouched placement of the part definition stays outside the
 *      assembly the whole time — the "does every placement see the edit"
 *      witness.
 *   2. Double-click the assembly open (real gesture) — session depth 1.
 *      The nested part instance surfaces to world ownership at its
 *      composed pose (same InstanceId — a move, not a copy), which is what
 *      makes it double-clickable in the next step.
 *   3. Double-click that surfaced nested instance — a SECOND, stacked
 *      session opens on top; depth 2.
 *   4. Push/Pull a real drag on the inner part's member face — a visible
 *      edit landing on the shared definition.
 *   5. Escape once: back to depth 1 (still editing the assembly, not
 *      closed). Escape again: depth 0, nothing open.
 *   6. The edit persisted on the SAME member object both placements share
 *      — there is no separate per-instance copy to diverge (the standalone
 *      placement was never clicked, so its geometry changing too is the
 *      whole point of a shared definition).
 *
 * Depth is read the same way group-session.spec.ts's own stacked-session
 * journey does: the Outliner's "editing" chip (DocumentTree.tsx) renders on
 * every node on the combined session-stack path, so its count on the page
 * IS the stack depth — the same mechanism this file relies on already
 * covers group sessions; nothing here is component-specific to it.
 * `getExplodeSessionInstance()` cross-checks the identity of the innermost
 * frame specifically (`Document::component_session` reads only
 * `sessions.last()`), including the load-bearing claim that drilling in
 * lands on the EXACT SAME InstanceId that was folded in during setup.
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

/** A real click-drag-click through a two-click tool gesture (Push/Pull) —
 *  explode-session.spec.ts's own proven Push/Pull drag pattern. Reliable
 *  here for the same reason it is there: a session member is a plain world
 *  object for the edit's duration, no instance pose involved. */
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

/** A real double-click at a world point — opens a session (or drills into
 *  one already open) on whatever it resolves to at the current context
 *  depth, matching explode-session.spec.ts/group-session.spec.ts's own
 *  helper of the same name. */
async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

/** The Outliner's "editing" chip count — one per node on the combined
 *  session-stack path (DocumentTree.tsx's `NodeRow`/`Row`), so it doubles
 *  as the session stack depth. Exact match: a substring match would also
 *  catch the viewport's "Editing <name>" status overlay (see
 *  group-session.spec.ts's own identical comment). */
async function editingChipCount(page: Page): Promise<number> {
  return page.getByText('editing', { exact: true }).count()
}

// A 3/4 view wide enough to cover the part's standalone placement
// (x:[0,1] y:[0,1]), the assembly's own box (x:[3,4] y:[0,1]), and the
// nested part placement once surfaced (x:[3,4] y:[3,4]) — all at z:[0,1].
const CAMERA: CameraParams = {
  position: { x: 11, y: -11, z: 10 },
  target: { x: 2.5, y: 2, z: 0.6 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 60,
  near: 0.1,
  far: 1000,
}

test('nested component: drill into a sub-component, edit it, step back out', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // ---- setup: a part component (one box) and an assembly component (a
  // second, unrelated box) — see the module doc for why this route (open,
  // placeInstance, close) rather than a Make-Component-with-mixed-selection
  // gesture.
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const partBox = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance: partInstance, component: partComponent } = h.makeComponent([partBox])
    const assemblyBox = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    const { instance: assemblyInstance, component: assemblyComponent } = h.makeComponent([assemblyBox])
    const partMember = h.getComponentMemberObjects(partComponent)[0]
    return { partInstance, partComponent, assemblyInstance, assemblyComponent, partMember }
  })
  const { partComponent, assemblyInstance, partMember } = setup

  const boundsPristine = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    partMember,
  )

  // ---- fold a placed part instance into the assembly as a nested member:
  // open the assembly's session (real double-click on its top face — a
  // uniformly-posed fresh instance, so this is the plain session-open path,
  // not the K1/K2 fallback), place a part instance while it's open, close.
  await dblClickWorld(page, ctx, 3.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(assemblyInstance)
  const nestedPartInstance = await page.evaluate(
    (c) => window.__hew_test!.placeInstance(c, 3, 3, 0),
    partComponent,
  )
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()

  // ---- 1/2. Double-click the assembly open (the acceptance case's real
  // entry point) — depth 1. The nested member instance surfaces to world
  // ownership at its composed pose (same InstanceId — a move, not a copy),
  // which is what makes it independently double-clickable next.
  await dblClickWorld(page, ctx, 3.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(assemblyInstance)
  expect(await editingChipCount(page)).toBe(1)
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), partComponent)))
    .toEqual(new Set([setup.partInstance, nestedPartInstance]))

  // ---- 3. Double-click the surfaced nested instance — a SECOND, stacked
  // session opens on top of the assembly's; depth 2. It lands on the EXACT
  // SAME InstanceId that was folded in during setup — proof the drill-down
  // reached the nested member, not some other node that happens to render
  // at that spot.
  await dblClickWorld(page, ctx, 3.5, 3.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(nestedPartInstance)
  expect(await editingChipCount(page)).toBe(2)

  // ---- 4. A real Push/Pull drag on the inner part's own member face — the
  // member is a plain world object for this innermost session's duration,
  // the same reason explode-session.spec.ts's drags are reliable.
  const boundsBeforePush = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    partMember,
  )
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [3.5, 3.5, 1], [3.5, 3.5, 2.5])
  const boundsAfterPush = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    partMember,
  )
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  expect(boundsAfterPush[5]).toBeGreaterThan(boundsBeforePush[5] + 1) // maxZ grew well past the original 1

  // ---- 5. Escape once: back to depth 1 (still editing the assembly, not
  // closed out entirely) — matching group-session.spec.ts's own two-Escape
  // layering check.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(assemblyInstance)
  expect(await editingChipCount(page)).toBe(1)

  // Escape again: depth 0, nothing open.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  expect(await editingChipCount(page)).toBe(0)

  // ---- 6. The edit persisted on the SHARED member object both placements
  // reference — there is no separate per-instance copy to diverge
  // (component-edit.spec.ts's own doc makes the same point about a flat
  // definition's members; it holds identically for a nested one). Asserting
  // the member's bounds directly IS asserting every placement — the
  // standalone instance and the assembly's nested member alike — shows the
  // taller box, since both point at this exact object. `getInstancesOf`
  // (`instances_of`) lists every non-hidden instance of the definition
  // regardless of nesting depth — folding into the assembly changes the
  // nested instance's OWNER, not its hidden flag — so both placements are
  // still listed here, exactly as before the fold; nesting is invisible to
  // this query by design (`Document::instances_of`'s own doc: "the visible
  // instances that place `component`").
  const boundsFinal = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    partMember,
  )
  expect(boundsFinal[5]).toBeGreaterThan(boundsPristine[5] + 1)
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), partComponent)))
    .toEqual(new Set([setup.partInstance, nestedPartInstance]))
})

/**
 * The playtest bug this fixes: while editing a component, its member
 * GROUPS surface as ordinary world groups — double-clicking one now opens a
 * stacked GROUP session on top of the component frame (it used to refuse
 * with "Another component is already open for editing"). This is the shape
 * an imported SketchUp assembly takes (the maintainer hit it on a real
 * model whose component's members are groups of solids). The kernel spec
 * `group_sessions_open_inside_a_component`
 * (crates/kernel/tests/nested_component_specs.rs) proves the mechanics
 * headless with the exact same two boxes and coordinates this test uses;
 * this proves the APP path — the last three defects in this feature were
 * all app-layer reachability problems the kernel specs could not see.
 *
 * Setup: the harness's `makeComponent` hardcodes kind 0 (object) for every
 * id it's given (see its own implementation in test/harness.ts) — it
 * cannot express a group in the selection. The real app doesn't share that
 * limitation: `canMakeComponent`/`runMakeComponent` (treeModel.ts/App.tsx)
 * already accept a mixed group+object selection and forward it through
 * `structuralSelection`'s real kind tags. So the real route — and the one
 * this test drives — is a real gesture: select the group and the other box
 * via the harness's `selectNodes` (the same selection-state entry point a
 * click/marquee uses), then run Object ▸ Make Component through the actual
 * in-app menu (component-edit.spec.ts's own proven
 * `getByRole('button', { name: 'Object' })` → menu-item-click pattern).
 */
test('nested component: drill into a member group, edit it, step back out', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // ---- setup: two boxes — box A becomes a GROUP's sole member, box B stays
  // a plain object — then Make Component on [group, box B]. Same shapes and
  // world coordinates as the kernel spec's own setup (box A: x:[0,1] y:[0,1],
  // box B: x:[3,4] y:[0,1], both z:[0,1]) — both sit inside the existing
  // CAMERA's proven framing (see its own comment above).
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const boxA = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const boxB = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    const group = h.groupNodes([{ kind: 'object', id: boxA }])
    h.selectNodes([
      { kind: 'group', id: group },
      { kind: 'object', id: boxB },
    ])
    // Capture box A's id as seen nested inside the group BEFORE any
    // component folding or session touches it — group membership doesn't
    // rename/rebake ids (unlike an instance's composed-pose surfacing), but
    // this queries fresh rather than assuming that, matching
    // explode-session.spec.ts's own "query the member id, don't assume it"
    // pattern.
    const groupMembers = h.getGroupMembers(group)
    return { boxA, boxB, group, groupMemberId: groupMembers[0].id }
  })
  const { group, groupMemberId } = setup

  // ---- Make Component through the real Object menu — the mixed group+object
  // selection `canMakeComponent` allows (both are top-level siblings, no
  // parent) but the harness's own `makeComponent` stub cannot express.
  // Scoped + exact: a bare page-level 'Object' would also match the
  // tray's 'Object Info' section header.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object', exact: true }).click()
  await page.getByText('Make Component', { exact: true }).click()
  await page.waitForTimeout(200)

  // The command selects the new instance on success (`handleMakeComponent`)
  // — reading it back here is how this test learns the instance/definition
  // handles without a return value from a direct API call.
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel).toEqual([{ kind: 'instance', id: sel[0]?.id }])
  const instance = sel[0].id
  const component = await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), instance)
  expect(component).not.toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  // The definition kept the group whole as one member, alongside box B as a
  // plain object member — exactly the kernel spec's own
  // `def_member_nodes(cid)` shape, checked here through the harness's own
  // group/object member queries since it has no generic "all def members"
  // accessor.
  const boundsPristine = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    groupMemberId,
  )
  const boxBMemberIds = await page.evaluate(
    (c) => window.__hew_test!.getComponentMemberObjects(c),
    component!,
  )
  expect(boxBMemberIds).toHaveLength(1)
  const boxBBoundsPristine = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    boxBMemberIds[0],
  )

  // ---- 2. Double-click the component instance open (real gesture) —
  // depth 1. Aimed at box B's top face (a plain member, not the group) so
  // this step alone can't be confused with the group-entry step below.
  await dblClickWorld(page, ctx, 3.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(instance)
  expect(await editingChipCount(page)).toBe(1)

  // ---- 3. Double-click the surfaced member GROUP — a SECOND, stacked
  // session opens on top of the component frame; this is the playtest bug's
  // exact repro (it used to refuse "Another component is already open for
  // editing"). The harness exposes no direct session-stack query (only
  // `getExplodeSessionInstance`, which — per `Document::component_session`
  // — reads ONLY `sessions.last()` and returns it exclusively when the
  // INNERMOST frame is a Component); with a Group frame now innermost it
  // reads null, not the outer component's instance. That transition — null,
  // while the chip count grows from 1 to 2 — is itself the proof the frame
  // stacked on top rather than replacing or refusing the component frame.
  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  expect(await editingChipCount(page)).toBe(2)

  // ---- 4. A real Push/Pull drag on the grouped solid's own top face — the
  // group session's ungroup posture makes it a genuinely top-level world
  // object for this innermost session's duration (group-session.spec.ts's
  // own rationale; no bake/pose involved, unlike a component session).
  const boundsBeforePush = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    groupMemberId,
  )
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [0.5, 0.5, 1], [0.5, 0.5, 2.5])
  const boundsAfterPush = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    groupMemberId,
  )
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  expect(boundsAfterPush[5]).toBeGreaterThan(boundsBeforePush[5] + 1) // maxZ grew well past the original 1

  // ---- 5. Escape once: back to depth 1 (still editing the component, not
  // closed) — matching group-session.spec.ts's/the sibling test's own
  // two-Escape layering check.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(instance)
  expect(await editingChipCount(page)).toBe(1)

  // Escape again: depth 0, nothing open.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  expect(await editingChipCount(page)).toBe(0)

  // ---- 6. The edit landed in the definition, and the definition still
  // carries a GROUP member (not something the group session's close
  // flattened away). `getComponentMemberObjects` only ever surfaces direct
  // OBJECT members (component-edit-parity.md) — the grouped box is nested a
  // level deeper, inside the group, so it can never appear there; that's a
  // genuine limitation of what this harness call can see, not a gap in this
  // assertion. What the harness CAN see and this asserts instead:
  //   - the grouped member's own bounds carry the push (direct proof the
  //     edit reached the shared definition, the same object handle queried
  //     throughout — no session ever renamed/rebaked it);
  //   - the definition's direct object member (box B) is untouched — the
  //     edit was scoped to the group, not the whole definition;
  //   - the group is still one of the definition's members at all (queried
  //     via `getGroupMembers`, which the group-session tests establish
  //     answers empty ONLY while ITS OWN session is open — with everything
  //     closed now, a non-empty answer here means the group survived as a
  //     real member, not dissolved/flattened by the close).
  const boundsFinal = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    groupMemberId,
  )
  expect(boundsFinal[5]).toBeGreaterThan(boundsPristine[5] + 1)

  const boxBMemberIdsFinal = await page.evaluate(
    (c) => window.__hew_test!.getComponentMemberObjects(c),
    component!,
  )
  expect(boxBMemberIdsFinal).toEqual(boxBMemberIds)
  const boxBBoundsFinal = await page.evaluate(
    (id) => window.__hew_test!.getObjectBounds(id),
    boxBMemberIdsFinal[0],
  )
  expect(boxBBoundsFinal).toEqual(boxBBoundsPristine)

  const groupMembersFinal = await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), group)
  expect(groupMembersFinal).toEqual([{ kind: 'object', id: groupMemberId }])
})
