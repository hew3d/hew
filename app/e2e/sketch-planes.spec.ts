import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Sketches on any plane — end-to-end (the sketch-planes design, all four
 * phases). Scripts the whole feature in one real-input journey (strategy 2,
 * docs/DEVELOPMENT.md — pixel interaction through the pinned camera):
 *
 *   1. Draw a rectangle on the ground with the REAL Rectangle tool.
 *   2. Rotate that sketch 90° onto a vertical plane — `rotateSketch` (the
 *      harness's Rodrigues bake, the same one `transform_sketch` commits for
 *      a real Rotate-tool drag; driven directly here per the harness's own
 *      convention in tools.spec.ts's Follow Me fixtures — a UI-driven Rotate
 *      of a SKETCH, not an object, adds camera/selection machinery that
 *      isn't this spec's subject).
 *   3. Hover-draw a line with the REAL Line tool between two points already
 *      on the vertical sketch's boundary — sketch mode (Phase 2) hover-adopts
 *      its plane, and the segment splits the rectangle into two regions.
 *   4. Idle-lock a plane with an arrow key (Phase 3) and draw a NEW rectangle
 *      in empty space with the REAL Rectangle tool — a second, distinct
 *      non-ground sketch.
 *   5. Push/Pull one of the vertical regions into a solid with the REAL
 *      Push/Pull tool (a click + typed exact distance — Phase 1's per-region
 *      plane-normal fix, exercised for real).
 *
 * Assertions are logical (`getSketchIds`, `getSketchRegionCount`,
 * `getSketchLines`, `getObjectCount`, `getLastError`) via the harness, per
 * the pyramid — geometry correctness stays in kernel/unit tests; this proves
 * the real input → tool → kernel wiring for a plane the ground-only app
 * never had to route through before.
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

/** Click the canvas at the pixel where `world` renders — a `mouse.move` first
 * so tools see a pointermove (snap resolve + preview) before the down, the
 * same order a human hand produces. */
async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

