import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Input-pipeline E2E — the first tests that drive Hew the way a user does:
 * real keyboard shortcuts, real pointer events on the WebGL canvas (through
 * raycast + snap), and real typed-VCB entry. This is docs/DEVELOPMENT.md strategy 2
 * ("pixel interaction") made live: `harness.setCamera` pins the app camera
 * to CAMERA below, `buildViewProjection` + `worldToPagePixel` project known
 * world points to page pixels, and `page.mouse` clicks land there.
 *
 * Scope discipline (the pyramid, docs/DEVELOPMENT.md): geometry correctness stays in
 * kernel tests, and per-op behavior in tools.spec.ts's semantic layer. What
 * ONLY this file proves is the wiring between them — that a keypress
 * activates the tool, a canvas click reaches the right world point, and the
 * typed buffer commits through the tool's VCB. Notably this covers the Arc
 * typed-entry (), which its commit shipped
 * unit-test-only "since the harness has no pointer+keyboard simulation
 * path" — Playwright is that path.
 *
 * The harness (`window.__hew_test`) is used here ONLY to pin the camera and
 * to *observe* (state hash, object count, world-ray probes) — never to
 * mutate the document (except the undo/redo spec, whose subject is the
 * keyboard binding itself, not object creation).
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

// The pinned pose every projection in this file derives from. Same shape the
// harness setCamera takes; near/far only affect NDC z (see the helper doc).
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

/** Page pixel for a world point under the pinned camera; throws on a miss so
 * a bad projection fails loudly instead of clicking a random pixel. */
function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

async function setup(page: Page): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
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

/** Click the canvas at the pixel where `world` renders. A `mouse.move` first,
 * so tools see a pointermove (snap resolve + preview) before the down — the
 * same order a human hand produces. */
async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

// ---------------------------------------------------------------------------
// The full journey: R → click → typed dims → P → click → typed depth = solid
// ---------------------------------------------------------------------------

test('journey: Rectangle (typed 2,2) then Push/Pull (typed 1) builds a real solid', async ({
  page,
}) => {
  const ctx = await setup(page)

  // Rectangle tool via its bare-letter shortcut.
  await page.keyboard.press('r')

  // Anchor at the origin, rubber-band toward +X/+Y (gives the typed dims
  // their sign), then let the VCB take over: "2,2" ⏎.
  await clickWorld(page, ctx, 0, 0, 0)
  await page.mouse.move(px(ctx, 1.5, 1.5, 0).x, px(ctx, 1.5, 1.5, 0).y)
  await page.keyboard.type('2,2')
  // The docked VCB (MeasurementBox) mirrors the typed buffer live.
  await expect(page.getByText('2,2 m')).toBeVisible()
  await page.keyboard.press('Enter')

  // The commit happened through the real tool: a sketch exists, no object yet.
  const afterRect = await page.evaluate(() => ({
    count: window.__hew_test!.getObjectCount(),
    err: window.__hew_test!.getLastError(),
  }))
  expect(afterRect.count).toBe(0)
  expect(afterRect.err).toBeNull()

  // Push/Pull via shortcut; first click grabs the region under the cursor,
  // move upward for a positive (pull) sign, then type the exact depth.
  await page.keyboard.press('p')
  await clickWorld(page, ctx, 1, 1, 0)
  await page.mouse.move(px(ctx, 1, 1, 1.5).x, px(ctx, 1, 1, 1.5).y)
  await page.keyboard.type('1')
  await expect(page.getByText('Push depth')).toBeVisible()
  await page.keyboard.press('Enter')

  // One solid Object now exists (solids-first: the extrude created it).
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  // Geometry probes prove the typed values, not just "something changed":
  // the typed depth is exact (top at z=1), the footprint is where the clicks
  // put it. Probes use world rays (pickFace) — pixel-error tolerant.
  const probes = await page.evaluate(() => {
    const h = window.__hew_test!
    return {
      inside: h.pickFace([1, 1, 10], [0, 0, -1]), // down into the footprint
      outside: h.pickFace([5, 5, 10], [0, 0, -1]), // well clear of it
      sideBelowTop: h.pickFace([10, 1, 0.5], [-1, 0, 0]), // z=0.5 < height
      sideAboveTop: h.pickFace([10, 1, 1.5], [-1, 0, 0]), // z=1.5 > height 1
    }
  })
  expect(probes.inside).not.toBeNull()
  expect(probes.outside).toBeNull()
  expect(probes.sideBelowTop).not.toBeNull()
  expect(probes.sideAboveTop).toBeNull()
})

// ---------------------------------------------------------------------------
// Arc typed VCB entry — the ce22c70 previously unit-test-only
// ---------------------------------------------------------------------------

