import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * follow-me-2 region interaction — the two maintainer defects, driven end to
 * end with REAL pointer input on his actual file (`follow-me-2.hew`): a
 * tabletop box with a rectangle and a circle sketch standing perpendicular on
 * it. The harness (`window.__hew_test`) is used ONLY to load the bytes, pin
 * the camera, and read back the selection.
 *
 * FIX A — a drawn sketch region is a hoverable face. Hovering the fill of the
 *   standing rectangle resolves an "On Face" inference cue (the region snaps
 *   like a solid's face and occludes what's behind it), instead of the ray
 *   passing through to the ground/box beneath.
 * FIX B — an interior click on a closed region selects the whole island, not
 *   its nearest edge. Clicking the standing rectangle's fill selects the
 *   rectangle island (kind 'sketch-island'); before the fix the same click at
 *   this zoom selected a single boundary segment ('sketch-edge'). The circle
 *   already worked and still does.
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

const FIXTURE = fileURLToPath(new URL('./fixtures/follow-me-2.hew', import.meta.url))

function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

/** Wait for the harness, then aim the camera and return the projection ctx. */
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

/** Load the maintainer's file, then aim the camera. */
async function setup(page: Page, camera: CameraParams): Promise<Ctx> {
  await ready(page)
  const bytes = Array.from(readFileSync(FIXTURE))
  // `load` routes through the app's real File→Open path and throws if the
  // load is rejected.
  await page.evaluate((arr) => window.__hew_test!.load(arr), bytes)
  return aim(page, camera)
}

// Face-on view of the standing rectangle (plane y≈0.14442, normal +Y), at a
// distance where the rectangle's own edges sit inside the click pick-cone but
// outside the 8px hover snap radius — the maintainer's zoom.
const RECT_CENTER = { x: 0.11, y: 0.14441909951924115, z: 0.015 }
const RECT_CAMERA: CameraParams = {
  position: { x: 0.11, y: 1.0, z: 0.015 },
  target: RECT_CENTER,
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.01,
  far: 100,
}

// A closer face-on view of the rectangle: it fills more of the screen, so the
// fill centre sits well clear (≫ the 8px hover snap radius) of every edge —
// the hover can only resolve to the region's own face.
const RECT_CAMERA_NEAR: CameraParams = {
  position: { x: 0.11, y: 0.4, z: 0.015 },
  target: RECT_CENTER,
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.01,
  far: 100,
}

test('FIX A: hovering the standing rectangle fill resolves an On Face cue (not a pass-through)', async ({
  page,
}) => {
  const ctx = await setup(page, RECT_CAMERA_NEAR)
  // No OBJECT face lies under this ray (the box is not behind the fill here),
  // so any On Face cue can ONLY come from the sketch region itself — ruling
  // out a solid face behind it. The ray is straight down -Y through the fill.
  const objHit = await page.evaluate(
    () => window.__hew_test!.pickFace([0.11, 0.4, 0.015], [0, -1, 0]),
  )
  expect(objHit).toBeNull()

  // Select tool is live by default; hovering resolves the inference cue.
  const p = px(ctx, RECT_CENTER.x, RECT_CENTER.y, RECT_CENTER.z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.move(p.x + 1, p.y) // a second move so the app re-resolves
  // The cursor-anchored chip reads "On Face" — the region registered a face
  // the cursor snapped to, rather than the ray passing through it.
  await expect(page.getByText('On Face', { exact: true })).toBeVisible()
})

test('FIX B: clicking the standing rectangle fill selects the whole island, not an edge', async ({
  page,
}) => {
  const ctx = await setup(page, RECT_CAMERA)
  const p = px(ctx, RECT_CENTER.x, RECT_CENTER.y, RECT_CENTER.z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
  // The whole rectangle island is selected — NOT a single boundary segment.
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'sketch-island'
  })
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel[0].kind).toBe('sketch-island')
})

test('FIX B: clicking the standing circle fill still selects the whole island', async ({
  page,
}) => {
  // A deep-interior circle point clear of the rectangle's x-range AND above
  // the tabletop box (z=0.025 > the box top z=0.015), so the click ray meets
  // ONLY the circle region — no rectangle behind, no solid in the way.
  const target = { x: 0.09, y: 0.08181245342560017, z: 0.025 }
  const ctx = await setup(page, {
    position: { x: 0.09, y: 0.4, z: 0.025 },
    target,
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.01,
    far: 100,
  })
  // No solid under this ray — the click can only resolve to the circle region.
  const objHit = await page.evaluate(
    () => window.__hew_test!.pickFace([0.09, 0.4, 0.025], [0, -1, 0]),
  )
  expect(objHit).toBeNull()

  const p = px(ctx, target.x, target.y, target.z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'sketch-island'
  })
})

