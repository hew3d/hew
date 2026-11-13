import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Copy/Cut/Paste/Paste In Place (docs/design/v1.1-cycle.md Lane D): the
 * always-on JS keydown bindings (⌘C/⌘X/⌘V/⇧⌘V — no native accelerator, see
 * main.rs's comment by `edit_cut`), driven with real keyboard + mouse
 * events, exactly like `playtest-fixes.spec.ts`'s Move+Alt copy specs.
 *
 * Cross-window paste (the Tauri shared-clipboard mirror) is desktop-only
 * and can't be exercised by this web-build suite — `modelClipboard.test.ts`
 * covers that half in isolation (mocked `@tauri-apps/api/core`).
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 8, y: 6, z: 8 },
  target: { x: 1, y: 1, z: 0 },
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

async function pinCamera(page: Page, cam: CameraParams): Promise<Ctx> {
  await page.evaluate(
    (c) =>
      window.__hew_test!.setCamera({
        position: [c.position.x, c.position.y, c.position.z],
        target: [c.target.x, c.target.y, c.target.z],
        up: [c.up.x, c.up.y, c.up.z],
        fovDeg: c.fovDeg,
      }),
    cam,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(cam, rect.width / rect.height), rect }
}

async function setup(page: Page, linked = false): Promise<Ctx> {
  // Most specs here predate linked copies and assert on plain, independent
  // copies; the preference gives them exactly that. The instance specs ask
  // for the default (`linked`); linked-copies.spec.ts covers the rest.
  await page.addInitScript(
    (on) => localStorage.setItem('hew.settings.copiesLinked', String(on)),
    linked,
  )
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  return pinCamera(page, CAMERA)
}

async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

/** A real double-click at a world point — opens a session (group or
 *  component) on whatever it resolves to, mirroring group-session.spec.ts's
 *  own helper of the same name. */
async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

/** Activate a tool with no rail slot via the web MenuBar's Tools dropdown —
 *  mirrors axes-tool.spec.ts's own helper (Drawing Axes lives here, not on
 *  the rail). Blurs afterward so a following `page.keyboard.press` reaches
 *  the canvas, not the menu item's button. */
async function activateFromToolsMenu(page: Page, label: string): Promise<void> {
  const menuBar = page.getByTestId('menu-bar')
  await menuBar.getByRole('button', { name: /^tools$/i }).click()
  await menuBar.getByText(label, { exact: true }).click()
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
}

// Paste target: world (2,-2,0) projects (under CAMERA) onto open canvas, well
// clear of the bottom Draw dock's buttons — a farther point like (4,4,0)
// projects UNDER that fixed overlay, so the "click" lands on a tool button
// instead of committing the placement.

test('Copy then Paste places a second box at the click point', async ({ page }) => {
  const ctx = await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((oid) => window.__hew_test!.selectObjects([oid]), id)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  await page.keyboard.press('Control+c')
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()

  await page.keyboard.press('Control+v')
  // The paste arms asynchronously (the clipboard is read first); wait for
  // the placement hint before clicking, as a hand would wait for the ghost.
  await expect(page.getByText('Click to place', { exact: false })).toBeVisible()
  // The ghost is armed — a click on open ground commits the paste there.
  await clickWorld(page, ctx, 2, -2, 0)

  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2)
  // The pasted copy is now the selection (LibraryPlaceTool's onPlaced
  // contract), distinct from the original.
  const selection = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(selection).toHaveLength(1)
  expect(selection[0].id).not.toBe(id)

  // The original is untouched — a real second box exists, not a move.
  const originalBounds = await page.evaluate((oid) => window.__hew_test!.getObjectBounds(oid), id)
  expect(originalBounds[0]).toBeCloseTo(0, 5)
  expect(originalBounds[1]).toBeCloseTo(0, 5)
})

test('Paste In Place inserts at the original position with no click, no armed gesture', async ({ page }) => {
  await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([2, 2, 0], [3, 3, 0], 1))
  await page.evaluate((oid) => window.__hew_test!.selectObjects([oid]), id)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  const originalBounds = await page.evaluate((oid) => window.__hew_test!.getObjectBounds(oid), id)

  await page.keyboard.press('Control+c')
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()

  await page.keyboard.press('Control+Shift+v')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2)

  const selection = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(selection).toHaveLength(1)
  const pastedId = selection[0].id
  expect(pastedId).not.toBe(id)
  const pastedBounds = await page.evaluate((oid) => window.__hew_test!.getObjectBounds(oid), pastedId)
  // Identity affine: the pasted copy overlaps the original exactly.
  for (let i = 0; i < 6; i++) {
    expect(pastedBounds[i]).toBeCloseTo(originalBounds[i], 5)
  }
})

