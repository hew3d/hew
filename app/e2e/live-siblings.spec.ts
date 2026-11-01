import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Live sibling placements (ARCHITECTURE.md 2.11; bug report HEW-AAQG-41X7):
 * a component copied once (two instances, one definition), one copy opened
 * for editing with a real double-click. The OTHER copy must stay visible
 * for the whole session and show the edit as it happens — SketchUp's
 * behavior, and what a person coming from SketchUp expects. It used to
 * vanish: the explode session hid every placement of the definition.
 *
 * Real pointer input throughout (the same double-click / two-click Push/Pull
 * drag explode-session.spec.ts uses). Ground truth is structural where a
 * query exists (`getSessionPlacements`, `getInstanceExpandedMembers`,
 * `pickFace` through the sibling — it is snappable/pickable like any live
 * instance), and a screenshot crop of the sibling's own screen region where
 * only pixels can say "it is drawn, and it changed with the edit".
 *
 * A second test covers Part A of the same report: Push/Pull hovering a
 * component face OUTSIDE its edit session explains itself beside the cursor
 * and in the status bar before the click, and the refused click's toast is
 * the emphasized guidance toast.
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

/** Move first (settles the hover pick before the down), then click. */
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

/** A real click-drag-click through Push/Pull: first click arms, a real
 *  move drags, the second click commits. */
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

/** A real double-click at a world point — opens an explode session on a
 *  uniformly-posed component instance. */
async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

/** The page-pixel box the world box `[lo, hi]` projects into, padded. */
function screenBox(
  ctx: Ctx,
  lo: [number, number, number],
  hi: [number, number, number],
  pad = 6,
): { x: number; y: number; width: number; height: number } {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity
  for (const x of [lo[0], hi[0]]) for (const y of [lo[1], hi[1]]) for (const z of [lo[2], hi[2]]) {
    const p = px(ctx, x, y, z)
    minX = Math.min(minX, p.x); maxX = Math.max(maxX, p.x)
    minY = Math.min(minY, p.y); maxY = Math.max(maxY, p.y)
  }
  return { x: minX - pad, y: minY - pad, width: maxX - minX + 2 * pad, height: maxY - minY + 2 * pad }
}