test('Arc: two chord clicks + typed bulge ⏎ commits through the live VCB', async ({ page }) => {
  const ctx = await setup(page)
  const hash0 = await page.evaluate(() => window.__hew_test!.getStateHash())

  await page.keyboard.press('a')
  await expect(
    page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Arc' }),
  ).toHaveAttribute('aria-checked', 'true')

  // Chord A → B on the ground — deliberately slanted OFF the world axes:
  // a chord lying on an axis line keeps the bulge cursor axis-snapped back
  // onto the chord (sagitta 0), so the tool correctly refuses it as flat.
  // Then hover the CCW side so the typed |sagitta| takes its sign from the
  // last-seen bulge side (the ArcTool doc contract).
  await clickWorld(page, ctx, 0.2, 0.3, 0)
  await clickWorld(page, ctx, 2.1, 1.1, 0)
  await page.mouse.move(px(ctx, 0.8, 1.5, 0).x, px(ctx, 0.8, 1.5, 0).y)
  // The live bulge preview reports the arc radius — proof the pointer is
  // driving the real rubber-band, not just the typed buffer.
  await expect(page.getByText(/^R /)).toBeVisible()

  await page.keyboard.type('0.5')
  await expect(page.getByText('0.5 m')).toBeVisible() // VCB mirrors the buffer
  await page.keyboard.press('Enter')

  const after = await page.evaluate(() => ({
    hash: window.__hew_test!.getStateHash(),
    err: window.__hew_test!.getLastError(),
    count: window.__hew_test!.getObjectCount(),
  }))
  expect(after.err).toBeNull()
  expect(after.hash).not.toBe(hash0) // the arc's segments were committed
  expect(after.count).toBe(0) // an open arc alone creates no object
  // (No canUndo assert: sketch segments don't create scene-undo entries —
  // see the begin_ground_sketch note in tools.spec.ts's undo spec.)
})

// ---------------------------------------------------------------------------
// Escape cancels a gesture without touching the document
// ---------------------------------------------------------------------------

test('Escape mid-gesture: no document mutation is committed', async ({ page }) => {
  const ctx = await setup(page)
  const hash0 = await page.evaluate(() => window.__hew_test!.getStateHash())

  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0, 0, 0) // anchor only — nothing kernel-side yet
  await page.mouse.move(px(ctx, 1, 1, 0).x, px(ctx, 1, 1, 0).y)
  await page.keyboard.press('Escape')

  const after = await page.evaluate(() => ({
    hash: window.__hew_test!.getStateHash(),
    canUndo: window.__hew_test!.canUndo(),
  }))
  expect(after.hash).toBe(hash0)
  expect(after.canUndo).toBe(false)
})

// ---------------------------------------------------------------------------
// Selection by real canvas click (SelectTool ray-pick, not harness.selectObjects)
// ---------------------------------------------------------------------------

test('Select: clicking rendered geometry selects it; clicking empty ground clears', async ({
  page,
}) => {
  const ctx = await setup(page)
  const boxId = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))

  // Click the pixel where the box's top-face center (1,1,1) renders — the
  // SelectTool pick_face path, i.e. the click → raycast → selection wiring
  // that harness selectObjects() bypasses.
  await clickWorld(page, ctx, 1, 1, 1)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  const selection = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(selection[0]).toEqual({ kind: 'object', id: boxId })

  // The contextual dock followed the click-made selection.
  await expect(page.locator('.hew-dock')).toContainText('OBJECT')

  // Clicking far-away empty ground is a pick miss → selection clears.
  await clickWorld(page, ctx, -3, -3, 0)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 0)
  await expect(page.locator('.hew-dock')).toContainText('DRAW')
})

// ---------------------------------------------------------------------------
// Delete key — the dedicated always-on handler, not a menu action
// ---------------------------------------------------------------------------

test('Delete key: removes the click-selected object through the live handler', async ({
  page,
}) => {
  const ctx = await setup(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))

  await clickWorld(page, ctx, 1, 1, 1)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  await page.keyboard.press('Delete')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 0)
  // The selection emptied with the object, and the dock fell back to DRAW.
  expect(await page.evaluate(() => window.__hew_test!.getSelection())).toHaveLength(0)
  await expect(page.locator('.hew-dock')).toContainText('DRAW')
})

// ---------------------------------------------------------------------------
// Undo / redo through the real keyboard binding (not harness.undo())
// ---------------------------------------------------------------------------

test('Ctrl+Z undoes and Ctrl+Shift+Z redoes through the live keydown path', async ({ page }) => {
  await setup(page)

  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 0)

  // Uppercase 'Z': a physical Shift+Z produces `key === 'Z'` — pressing the
  // lowercase variant here would mask case-sensitivity bugs in the handler.
  await page.keyboard.press('Control+Shift+Z')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
})

// ---------------------------------------------------------------------------
// Group / Ungroup through the real keyboard binding (not the menu)
// ---------------------------------------------------------------------------