test('Cut removes the original and leaves the clipboard pasteable', async ({ page }) => {
  const ctx = await setup(page)
  const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((oid) => window.__hew_test!.selectObjects([oid]), id)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  await page.keyboard.press('Control+x')
  // Cut = Copy (async: hashes the bytes before the clipboard is actually
  // set) + delete — wait for the "Copied" toast (Copy's own completion
  // signal, same as every other test here) before Ctrl+V, or a fast
  // keypress can race the still-in-flight clipboard write and land on an
  // empty clipboard (a silent no-op paste).
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 0)

  await page.keyboard.press('Control+v')
  await expect(page.getByText('Click to place', { exact: false })).toBeVisible()
  await clickWorld(page, ctx, 2, -2, 0)
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
})

test('a sketch-only selection refuses Copy with a toast, not a crash', async ({ page }) => {
  const ctx = await setup(page)
  // A detached (never-extruded) rectangle sketch — `drawRectangle` leaves
  // exactly that behind, the sketch-scoped selection Copy must refuse.
  await page.evaluate(() => window.__hew_test!.drawRectangle([0, 0, 0], [1, 1, 0]))
  // Click its fill (a real gesture, not a constructed NodeRef — region/island
  // ids aren't the same numbering) to select the whole island.
  await clickWorld(page, ctx, 0.5, 0.5, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'draft-island'
  })
  await page.keyboard.press('Control+c')
  await expect(page.getByText(/Sketch geometry can't be copied yet/)).toBeVisible()
})

test('Cut of a multi-node selection is ONE undo step — a single Ctrl+Z restores everything', async ({ page }) => {
  await setup(page)
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const b = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    const c = h.drawBox([6, 0, 0], [7, 1, 0], 1)
    return { a, b, c }
  })
  await page.evaluate(
    (nodeIds) => window.__hew_test!.selectObjects(nodeIds),
    [ids.a, ids.b, ids.c],
  )
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 3)

  await page.keyboard.press('Control+x')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 0)

  // ONE undo restores all three — `Scene.delete_selection` batches the
  // whole cut into a single compound undo entry (adversarial review
  // finding 3), not three separate `delete_node` entries a single Ctrl+Z
  // would only partially unwind.
  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 3)
  const restoredIds = new Set(await page.evaluate(() => window.__hew_test!.getObjectIds()))
  expect(restoredIds).toEqual(new Set([ids.a, ids.b, ids.c]))
})

test('Paste In Place overlaps the original even when the drawing axes are moved', async ({ page }) => {
  const ctx = await setup(page)

  // Move the drawing axes off world identity with a REAL 3-click gesture —
  // axes-tool.spec.ts's own box/vertices, verbatim (a box near the camera's
  // target so the picks land precisely; see that spec's module doc for why
  // the TOP face, not bottom, from this camera pose, and why these three
  // particular vertices give a genuine non-identity rotation).
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await activateFromToolsMenu(page, 'Drawing Axes')
  await clickWorld(page, ctx, 0, 0, 1) // new origin
  await clickWorld(page, ctx, 0, 2, 1) // red (X) direction pick
  await clickWorld(page, ctx, 2, 0, 1) // green (Y) direction pick — commits
  const axes = await page.evaluate(() => window.__hew_test!.getDrawingAxes())
  expect(axes).not.toEqual([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])

  // The object under test, drawn AFTER the axes moved and well clear of the
  // axes-gesture box above — copied and pasted in place with the axes
  // still non-identity throughout.
  const id = await page.evaluate(() => window.__hew_test!.drawBox([5, 5, 0], [6, 6, 0], 1))
  await page.evaluate((oid) => window.__hew_test!.selectObjects([oid]), id)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  const originalBounds = await page.evaluate((oid) => window.__hew_test!.getObjectBounds(oid), id)

  await page.keyboard.press('Control+c')
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()

  const before = await page.evaluate(() => window.__hew_test!.getObjectCount())
  await page.keyboard.press('Control+Shift+v')
  await page.waitForFunction(
    (n) => window.__hew_test!.getObjectCount() === n,
    before + 1,
  )

  const selection = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(selection).toHaveLength(1)
  const pastedId = selection[0].id
  expect(pastedId).not.toBe(id)
  const pastedBounds = await page.evaluate((oid) => window.__hew_test!.getObjectBounds(oid), pastedId)
  // The kernel-computed `extract_item_placement` affine (the axes frame's
  // own forward transform, since extract_item does NOT re-origin when the
  // axes are moved) reconstructs the original position exactly — a bare
  // identity affine would instead have left it near the moved axes' own
  // origin, nowhere close to (2,2,0)-(3,3,1).
  for (let i = 0; i < 6; i++) {
    expect(pastedBounds[i]).toBeCloseTo(originalBounds[i], 5)
  }
})

