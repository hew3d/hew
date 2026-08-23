import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Axes tool E2E — the movable drawing axes gesture (tool-parity §4), driven
 * with REAL pointer events on the canvas (`page.mouse`), the same
 * pixel-interaction pattern `input-pipeline.spec.ts` uses — not the semantic
 * `window.__hew_test` harness shortcuts `tools.spec.ts` relies on. This is
 * specifically a test of the click → snap → gesture → `set_axes` pipeline,
 * so it has to go through real clicks; a harness call would bypass exactly
 * the wiring being tested.
 *
 * Deliberate simplification (documented per the task brief, which allows
 * trading "perfect" inference-snapped picks for robust determinism): rather
 * than clicking bare, unsnapped points in empty space and hoping the
 * inference engine happens to resolve them to the intended coordinates, the
 * three Axes-tool picks land on the VERTICES of a harness-drawn box
 * (`drawBox`) — real vertex snapping onto known, exact geometry, exactly the
 * technique `input-pipeline.spec.ts`'s own Rotate/Select specs already rely
 * on for deterministic real-pointer clicks. The picked vertices are chosen
 * so the resulting frame is a genuine (non-identity) rotation: red along
 * former world +Y, green along former world +X. The three picks land on the
 * box's TOP face (z = height) rather than its bottom — from this suite's
 * camera pose the bottom corners sit behind the box's own solid geometry
 * (a ray toward their screen pixel hits the nearer top/side faces first),
 * while the whole top face is unobstructed and faces the camera directly.
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

async function setup(page: Page): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
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

/** Click the canvas at the pixel where `world` renders — move first so the
 *  tool sees a hover (snap resolve) before the down, as a real hand
 *  produces (mirrors input-pipeline.spec.ts's `clickWorld`). */
async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

/** Activate a tool with no rail slot via the web MenuBar's Tools dropdown
 *  (Axes/Protractor/Slice/Edit Vertex all live here, not on the rail) —
 *  mirrors `section-plane.spec.ts`'s View-menu row helper. Blurs afterward
 *  so a following `page.keyboard.press` reaches the canvas, not the menu
 *  item's button. */
async function activateFromToolsMenu(page: Page, label: string): Promise<void> {
  const menuBar = page.getByTestId('menu-bar')
  await menuBar.getByRole('button', { name: /^tools$/i }).click()
  await menuBar.getByText(label, { exact: true }).click()
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
}

/** Reset the drawing axes to world identity via View ▸ Reset Drawing Axes. */
async function resetAxesFromMenu(page: Page): Promise<void> {
  const menuBar = page.getByTestId('menu-bar')
  await menuBar.getByRole('button', { name: /^view$/i }).click()
  await menuBar.getByText('Reset Drawing Axes', { exact: true }).click()
}

const WORLD_IDENTITY = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]