test('Ctrl+G groups the selection and Ctrl+Shift+G ungroups, through the live keydown path', async ({
  page,
}) => {
  await setup(page)

  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.drawBox([2, 0, 0], [3, 1, 0], 1)
    h.selectObjects(h.getObjectIds())
  })
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 2)

  await page.keyboard.press('Control+g')
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'group'
  })

  // Uppercase 'G' — a physical Shift+G produces `key === 'G'`; the lowercase
  // variant would mask case-sensitivity bugs (same rationale as redo above).
  await page.keyboard.press('Control+Shift+G')
  // Ungroup clears the selection; both objects are top-level again.
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 0)
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(2)
})

// ---------------------------------------------------------------------------
// Rotate: standing a drawn shape upright through the REAL gesture path.
// This exact flow shipped broken twice while the harness-driven equivalents
// stayed green (`rotateSketch` bypasses the tool's axis lock, VCB, and
// selection plan), so it earns a permanent real-pointer seat: arrow-key axis
// lock, two canvas clicks, typed degrees.
// ---------------------------------------------------------------------------

test('Rotate: X-locked 90° (arrow lock, two clicks, typed angle) stands a drawn rectangle upright, no error toast', async ({
  page,
}) => {
  const ctx = await setup(page)

  // A closed shape through the real Rectangle tool.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0, 0, 0)
  await page.mouse.move(px(ctx, 0.7, 0.7, 0).x, px(ctx, 0.7, 0.7, 0).y)
  await page.keyboard.type('1,1')
  await page.keyboard.press('Enter')

  const ids0 = await page.evaluate(() => window.__hew_test!.getSketchIds())
  expect(ids0).toHaveLength(1)
  const sketch = ids0[0]

  // Select the shape by clicking one of its EDGES — the shape-granularity
  // path: the transform plan resolves the edge to the island it rides with.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0.5, 0, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'sketch-edge'
  })

  // Rotate: lock the X axis with the arrow key (the tilt no ground hover
  // could infer), pivot at the shape's corner, reference point off the
  // axis, then the typed-VCB commit.
  await page.keyboard.press('q')
  await page.keyboard.press('ArrowRight')
  await clickWorld(page, ctx, 0, 0, 0)
  await clickWorld(page, ctx, 0, 1, 0)
  await page.keyboard.type('90')
  await page.keyboard.press('Enter')

  // The island stood up: every endpoint left the ground into the x–z plane
  // through the pivot. Wait on the geometry (commit → re-pull is async to
  // the keydown), then assert the full shape.
  await page.waitForFunction(
    (s) => {
      const lines = window.__hew_test!.getSketchLines(s)
      return lines.length > 0 && lines.some((_, i) => i % 3 === 2 && Math.abs(lines[i]) > 0.5)
    },
    sketch,
    { timeout: 10_000 },
  )
  const after = await page.evaluate(
    (s) => ({
      ids: window.__hew_test!.getSketchIds(),
      lines: window.__hew_test!.getSketchLines(s),
    }),
    sketch,
  )
  // Sole island → whole-sketch bake: same sketch, no detach, all 4 edges.
  expect(after.ids).toEqual([sketch])
  expect(after.lines).toHaveLength(24)
  let maxAbsY = 0
  let minZ = Infinity
  let maxZ = -Infinity
  for (let i = 0; i < after.lines.length; i += 3) {
    maxAbsY = Math.max(maxAbsY, Math.abs(after.lines[i + 1]))
    minZ = Math.min(minZ, after.lines[i + 2])
    maxZ = Math.max(maxZ, after.lines[i + 2])
  }
  expect(maxAbsY).toBeLessThan(1e-6) // upright in the y = 0 plane
  expect(maxZ - minZ).toBeCloseTo(1, 5) // the full 1 m side now spans height

  // And it committed cleanly — no error toast. [WouldRetopologize] is what
  // this flow used to surface; [PointOffPlane] is its kernel-level cause.
  await expect(page.getByText('[WouldRetopologize]')).toHaveCount(0)
  await expect(page.getByText('[PointOffPlane]')).toHaveCount(0)
})

test('Ctrl+G with a single object selected is a no-op — the keyboard path honors the ≥2-sibling gate', async ({
  page,
}) => {
  await setup(page)

  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.selectObjects(h.getObjectIds())
  })
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  const hashBefore = await page.evaluate(() => window.__hew_test!.getStateHash())

  // The Edit menu disables Group for a 1-node selection; the accelerator must
  // agree — before the handler-side gate, this silently made a 1-member group.
  await page.keyboard.press('Control+g')

  // Positive control: Ctrl+Shift+G on the (still object) selection is also
  // a no-op, then verify the document hash and selection never changed.
  await page.keyboard.press('Control+Shift+G')
  await expect
    .poll(async () => page.evaluate(() => window.__hew_test!.getStateHash()))
    .toBe(hashBefore)
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel).toHaveLength(1)
  expect(sel[0].kind).toBe('object')
})