test('Cut, enter a group, Paste In Place inside the session: the part folds in as a member on close', async ({ page }) => {
  const ctx = await setup(page)

  // The group's one member (box) plus the part to cut — well clear of it,
  // so the cut/paste never touches the group's own geometry.
  const setupIds = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const group = h.groupNodes([{ kind: 'object', id: box }])
    const part = h.drawBox([5, 5, 0], [6, 6, 0], 1)
    return { box, group, part }
  })
  const { group, part } = setupIds
  const originalPartBounds = await page.evaluate(
    (oid) => window.__hew_test!.getObjectBounds(oid),
    part,
  )

  // ---- 1. Select the part and Cut it (⌘X).
  await page.evaluate((oid) => window.__hew_test!.selectObjects([oid]), part)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('Control+x')
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  const hashAfterCut = await page.evaluate(() => window.__hew_test!.getStateHash())

  // ---- 2. Double-click into the group to edit it — a real double-click,
  // not the harness.
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(200)
  const hashAfterOpen = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfterOpen).not.toBe(hashAfterCut)

  // ---- 3. Paste In Place (⇧⌘V) WHILE the session is open — `insert_item`
  // is not refused during a group session (only a component session
  // refuses ExplodeSessionScope): the pasted part lands surfaced, inside
  // the session's own scope.
  await page.keyboard.press('Control+Shift+v')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2)
  const selection = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(selection).toHaveLength(1)
  const pastedId = selection[0].id
  expect(pastedId).not.toBe(part)
  const pastedBoundsInSession = await page.evaluate(
    (oid) => window.__hew_test!.getObjectBounds(oid),
    pastedId,
  )
  for (let i = 0; i < 6; i++) {
    expect(pastedBoundsInSession[i]).toBeCloseTo(originalPartBounds[i], 5)
  }
  const hashAfterPaste = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfterPaste).not.toBe(hashAfterOpen)

  // ---- 4. Exit the edit (Escape) — every node surfaced since the session
  // opened folds into the group at close (`exit_group_session` /
  // `nodes_surfaced_since`), so the pasted part becomes a member, at its
  // original (pasted) position — folding is pure tree bookkeeping, no
  // geometry change.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  const membersAfter = await page.evaluate((g) => window.__hew_test!.getGroupMembers(g), group)
  expect(membersAfter.map((m) => m.id).sort()).toEqual(
    [setupIds.box, pastedId].sort(),
  )
  const pastedBoundsAfterClose = await page.evaluate(
    (oid) => window.__hew_test!.getObjectBounds(oid),
    pastedId,
  )
  for (let i = 0; i < 6; i++) {
    expect(pastedBoundsAfterClose[i]).toBeCloseTo(originalPartBounds[i], 5)
  }
  const hashAfterClose = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfterClose).not.toBe(hashAfterPaste)

  // ---- 5. One undo per step restores each in turn: close → paste → open
  // → cut, each a single labeled undo entry (DEVELOPMENT.md rule 9: undo
  // is subject to the same replay contract inside a session as outside
  // it).
  await page.keyboard.press('Control+z') // undoes the close/fold
  await page.waitForFunction(
    (h) => window.__hew_test!.getStateHash() === h,
    hashAfterPaste,
  )
  await page.keyboard.press('Control+z') // undoes the paste
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  await page.keyboard.press('Control+z') // undoes opening the session
  await page.waitForFunction(
    (h) => window.__hew_test!.getStateHash() === h,
    hashAfterCut,
  )
  await page.keyboard.press('Control+z') // undoes the cut
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2)
  const restoredPartId = (await page.evaluate(() => window.__hew_test!.getObjectIds())).find(
    (id) => id !== setupIds.box,
  )
  expect(restoredPartId).toBeDefined()
  const restoredBounds = await page.evaluate(
    (oid) => window.__hew_test!.getObjectBounds(oid),
    restoredPartId,
  )
  expect(restoredBounds).toEqual(originalPartBounds)
})