// A 3/4 view with both copies in frame: A is a 2×1×1 box at the origin, B
// the same definition placed 4 m along +x, with headroom above both for the
// pulled top to grow into.
const CAMERA: CameraParams = {
  position: { x: 3, y: -9, z: 6.5 },
  target: { x: 3, y: 0.5, z: 0.8 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.1,
  far: 1000,
}

test('live siblings: the other copy stays visible while one copy is edited, and shows the edit live', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // Where copy B WILL sit on screen (its box plus the headroom the pull
  // will raise it into) — the crop every pixel check below reads. Taken
  // first with only copy A in the model: the real "B absent" baseline.
  const bRegion = screenBox(ctx, [4, 0, 0], [6, 1, 2.2])
  const downOntoB = { origin: [5, 0.5, 5] as [number, number, number], dir: [0, 0, -1] as [number, number, number] }
  const first = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    return h.makeComponent([box])
  })
  await page.waitForTimeout(150)
  const bAbsent = await page.screenshot({ clip: bRegion })
  const instanceB = await page.evaluate((c) => window.__hew_test!.placeInstance(c, 4, 0, 0), first.component)
  await page.waitForTimeout(150)
  const setup = { ...first, instanceB }
  const { component } = setup
  const members0 = await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), component)
  expect(members0).toHaveLength(1)
  const memberId = members0[0]
  const bBefore = await page.screenshot({ clip: bRegion })
  expect(bBefore.equals(bAbsent)).toBe(false) // placing B drew something there

  // ---- Open copy A with a real double-click on its top face.
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(setup.instance)

  // Copy B is a LIVE placement rendering the session's world member.
  expect(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)).toEqual([instanceB])
  expect(await page.evaluate(() => window.__hew_test!.getSessionPlacements())).toEqual([instanceB])
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceExpandedMembers(i), instanceB)).toEqual([memberId])
  // Pickable/snappable like any instance: a ray down onto B's top strikes
  // the shared member THROUGH B.
  const pickB = await page.evaluate((r) => window.__hew_test!.pickFace(r.origin, r.dir), downOntoB)
  expect(pickB).not.toBeNull()
  expect(pickB!.object).toBe(memberId)
  expect(pickB!.instance).toBe(instanceB)
  // And it is still drawn: B's region differs from the real B-absent
  // baseline taken before B was placed. (Dimming — everything outside the
  // session fades — means it also differs from `bBefore`; the edit below
  // is what proves the drawing is LIVE.)
  const bOpen = await page.screenshot({ clip: bRegion })
  expect(bOpen.equals(bAbsent)).toBe(false)

  // ---- Push/Pull A's top face up by 1 m — a real drag. Copy B's pixels
  // must change with it (the pulled top grows into B's headroom), and B's
  // placement must still be the one shared member, now taller.
  await page.keyboard.press('p')
  await dragWorld(page, ctx, [1, 0.5, 1], [1, 0.5, 2])
  const boundsAfterPush = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(boundsAfterPush[5]).toBeGreaterThan(1.5)
  await page.waitForTimeout(200)
  const bPushed = await page.screenshot({ clip: bRegion })
  expect(bPushed.equals(bOpen)).toBe(false)
  await page.screenshot({ path: 'test-results/live-siblings-mid-session.png' })
  expect(await page.evaluate((i) => window.__hew_test!.getInstanceExpandedMembers(i), instanceB)).toEqual([memberId])
  const pickBPushed = await page.evaluate((r) => window.__hew_test!.pickFace(r.origin, r.dir), downOntoB)
  expect(pickBPushed?.instance).toBe(instanceB)

  // ---- Abort the edit: undo reverts copy B with it.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(setup.instance)
  const boundsUndone = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(boundsUndone[5]).toBeCloseTo(1, 6)
  // Not merely "different from the pushed state": pixel-identical to how
  // B looked right after the open, before the edit.
  const bUndone = await page.screenshot({ clip: bRegion })
  expect(bUndone.equals(bPushed)).toBe(false)
  expect(bUndone.equals(bOpen)).toBe(true)

  // ---- Redo, then Escape closes the session: both copies carry the edit.
  await page.evaluate(() => window.__hew_test!.redo())
  await page.waitForTimeout(150)
  await page.keyboard.press('Escape')
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getSessionPlacements())).toEqual([])
  expect(new Set(await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), component)))
    .toEqual(new Set([setup.instance, instanceB]))
  // Closed: B is undimmed again (differs from the mid-session look) and
  // taller than before the edit (differs from the pre-edit look), and a
  // pick down onto it lands on the raised top through B.
  const bClosed = await page.screenshot({ clip: bRegion })
  expect(bClosed.equals(bBefore)).toBe(false)
  expect(bClosed.equals(bPushed)).toBe(false)
  expect(bClosed.equals(bAbsent)).toBe(false)
  const pickClosed = await page.evaluate((r) => window.__hew_test!.pickFace(r.origin, r.dir), downOntoB)
  expect(pickClosed?.instance).toBe(instanceB)
  const boundsClosed = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), memberId)
  expect(boundsClosed[5]).toBeGreaterThan(1.5)
  await page.screenshot({ path: 'test-results/live-siblings-after-close.png' })
})