// A single wide oblique view that keeps every point this spec touches in
// frame: the ground rectangle near the origin, that same rectangle standing
// vertical after the rotate, the empty-space locked-plane rectangle a few
// meters over, and everything in between.
const CAMERA: CameraParams = {
  position: { x: 11, y: -9, z: 9 },
  target: { x: 3, y: 1, z: 1 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 55,
  near: 0.1,
  far: 1000,
}

test('sketches on any plane: draw, rotate onto vertical, hover-split, idle-lock draw, push/pull', async ({
  page,
}) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // ---- 1. Ground rectangle, via the REAL Rectangle tool (bare 'r') -------
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0, 0, 0)
  await clickWorld(page, ctx, 2, 2, 0)

  const afterRect = await page.evaluate(() => ({
    sketchIds: window.__hew_test!.getSketchIds(),
    objectCount: window.__hew_test!.getObjectCount(),
    lastError: window.__hew_test!.getLastError(),
  }))
  expect(afterRect.lastError).toBeNull()
  expect(afterRect.sketchIds).toHaveLength(1)
  expect(afterRect.objectCount).toBe(0)
  const wallSketch = afterRect.sketchIds[0]

  // ---- 2. Rotate it 90° about the X axis through the origin: the bottom
  // edge (0,0,0)-(2,0,0) stays put, the rectangle stands up onto the y=0
  // plane, spanning x∈[0,2], z∈[0,2] (Rodrigues about X: (x,y,0) → (x,0,y)).
  await page.evaluate(
    (sketch) => window.__hew_test!.rotateSketch(sketch, 90, [1, 0, 0], [0, 0, 0]),
    wallSketch,
  )

  const wallLines = await page.evaluate(
    (sketch) => window.__hew_test!.getSketchLines(sketch),
    wallSketch,
  )
  // Every vertex of the rotated sketch now sits at y = 0 (a non-ground
  // plane) — the tell that the rotate actually landed the rectangle
  // vertical rather than leaving it flat.
  for (let i = 1; i < wallLines.length; i += 3) {
    expect(wallLines[i]).toBeCloseTo(0, 9)
  }
  expect(Math.max(...[1, 4, 7, 10].map((i) => wallLines[i - 1]))).toBeGreaterThan(1) // some x > 1
  expect(Math.max(...[3, 6, 9, 12].map((i) => wallLines[i - 1]))).toBeGreaterThan(1) // some z > 1

  // ---- 3. Hover-draw a line with the REAL Line tool, splitting the wall --
  // Both endpoints sit exactly on the wall's own boundary (the bottom and
  // top edges' midpoints) — the first click hover-adopts the wall's plane
  // (Phase 2 sketch mode; a top-level, non-ground `pick_sketch` hit), and
  // because both ends already touch the existing boundary, the ONE segment
  // immediately splits the rectangle into two regions.
  await page.keyboard.press('l')
  await clickWorld(page, ctx, 1, 0, 0)
  await clickWorld(page, ctx, 1, 0, 2)
  // LineTool chains forward after a non-closing commit, or ends the chain
  // outright if the commit closed a region — either way, Escape returns to
  // idle without discarding what was already committed.
  await page.keyboard.press('Escape')

  const afterSplit = await page.evaluate(
    (sketch) => ({
      sketchIds: window.__hew_test!.getSketchIds(),
      regionCount: window.__hew_test!.getSketchRegionCount(sketch),
      lastError: window.__hew_test!.getLastError(),
    }),
    wallSketch,
  )
  expect(afterSplit.lastError).toBeNull()
  // No new sketch — the line landed in the SAME sketch it hover-adopted.
  expect(afterSplit.sketchIds).toHaveLength(1)
  expect(afterSplit.regionCount).toBe(2)

  // ---- 4. Idle-lock a plane (Phase 3) and draw a new rectangle in empty
  // space. ArrowLeft locks the future plane's normal to green/Y — the same
  // axis the wall happens to use, but through a DIFFERENT point (y=1 here
  // vs. the wall's y=0), so it mints its own sketch on its own plane rather
  // than reusing the wall's.
  await page.keyboard.press('r')
  await page.keyboard.press('ArrowLeft')
  await clickWorld(page, ctx, 4, 1, 0)
  await clickWorld(page, ctx, 6, 1, 2)

  const afterLockDraw = await page.evaluate(() => ({
    sketchIds: window.__hew_test!.getSketchIds(),
    objectCount: window.__hew_test!.getObjectCount(),
    lastError: window.__hew_test!.getLastError(),
  }))
  expect(afterLockDraw.lastError).toBeNull()
  expect(afterLockDraw.sketchIds).toHaveLength(2) // the wall + the new locked-plane sketch
  expect(afterLockDraw.objectCount).toBe(0)
  const lockSketch = afterLockDraw.sketchIds.find((id) => id !== wallSketch)
  if (lockSketch === undefined) throw new Error('no new sketch minted by the idle-locked draw')

  const lockLines = await page.evaluate(
    (sketch) => window.__hew_test!.getSketchLines(sketch),
    lockSketch,
  )
  // Every vertex of the NEW sketch sits at y = 1 — the locked plane's offset,
  // distinct from both the ground (y is non-zero) and the wall (y = 0).
  // (Precision is pixel-projection-limited here, unlike the exact-affine
  // `rotateSketch` check above — these points came through real mouse
  // clicks resolved via the snap service, not a direct kernel call.)
  for (let i = 1; i < lockLines.length; i += 3) {
    expect(Math.abs(lockLines[i] - 1)).toBeLessThan(0.01)
  }

  // ---- 5. Push/Pull one of the wall's two split regions into a solid ----
  // A click inside the LEFT half's fill (x < 1, so it can't land on the
  // x = 1 partition line) enters drag mode; typing an exact distance with no
  // drag extrudes OUTWARD along the region's own plane normal (Phase 1's
  // sketch_plane fix) regardless of camera angle — see PushPullTool's
  // `_commitFromTyped` no-drag-defaults-outward rule.
  await page.keyboard.press('p')
  await clickWorld(page, ctx, 0.5, 0, 1)
  await page.keyboard.type('1')
  await expect(page.getByText('Push depth')).toBeVisible()
  await page.keyboard.press('Enter')

  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  const final = await page.evaluate(() => ({
    objectCount: window.__hew_test!.getObjectCount(),
    lastError: window.__hew_test!.getLastError(),
  }))
  expect(final.lastError).toBeNull()
  expect(final.objectCount).toBe(1)

  // The extrusion grew along the wall's normal (±Y), not Z — probe with a
  // ray straight down -Y through the middle of the pushed region; it must
  // hit the new solid's face.
  const probe = await page.evaluate(() => window.__hew_test!.pickFace([0.5, 3, 1], [0, -1, 0]))
  expect(probe).not.toBeNull()
})