test('Paste In Place is refused with a clear toast while a component session is open', async ({ page }) => {
  const ctx = await setup(page)

  const setupIds = await page.evaluate(() => {
    const h = window.__hew_test!
    const member = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance } = h.makeComponent([member])
    const part = h.drawBox([5, 5, 0], [6, 6, 0], 1)
    return { instance, part }
  })

  // Copy a part, then double-click the instance to open its component
  // session — a real double-click, not the harness.
  await page.evaluate((oid) => window.__hew_test!.selectObjects([oid]), setupIds.part)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('Control+c')
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()

  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await page.waitForFunction(() => window.__hew_test!.getExplodeSessionInstance() !== null)

  // `insert_document` refuses `ExplodeSessionScope` while a COMPONENT
  // session is open (unlike a group session — see the test above) — a
  // clear toast, and the document untouched.
  const countBefore = await page.evaluate(() => window.__hew_test!.getObjectCount())
  await page.keyboard.press('Control+Shift+v')
  await expect(
    page.getByText(/isn't available while a group or component is open for editing/),
  ).toBeVisible()
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(countBefore)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance() !== null)).toBe(
    true,
  )

  await page.keyboard.press('Escape')
})

// ---------------------------------------------------------------------------
// Component instances stay LINKED through Copy/Paste within one document
// (SketchUp's model: the clipboard references the definition; Make Unique
// is how you break the link). The paste path once minted an unlinked copy
// of the definition — `insert_item` only ever reused a definition by
// `hew.library` provenance, which the ORIGINAL definition never carried —
// so editing the pasted copy left the original alone. Now Copy stamps the
// bytes with the window's origin and Paste links back to the live original.

/** Real Push/Pull drag: first click anchors, move drags, second click
 *  commits (live-siblings.spec.ts's helper of the same name). */
async function dragWorld(
  page: Page,
  ctx: Ctx,
  anchor: [number, number, number],
  target: [number, number, number],
): Promise<void> {
  const a = px(ctx, anchor[0], anchor[1], anchor[2])
  const t = px(ctx, target[0], target[1], target[2])
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.up()
  await page.mouse.move(t.x, t.y, { steps: 10 })
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
}

// A 3/4 view with the original at the origin and the paste target at
// (3, 0, 0) both on open canvas, with headroom above for the pulled top.
const LINK_CAMERA: CameraParams = {
  position: { x: 2.5, y: -8, z: 6 },
  target: { x: 2, y: 0.5, z: 0.8 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.1,
  far: 1000,
}

/** A box folded into a component, selected, and copied with a real ⌘C /
 *  Ctrl+C — the shared opening of every spec below. */
async function copyOneInstance(page: Page): Promise<{ instance: string; component: string }> {
  const made = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    return h.makeComponent([box])
  })
  await page.evaluate((i) => window.__hew_test!.selectNodes([{ kind: 'instance', id: i }]), made.instance)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('Control+c')
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()
  return made
}

test('Copy/Paste of a component instance places a LINKED instance: editing the paste edits the original', async ({
  page,
}) => {
  // The Components panel open, so the definition list is on screen.
  await page.addInitScript(() => {
    localStorage.setItem(
      'hew.settings.trayLayout',
      JSON.stringify({ modelInfo: false, objectInfo: false, materials: false, components: true, tags: false, scenes: false }),
    )
  })
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const ctx = await pinCamera(page, LINK_CAMERA)
  const { instance, component } = await copyOneInstance(page)

  // Paste with a real Ctrl+V and a click on open ground at (3, 0, 0).
  await page.keyboard.press('Control+v')
  await expect(page.getByText('Click to place', { exact: false })).toBeVisible()
  await clickWorld(page, ctx, 3, 0, 0)
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 2, component)

  // ONE definition, two placements — the paste is an instance of the
  // ORIGINAL definition, not of a new one.
  const selection = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(selection).toHaveLength(1)
  expect(selection[0].kind).toBe('instance')
  const pasted = selection[0].id
  expect(pasted).not.toBe(instance)
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), pasted)).toBe(component)
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([component])
  expect(await page.evaluate((c) => window.__hew_test!.definitionUsage(c), component)).toBe(2)
  await expect(page.getByTestId('components-row-usage')).toHaveCount(1)
  await expect(page.getByTestId('components-row-usage').first()).toHaveText('2')

  // Enter the PASTED copy with a real double-click on its top face, pull
  // the top up 1 m with a real Push/Pull drag, step out with Escape.
  const members = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  expect(members).toHaveLength(1)
  const memberId = members[0]
  const before = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(before[5]).toBeCloseTo(1, 6)
  // The paste re-origins to the copy's bottom center, so the pasted box
  // spans (2.5..3.5, -0.5..0.5); its top-face center is (3, 0, 1).
  await dblClickWorld(page, ctx, 3, 0, 1)
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(pasted)
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [3, 0, 1], [3, 0, 2])
  await page.keyboard.press('Escape')
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()

  // BOTH copies carry the edit: there is one shared member, now taller,
  // and both instances still place the same definition.
  const after = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  expect(after).toHaveLength(1)
  const bounds = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), after[0])
  expect(bounds[5]).toBeGreaterThan(1.5)
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)))
    .toEqual(new Set([instance, pasted]))
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), instance)).toBe(component)
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), pasted)).toBe(component)
  // A pick straight down onto the ORIGINAL lands on the raised top: the
  // original's shape changed with the paste's edit.
  const pickOriginal = await page.evaluate(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]),
  )
  expect(pickOriginal?.object).toBe(after[0])
  expect(pickOriginal?.instance).toBe(instance)
})