test('FINDING 1: clicking a partition line SHARED between two regions selects the EDGE, not a region', async ({
  page,
}) => {
  // Empty scene viewed straight down the ground plane.
  await ready(page)
  const ctx = await aim(page, {
    position: { x: 1, y: 0.5, z: 5 },
    target: { x: 1, y: 0.5, z: 0 },
    up: { x: 0, y: 1, z: 0 },
    fovDeg: 45,
    near: 0.01,
    far: 100,
  })
  // A 2×1 rectangle split at x=1 into two 1×1 regions of ONE sketch — the
  // partition edge has region-interior on BOTH sides (no exterior gap). The
  // point chain is an Eulerian traversal of the split-rect graph (each edge
  // drawn once; the last segment is the partition), so no edge is retraced.
  const res = await page.evaluate(() =>
    window.__hew_test!.drawLineChain([
      [1, 0, 0],
      [2, 0, 0],
      [2, 1, 0],
      [1, 1, 0],
      [0, 1, 0],
      [0, 0, 0],
      [1, 0, 0],
      [1, 1, 0],
    ]),
  )
  // The final sketch holds exactly two regions sharing the partition (the
  // returned list is cumulative — the outer region forms, then splits).
  const finalRegions = await page.evaluate(
    (sketch) => window.__hew_test!.getSketchRegionCount(sketch),
    res.sketch,
  )
  expect(finalRegions).toBe(2)

  // Click ON the partition line (x=1), off its midpoint. Pre-fix the region
  // won and the partition was unselectable; now the tight-aperture edge wins.
  const p = px(ctx, 1, 0.3, 0)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'sketch-edge'
  })

  // And it stays deletable-to-merge: deleting the selected partition leaves a
  // single region (the two halves merged).
  await page.keyboard.press('Delete')
  await page.waitForFunction(
    (sketch) => window.__hew_test!.getSketchRegionCount(sketch) === 1,
    res.sketch,
  )
})

test('DELTA-FIX: dead-centre click on a solid whose top face lies on the WORLD ORIGIN selects the object (not clear)', async ({
  page,
}) => {
  // The origin registers as a provenance-less Endpoint (the strongest kind),
  // outranking the solid's OnFace under it. The click must still select the
  // solid — earlier it fell through to the sketch pickers and cleared.
  await ready(page)
  const ctx = await aim(page, {
    position: { x: 0, y: 0, z: 8 },
    target: { x: 0, y: 0, z: 0 },
    up: { x: 0, y: 1, z: 0 },
    fovDeg: 45,
    near: 0.01,
    far: 100,
  })
  // A rectangle spanning the origin, extruded DOWN into a pit → its top face
  // lies on z=0 through the origin, unoccluded from above.
  const { sketch, region } = await page.evaluate(() =>
    window.__hew_test!.drawRectangle([-1, -1, 0], [1, 1, 0]),
  )
  await page.evaluate(
    ([s, r]) => window.__hew_test!.extrudeRegion(s, r, -1),
    [sketch, region] as const,
  )
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  const p = px(ctx, 0, 0, 0)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'object'
  })
})

test('DELTA-2 FIX: dragging a bare OPEN sketch line arms a Move of the line ISLAND, not the solid beneath it', async ({
  page,
}) => {
  // A bare sketch edge carries sketch provenance but no object; the drag arm
  // was missing that branch, so it fell through to a raw pick_face and armed a
  // Move of whatever solid the ray crossed (even one beyond the far plane).
  await ready(page)
  const ctx = await aim(page, {
    position: { x: 0, y: 0, z: 8 },
    target: { x: 0, y: 0, z: 0 },
    up: { x: 0, y: 1, z: 0 },
    fovDeg: 45,
    near: 0.01,
    far: 100,
  })
  // A solid directly under the ground line (its top face at z=0 spans the
  // line): pre-fix, dragging the line would arm a Move of THIS box.
  const box = await page.evaluate(() => window.__hew_test!.drawRectangle([-2, -2, 0], [2, 2, 0]))
  await page.evaluate(
    ([s, r]) => window.__hew_test!.extrudeRegion(s, r, -1),
    [box.sketch, box.region] as const,
  )
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
  // A BARE open polyline on z=0 that never closes into a region.
  const line = await page.evaluate(() =>
    window.__hew_test!.drawLineChain([
      [-1, 0, 0],
      [1, 0, 0],
    ]),
  )
  const islands = await page.evaluate((s) => window.__hew_test!.getSketchIslands(s), line.sketch)
  expect(islands.length).toBe(1)

  // Press ON the line (a sketch-edge snap) and drag past the 5px threshold.
  const p = px(ctx, 0.3, 0, 0)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.move(p.x + 24, p.y)
  await page.mouse.move(p.x + 24, p.y + 6)
  // The drag armed a Move of the LINE — a sketch-edge (which the transform
  // layer moves at island granularity), matching what a CLICK selects — NOT
  // the box beneath it (which would select as 'object').
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'sketch-edge'
  })
  await page.mouse.up()
})