test('Drawing Axes tool: three real clicks move the drawing axes, and a locked draw lands on the new plane', async ({
  page,
}) => {
  const ctx = await setup(page)

  // A box gives three well-defined, non-collinear vertices to click:
  // (0,0,1) the new origin, (0,2,1) along the new red (X) axis, (2,0,1)
  // completing the new green (Y) axis — all on the box's TOP face (see the
  // module doc comment on why top, not bottom). This is a real rotation,
  // not a relabeling: red ends up along former world +Y, green along
  // former world +X (Gram-Schmidt from the raw picks — see AxesTool's own
  // doc comment).
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))

  const hash0 = await page.evaluate(() => window.__hew_test!.getStateHash())
  await activateFromToolsMenu(page, 'Drawing Axes')

  await clickWorld(page, ctx, 0, 0, 1) // origin
  await clickWorld(page, ctx, 0, 2, 1) // red (X) direction pick
  await clickWorld(page, ctx, 2, 0, 1) // green (Y) direction pick — commits

  // The frame actually moved (real `set_axes` call, one undo step).
  const afterMove = await page.evaluate(() => ({
    axes: window.__hew_test!.getDrawingAxes(),
    hash: window.__hew_test!.getStateHash(),
    canUndo: window.__hew_test!.canUndo(),
    err: window.__hew_test!.getLastError(),
  }))
  expect(afterMove.err).toBeNull()
  expect(afterMove.hash).not.toBe(hash0)
  expect(afterMove.canUndo).toBe(true)
  const [ox, oy, oz, xx, xy, xz, yx, yy, yz] = afterMove.axes
  const closeTriple = (got: number[], want: number[]): void => {
    for (let i = 0; i < 3; i++) expect(got[i]).toBeCloseTo(want[i], 5)
  }
  closeTriple([ox, oy, oz], [0, 0, 1])
  closeTriple([xx, xy, xz], [0, 1, 0])
  closeTriple([yx, yy, yz], [1, 0, 0])

  // Switch to Rectangle, lock the idle plane to the frame's red (X) axis
  // (ArrowRight = axis 0), and draw with a typed exact size. The click
  // lands on open ground well clear of the box, so its snap resolves via
  // plain ray∩ground rather than the box's own geometry — the point itself
  // doesn't matter, only that the resulting plane's normal is the FRAME's
  // red direction (0,1,0), a vertical plane, nothing like the ground.
  await page.keyboard.press('r')
  await page.keyboard.press('ArrowRight')
  await clickWorld(page, ctx, 3, 3, 0)
  // The locked plane is y ≈ 3 (fixed) with x/z free — move within it (NOT
  // varying y) so the rubber-band preview lands in-plane, mirroring the
  // ground-mode "move diagonally from the anchor" idiom used elsewhere.
  await page.mouse.move(px(ctx, 4, 3, 1).x, px(ctx, 4, 3, 1).y)
  await page.keyboard.type('1,1')
  await page.keyboard.press('Enter')

  const sketchIds = await page.evaluate(() => window.__hew_test!.getSketchIds())
  expect(sketchIds).toHaveLength(1)
  const plane = await page.evaluate(
    (s) => window.__hew_test!.getSketchPlane(s),
    sketchIds[0],
  )
  expect(plane).not.toBeNull()
  const [, , , nx, ny, nz] = plane as number[]
  // The drawn sketch's plane normal matches the MOVED frame's red axis
  // (0,1,0) — not the world ground plane's (0,0,1).
  expect(Math.abs(nx)).toBeCloseTo(0, 6)
  expect(Math.abs(ny)).toBeCloseTo(1, 6)
  expect(Math.abs(nz)).toBeCloseTo(0, 6)

  // View ▸ Reset Drawing Axes puts the frame back to world identity.
  await resetAxesFromMenu(page)
  const afterReset = await page.evaluate(() => window.__hew_test!.getDrawingAxes())
  expect(afterReset).toEqual(WORLD_IDENTITY)
})

test('Drawing Axes tool: Escape steps back one stage instead of fully cancelling', async ({ page }) => {
  const ctx = await setup(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  // Captured AFTER drawBox (itself an undoable commit) — this test's own
  // claim is narrower: the Axes GESTURE below commits nothing, not that the
  // document has no history at all.
  const hash0 = await page.evaluate(() => window.__hew_test!.getStateHash())

  await activateFromToolsMenu(page, 'Drawing Axes')
  await clickWorld(page, ctx, 0, 0, 1) // origin picked
  await clickWorld(page, ctx, 0, 2, 1) // X picked — awaiting the Y (green) click

  await page.keyboard.press('Escape') // steps back to origin-picked, not idle
  await page.keyboard.press('Escape') // steps back to idle

  // Neither Escape committed anything — the frame and the document hash
  // are both untouched.
  const axes = await page.evaluate(() => window.__hew_test!.getDrawingAxes())
  expect(axes).toEqual(WORLD_IDENTITY)
  expect(await page.evaluate(() => window.__hew_test!.getStateHash())).toBe(hash0)
})