test('live siblings: a Push/Pull drag previews on the other copy mid-drag, and Escape takes it away', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  const bRegion = screenBox(ctx, [4, 0, 0], [6, 1, 2.2])
  const downOntoB = { origin: [5, 0.5, 5] as [number, number, number], dir: [0, 0, -1] as [number, number, number] }
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const { instance, component } = h.makeComponent([box])
    const instanceB = h.placeInstance(component, 4, 0, 0)
    return { instance, component, instanceB, memberId: h.getComponentMemberObjects(component)[0] }
  })
  await page.waitForTimeout(150)

  // Open copy A, pick Push/Pull, and park the cursor on A's top: the idle
  // look of copy B every comparison below is against.
  await dblClickWorld(page, ctx, 1, 0.5, 1)
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(setup.instance)
  await page.keyboard.press('p')
  const anchor = px(ctx, 1, 0.5, 1)
  const target = px(ctx, 1, 0.5, 2)
  const park = async (): Promise<void> => {
    await page.mouse.move(anchor.x - 4, anchor.y - 4)
    await page.waitForTimeout(40)
    await page.mouse.move(anchor.x, anchor.y)
    await page.waitForTimeout(200)
  }
  await park()
  const bIdle = await page.screenshot({ clip: bRegion })

  // Arm the drag on A's top and move up 1 m WITHOUT the committing click:
  // the swept ghost stands on A, and its mirror stands on B. Nothing has
  // reached the kernel — B's member is still 1 m tall and B still picks as
  // itself — yet B's pixels changed with the gesture.
  await page.mouse.down()
  await page.mouse.up()
  await page.mouse.move(target.x, target.y, { steps: 10 })
  await page.waitForTimeout(250)
  const bMidDrag = await page.screenshot({ clip: bRegion })
  await page.screenshot({ path: 'test-results/live-siblings-mid-drag.png' })
  expect(bMidDrag.equals(bIdle), 'copy B shows the drag in flight').toBe(false)
  const boundsMid = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), setup.memberId)
  expect(boundsMid[5]).toBeCloseTo(1, 6)
  const pickMid = await page.evaluate((r) => window.__hew_test!.pickFace(r.origin, r.dir), downOntoB)
  expect(pickMid?.instance).toBe(setup.instanceB)
  expect(await page.evaluate(() => window.__hew_test!.getSessionPlacements())).toEqual([setup.instanceB])

  // Escape cancels the gesture (the session stays open): the mirror goes
  // with the ghost, and B is pixel-identical to its idle look.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(100)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(setup.instance)
  await park()
  const bAfterEscape = await page.screenshot({ clip: bRegion })
  await page.screenshot({ path: 'test-results/live-siblings-after-escape.png' })
  expect(bAfterEscape.equals(bIdle), 'copy B is back to its idle look after Escape').toBe(true)

  // A committed drag leaves no mirror behind either. The committed look
  // of B is compared with a baseline that never had a preview at all: the
  // same pull done through the harness (no gesture, no ghost, no mirror),
  // so a mirror lingering after the drag's commit would make the two
  // differ. Then undo, and B is pixel-identical to idle again.
  await page.mouse.down()
  await page.mouse.up()
  await page.mouse.move(target.x, target.y, { steps: 10 })
  await page.waitForTimeout(150)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
  const boundsCommitted = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), setup.memberId)
  expect(boundsCommitted[5]).toBeCloseTo(2, 3)
  await park()
  const bCommitted = await page.screenshot({ clip: bRegion })
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(250)
  await page.evaluate((id) => {
    const h = window.__hew_test!
    const top = h.pickFace([1, 0.5, 5], [0, 0, -1])
    if (top === null || top.object !== id) throw new Error('the member top did not pick')
    h.pushPull(top.object, top.face, 1)
  }, setup.memberId)
  await page.waitForTimeout(250)
  const boundsBaseline = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), setup.memberId)
  expect(boundsBaseline[5]).toBeCloseTo(2, 3)
  await park()
  const bCommittedBaseline = await page.screenshot({ clip: bRegion })
  expect(bCommitted.equals(bCommittedBaseline), 'the drag’s commit shows the solid alone, like a pull with no gesture').toBe(true)
  expect(bCommitted.equals(bMidDrag)).toBe(false)
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(250)
  await park()
  const bUndone = await page.screenshot({ clip: bRegion })
  expect(bUndone.equals(bIdle), 'copy B is back to its idle look after commit + undo').toBe(true)

  await page.keyboard.press('Escape')
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
})

test('live siblings: Push/Pull explains a component face outside its edit session beside the cursor', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))
  await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const { component } = h.makeComponent([box])
    h.placeInstance(component, 4, 0, 0)
  })

  await page.keyboard.press('p')
  // Hover copy A's top face: the inference chip carries the way in, and so
  // does the status bar — before any click.
  const top = px(ctx, 1, 0.5, 1)
  await page.mouse.move(top.x - 6, top.y - 6)
  await page.waitForTimeout(60)
  await page.mouse.move(top.x, top.y)
  await page.waitForTimeout(200)
  const hint = page.getByTestId('inference-tip-hint')
  await expect(hint).toBeVisible()
  await expect(hint).toContainText('double-click it to edit the component')
  expect(await page.getByText('double-click it to edit the component').count()).toBeGreaterThanOrEqual(2) // the chip and the status bar

  // The refused click adds the emphasized guidance toast: the one toast
  // on screen carries the hint, is marked as guidance (accent styling, no
  // code prefix), and the click itself drops the hover memo.
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  const toast = page.getByTestId('toast').filter({ hasText: 'double-click it to edit the component' })
  await expect(toast).toHaveCount(1)
  await expect(toast).toHaveAttribute('data-emphasis', 'true')
  await expect(toast).not.toContainText('EditContextHint')
  await page.screenshot({ path: 'test-results/live-siblings-component-face-hint.png' })

  // Over empty ground there is nothing to explain.
  const ground = px(ctx, 3, -2, 0)
  await page.mouse.move(ground.x, ground.y)
  await page.waitForTimeout(200)
  await expect(page.getByTestId('inference-tip-hint')).toHaveCount(0)
})