test('Paste In Place of a component instance is linked too, and undo of the paste leaves the definition alone', async ({
  page,
}) => {
  await setup(page, true)
  const { instance, component } = await copyOneInstance(page)

  await page.keyboard.press('Control+Shift+v')
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 2, component)
  const pasted = (await page.evaluate(() => window.__hew_test!.getSelection()))[0].id
  expect(pasted).not.toBe(instance)
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), pasted)).toBe(component)
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([component])

  // Undo the paste: the pasted instance goes, the shared definition and the
  // original placement stay exactly as they were. Redo brings it back,
  // still linked.
  await page.keyboard.press('Control+z')
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 1, component)
  expect(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).toEqual([instance])
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([component])
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), pasted)).toBeNull()
  await page.evaluate(() => window.__hew_test!.redo())
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 2, component)
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), pasted)).toBe(component)
})

test('Copy of several instances of one definition pastes them onto that ONE definition', async ({ page }) => {
  await setup(page, true)
  const made = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const second = h.placeInstance(component, 2, 0, 0)
    return { instance, component, second }
  })
  await page.evaluate(
    (ids) => window.__hew_test!.selectNodes(ids.map((id) => ({ kind: 'instance', id }))),
    [made.instance, made.second],
  )
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 2)
  await page.keyboard.press('Control+c')
  await expect(page.getByText('Copied 2 objects', { exact: false })).toBeVisible()

  await page.keyboard.press('Control+Shift+v')
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 4, made.component)
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([made.component])
  expect(await page.evaluate((c) => window.__hew_test!.definitionUsage(c), made.component)).toBe(4)
})

test('Cut then Paste of a component instance re-places the same definition', async ({ page }) => {
  await setup(page, true)
  const made = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const sibling = h.placeInstance(component, 2, 0, 0)
    return { instance, component, sibling }
  })
  await page.evaluate((i) => window.__hew_test!.selectNodes([{ kind: 'instance', id: i }]), made.instance)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('Control+x')
  await expect(page.getByText('Copied 1 object', { exact: false })).toBeVisible()
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 1, made.component)

  await page.keyboard.press('Control+Shift+v')
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 2, made.component)
  const pasted = (await page.evaluate(() => window.__hew_test!.getSelection()))[0].id
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), pasted)).toBe(made.component)
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), made.sibling)).toBe(made.component)
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([made.component])
})

test('Move+copy of an instance (the path that was already linked) still shares the definition', async ({ page }) => {
  await setup(page)
  const made = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const copy = h.copyInstance(instance, 2, 0, 0)
    return { instance, component, copy }
  })
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), made.copy)).toBe(made.component)
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([made.component])
  expect(await page.evaluate((c) => window.__hew_test!.definitionUsage(c), made.component)).toBe(2)
})

test('A copy taken before the document is reloaded pastes as an ordinary copy, not a link', async ({ page }) => {
  await setup(page)
  const { instance, component } = await copyOneInstance(page)

  // Replace the document through the real Open path (same bytes, so the
  // definition keeps its stable id): the kernel ids collide by design, and
  // only the origin epoch tells the old copy apart from this document.
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.load(h.save())
  })
  await page.waitForFunction((c) => window.__hew_test!.getInstancesOf(c).length === 1, component)
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([component])

  await page.keyboard.press('Control+Shift+v')
  await page.waitForFunction(() => window.__hew_test!.getComponentIds().length === 2)
  const pasted = (await page.evaluate(() => window.__hew_test!.getSelection()))[0].id
  expect(pasted).not.toBe(instance)
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceDef(i), pasted)).not.toBe(component)
  expect(await page.evaluate((c) => window.__hew_test!.definitionUsage(c), component)).toBe(1)
})
