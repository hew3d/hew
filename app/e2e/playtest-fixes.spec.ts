import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Real-gesture regression pins for the maintainer-playtest findings that
 * shipped harness-green but gesture-broken:
 *
 *  1. A drawn (unextruded) circle offers its exact center (and quadrants)
 *     to inference — in every context, including a detached upright sketch.
 *  2. Move+Alt copies a sketch selection (it silently fell back to a plain
 *     move before).
 *  3. Tape Measure drags a parallel guide off a world axis (the analytic
 *     axis snap is the source; the clamped render geometry is irrelevant).
 *
 * Everything here drives real pointer events on the canvas; the harness is
 * used only to pin the camera and read state back (docs/DEVELOPMENT.md §6:
 * raw pointer tests exist precisely for screen-to-world wiring).
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

async function setup(page: Page): Promise<Ctx> {
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

/** Hover `p` until the inference chip shows `label`. Each poll wiggles the
 * pointer one pixel and back so a fresh pointermove resolves against the
 * CURRENT camera — a single move can race the async `setCamera` apply. */
async function hoverUntilCue(
  page: Page,
  p: { x: number; y: number },
  label: string,
): Promise<void> {
  await expect
    .poll(
      async () => {
        await page.mouse.move(p.x + 1, p.y)
        await page.mouse.move(p.x, p.y)
        return page.getByText(label, { exact: true }).count()
      },
      { timeout: 10_000 },
    )
    .toBeGreaterThan(0)
}

/** Unique endpoints (1e-9 merge) of a sketch's line soup, as [x,y,z][]. */
function uniquePoints(lines: number[]): [number, number, number][] {
  const pts: [number, number, number][] = []
  for (let i = 0; i < lines.length; i += 3) {
    const p: [number, number, number] = [lines[i], lines[i + 1], lines[i + 2]]
    if (!pts.some((q) => Math.hypot(q[0] - p[0], q[1] - p[1], q[2] - p[2]) < 1e-9)) pts.push(p)
  }
  return pts
}

/** Mean of a point set — for a full regular circle ring this reproduces the
 * analytic center up to the f32 readback quantization (~1e-8), tight enough
 * to distinguish a center-snap landing from a raw ground click (mm off). */
function mean(pts: [number, number, number][]): [number, number, number] {
  const s = pts.reduce((a, p) => [a[0] + p[0], a[1] + p[1], a[2] + p[2]], [0, 0, 0])
  return [s[0] / pts.length, s[1] / pts.length, s[2] / pts.length]
}

// ---------------------------------------------------------------------------
// Finding 1 — a drawn circle's exact center snaps before any extrusion
// ---------------------------------------------------------------------------

test('drawn 10 cm circle: Center cue on hover, Line click lands exactly at the analytic center', async ({
  page,
}) => {
  const ctx = await setup(page)

  // Draw the circle through the real Circle tool: click the center, drag the
  // rubber band OFF the cardinal directions (the facet phase follows it, so
  // no vertex lands on a quadrant point), then type the exact 10 cm radius.
  await page.keyboard.press('c')
  await clickWorld(page, ctx, 1, 1, 0)
  await page.mouse.move(px(ctx, 1.31, 1.4, 0).x, px(ctx, 1.31, 1.4, 0).y)
  await page.keyboard.type('0.1')
  await page.keyboard.press('Enter')

  const sketchIds = await page.evaluate(() => window.__hew_test!.getSketchIds())
  expect(sketchIds).toHaveLength(1)
  const sketch = sketchIds[0]
  const ring = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  expect(ring.length).toBeGreaterThanOrEqual(24)
  const center = mean(ring)
  // Sanity: the ring is the typed 10 cm radius around its center.
  for (const p of ring) {
    expect(Math.hypot(p[0] - center[0], p[1] - center[1])).toBeCloseTo(0.1, 6)
  }

  // Move in close (a 10 cm circle is tiny from the default pose) and hover
  // the center with the Move tool: the Center cue chip must appear.
  const close: CameraParams = {
    position: { x: center[0] + 1.1, y: center[1] + 0.85, z: 1.0 },
    target: { x: center[0], y: center[1], z: 0 },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.1,
    far: 1000,
  }
  const cctx = await pinCamera(page, close)
  await page.keyboard.press('m')
  const cpx = px(cctx, center[0], center[1], 0)
  await hoverUntilCue(page, cpx, 'Center')

  // A quadrant point of the exact circle shows the Quadrant cue. Pick the
  // cardinal farthest from any facet vertex so Endpoint can't outrank it.
  const cardinals: [number, number, number][] = [
    [center[0] + 0.1, center[1], 0],
    [center[0] - 0.1, center[1], 0],
    [center[0], center[1] + 0.1, 0],
    [center[0], center[1] - 0.1, 0],
  ]
  const clearest = cardinals
    .map((c) => ({
      c,
      d: Math.min(...ring.map((p) => Math.hypot(p[0] - c[0], p[1] - c[1], p[2] - c[2]))),
    }))
    .sort((a, b) => b.d - a.d)[0].c
  // Zoom right onto the cardinal: the nearest facet vertex can sit as
  // close as ~6 mm to it (the facet phase follows the rubber-band pixel,
  // which differs per engine), and inside the pick cone that Endpoint
  // outranks Quadrant — so get close enough that 6 mm is many pixels.
  const quadCam: CameraParams = {
    position: { x: clearest[0] + 0.18, y: clearest[1] + 0.13, z: 0.16 },
    target: { x: clearest[0], y: clearest[1], z: 0 },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.1,
    far: 1000,
  }
  const qctx = await pinCamera(page, quadCam)
  const qpx = px(qctx, clearest[0], clearest[1], clearest[2])
  await hoverUntilCue(page, qpx, 'Quadrant')
  // Back to the center-hover pose for the Line-tool landing below.
  await pinCamera(page, close)

  // Line tool: first click on the hovered center must land EXACTLY at the
  // analytic center — not at the raw pick-ray/ground intersection, which is
  // millimeters off at this zoom. Second click somewhere on the ground,
  // then Escape ends the chain.
  await page.keyboard.press('l')
  await hoverUntilCue(page, cpx, 'Center')
  await page.mouse.down()
  await page.mouse.up()
  await clickWorld(page, cctx, center[0] + 0.35, center[1] - 0.05, 0)
  await page.keyboard.press('Escape')

  const after = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  const nearest = Math.min(
    ...after.map((p) => Math.hypot(p[0] - center[0], p[1] - center[1], p[2] - center[2])),
  )
  // getSketchLines reads back Float32 buffers, so both the ring mean and the
  // landed endpoint carry ~1e-8 quantization; a missed snap (raw ground
  // click) would be off by MILLIMETERS at this zoom.
  expect(nearest).toBeLessThan(1e-6)
})

test('detached upright sketch keeps its circle center snap (Center cue after out-of-plane rotate)', async ({
  page,
}) => {
  const ctx = await setup(page)

  // Two islands on the shared ground sketch: a rectangle and a circle.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0, 0, 0)
  await page.mouse.move(px(ctx, 0.6, 0.6, 0).x, px(ctx, 0.6, 0.6, 0).y)
  await page.keyboard.type('1,1')
  await page.keyboard.press('Enter')

  await page.keyboard.press('c')
  await clickWorld(page, ctx, 2.5, 1.5, 0)
  await page.mouse.move(px(ctx, 2.85, 1.72, 0).x, px(ctx, 2.85, 1.72, 0).y)
  await page.keyboard.type('0.2')
  await page.keyboard.press('Enter')

  const ids0 = await page.evaluate(() => window.__hew_test!.getSketchIds())
  expect(ids0).toHaveLength(1)

  // Select the circle by clicking its rim, then rotate it 90° about a
  // ground X-line through a point near the rim: a subset island leaving the
  // plane detaches into its own standing sketch.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 2.7, 1.5, 0)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  await page.keyboard.press('q')
  await page.keyboard.press('ArrowRight')
  await clickWorld(page, ctx, 2.5, 2.0, 0)
  await clickWorld(page, ctx, 2.5, 2.8, 0)
  await page.keyboard.type('90')
  await page.keyboard.press('Enter')

  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 2, null, {
    timeout: 10_000,
  })
  const ids1 = await page.evaluate(() => window.__hew_test!.getSketchIds())
  const detached = ids1.find((id) => id !== ids0[0])!
  const ring = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), detached),
  )
  const center = mean(ring)
  expect(Math.abs(center[2])).toBeGreaterThan(0.05) // it really stood up

  // Hover the standing circle's center: the cue must appear there too.
  const close: CameraParams = {
    position: { x: center[0] + 0.9, y: center[1] - 1.3, z: center[2] + 0.7 },
    target: { x: center[0], y: center[1], z: center[2] },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.1,
    far: 1000,
  }
  const cctx = await pinCamera(page, close)
  await page.keyboard.press('m')
  const cpx = px(cctx, center[0], center[1], center[2])
  await hoverUntilCue(page, cpx, 'Center')
})

// ---------------------------------------------------------------------------
// Finding 2 — Move+Alt copies a sketch selection (circle → two circles)
// ---------------------------------------------------------------------------

test('Move+Alt on a drawn circle copies it 8 cm along X: two true circles, one undo step', async ({
  page,
}) => {
  const ctx = await setup(page)

  // A 10 cm circle through the real Circle tool.
  await page.keyboard.press('c')
  await clickWorld(page, ctx, 1, 1, 0)
  await page.mouse.move(px(ctx, 1.31, 1.4, 0).x, px(ctx, 1.31, 1.4, 0).y)
  await page.keyboard.type('0.1')
  await page.keyboard.press('Enter')

  const sketch = (await page.evaluate(() => window.__hew_test!.getSketchIds()))[0]
  const before = await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch)
  const ring = uniquePoints(before)
  const center = mean(ring)

  // Select the circle by clicking its rim.
  await page.keyboard.press(' ')
  const rim = ring[0]
  await clickWorld(page, ctx, rim[0], rim[1], rim[2])
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  // Move tool: base at the rim, tap Alt (durable copy), lock X, type the
  // exact 8 cm, Enter. (8 cm < the 20 cm diameter, so the copy OVERLAPS its
  // source — the replay goes through the ordinary sticky rules, which split
  // both circles at the crossings; both stay true curve chains.)
  await page.keyboard.press('m')
  await clickWorld(page, ctx, rim[0], rim[1], rim[2])
  await page.keyboard.press('Alt')
  await expect(page.getByText(/Copy ·/)).toBeVisible()
  await page.keyboard.press('ArrowRight')
  // Cursor to the +X side to give the typed distance its sign. The exact
  // spot is chosen OFF every world axis's screen projection: from this
  // camera, rays near the circle pass within centimeters of the Z axis
  // (underground, behind the ground plane), and that OnAxis candidate
  // would win the pick cone and project to x = 0 on the locked line,
  // flipping the sign.
  await page.mouse.move(px(ctx, rim[0] + 0.7, rim[1] - 0.5, 0).x, px(ctx, rim[0] + 0.7, rim[1] - 0.5, 0).y)
  await page.keyboard.type('0.08')
  await page.keyboard.press('Enter')

  // The copy landed: at least a second circle's worth of segments (more,
  // with the crossing splits), and every original ring vertex survives in
  // place — the source was copied, not moved.
  await page.waitForFunction(
    (args) => window.__hew_test!.getSketchLines(args.s).length >= args.n * 2,
    { s: sketch, n: before.length },
  )
  const after = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  for (const q of ring) {
    expect(
      after.some((p) => Math.hypot(q[0] - p[0], q[1] - p[1], q[2] - p[2]) < 1e-7),
    ).toBe(true)
  }

  // ONE undo removes the whole copy (the gesture is one step) and restores
  // the original exactly; redo brings it back for the identity check below.
  await page.keyboard.press('Control+z')
  await page.waitForFunction(
    (args) => window.__hew_test!.getSketchLines(args.s).length === args.n,
    { s: sketch, n: before.length },
  )
  const restored = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  expect(restored).toHaveLength(ring.length)
  expect(
    restored.every((p) => ring.some((q) => Math.hypot(q[0] - p[0], q[1] - p[1], q[2] - p[2]) < 1e-7)),
  ).toBe(true)
  await page.keyboard.press('Control+Shift+Z')
  await page.waitForFunction(
    (args) => window.__hew_test!.getSketchLines(args.s).length >= args.n * 2,
    { s: sketch, n: before.length },
  )

  // The copy is a TRUE circle at exactly +8 cm: its analytic center snaps,
  // and a Line click through that snap lands there to f32 readback
  // precision (a raw ground click would be mm off).
  const cc: [number, number, number] = [center[0] + 0.08, center[1], 0]
  // Tight pose: the copy's center sits only 2 cm inside the ORIGINAL
  // circle's rim (the circles overlap), and from arm's length that rim
  // vertex falls inside the pick cone where Endpoint outranks Center.
  const close: CameraParams = {
    position: { x: cc[0] + 0.35, y: cc[1] + 0.27, z: 0.32 },
    target: { x: cc[0], y: cc[1], z: 0 },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.1,
    far: 1000,
  }
  const cctx = await pinCamera(page, close)
  await page.keyboard.press('l')
  const cpx = px(cctx, cc[0], cc[1], cc[2])
  await hoverUntilCue(page, cpx, 'Center')
  await page.mouse.down()
  await page.mouse.up()
  await clickWorld(page, cctx, cc[0] + 0.12, cc[1] - 0.03, 0)
  await page.keyboard.press('Escape')
  const withLine = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  const nearest = Math.min(
    ...withLine.map((p) => Math.hypot(p[0] - cc[0], p[1] - cc[1], p[2] - cc[2])),
  )
  expect(nearest).toBeLessThan(1e-6)
})

test('Move+Alt copies a drawn circle UP the Z axis: a second sketch on the lifted plane, source stays, one undo', async ({
  page,
}) => {
  const ctx = await setup(page)

  // A 10 cm circle on the ground through the real Circle tool.
  await page.keyboard.press('c')
  await clickWorld(page, ctx, 1, 1, 0)
  await page.mouse.move(px(ctx, 1.31, 1.4, 0).x, px(ctx, 1.31, 1.4, 0).y)
  await page.keyboard.type('0.1')
  await page.keyboard.press('Enter')

  const sketch = (await page.evaluate(() => window.__hew_test!.getSketchIds()))[0]
  const before = await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch)
  const ring = uniquePoints(before)
  const center = mean(ring)

  // Select the circle by clicking its rim.
  await page.keyboard.press(' ')
  const rim = ring[0]
  await clickWorld(page, ctx, rim[0], rim[1], rim[2])
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  // Move tool: base at the rim, tap Alt (durable copy), lock Z (ArrowUp),
  // type an exact 50 cm height, Enter. Out-of-plane, so the copy detaches
  // onto its OWN new sketch on the lifted plane — the source is untouched.
  await page.keyboard.press('m')
  await clickWorld(page, ctx, rim[0], rim[1], rim[2])
  await page.keyboard.press('Alt')
  await expect(page.getByText(/Copy ·/)).toBeVisible()
  await page.keyboard.press('ArrowUp')
  // Nudge the cursor toward the +Z projection so the typed height takes the
  // upward sign (a downward drag would copy below the ground).
  await page.mouse.move(px(ctx, rim[0], rim[1], 0.5).x, px(ctx, rim[0], rim[1], 0.5).y)
  await page.keyboard.type('0.5')
  await page.keyboard.press('Enter')

  // A SECOND sketch appeared; the source sketch is unchanged (still on the
  // ground, still exactly the drawn ring).
  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 2, null, {
    timeout: 10_000,
  })
  const ids = await page.evaluate(() => window.__hew_test!.getSketchIds())
  const copySketch = ids.find((id) => id !== sketch)!
  const sourceAfter = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  expect(sourceAfter).toHaveLength(ring.length)
  expect(
    sourceAfter.every((p) => ring.some((q) => Math.hypot(q[0] - p[0], q[1] - p[1], q[2] - p[2]) < 1e-7)),
  ).toBe(true)

  // The copy is the same circle lifted +50 cm: its own ring on the z=0.5
  // plane, same radius around the lifted center.
  const copyRing = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), copySketch),
  )
  expect(copyRing.length).toBe(ring.length)
  const copyCenter = mean(copyRing)
  expect(copyCenter[2]).toBeCloseTo(0.5, 5) // it really lifted off the ground
  for (const p of copyRing) {
    expect(Math.hypot(p[0] - copyCenter[0], p[1] - copyCenter[1])).toBeCloseTo(0.1, 5)
    expect(p[2]).toBeCloseTo(0.5, 5)
  }

  // ONE undo removes just the copy (the new sketch is gone); the source
  // stays. Redo brings the copy back.
  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 1, null, {
    timeout: 10_000,
  })
  const afterUndo = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  expect(afterUndo).toHaveLength(ring.length) // source untouched by the undo
  await page.keyboard.press('Control+Shift+Z')
  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 2, null, {
    timeout: 10_000,
  })

  // The copy is a TRUE circle, not just facets: hover its lifted center and
  // the Center inference cue must appear there (only analytic curve rims
  // register a snappable center).
  const cCenter: [number, number, number] = [center[0], center[1], 0.5]
  const close: CameraParams = {
    position: { x: cCenter[0] + 0.9, y: cCenter[1] - 1.3, z: cCenter[2] + 0.7 },
    target: { x: cCenter[0], y: cCenter[1], z: cCenter[2] },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.1,
    far: 1000,
  }
  const cctx = await pinCamera(page, close)
  await page.keyboard.press('m')
  const cpx = px(cctx, cCenter[0], cCenter[1], cCenter[2])
  await hoverUntilCue(page, cpx, 'Center')
})

test('Move+Alt copies a donut (region + hole) up the Z axis onto ONE new sketch, both boundaries together', async ({
  page,
}) => {
  await setup(page)
  // Look straight down so the two nested rectangles are far apart in pixels:
  // the inner rectangle's first corner must not snap to an outer corner.
  const ctx = await pinCamera(page, {
    position: { x: 0.8, y: 0.8, z: 6 },
    target: { x: 0.8, y: 0.8, z: 0 },
    up: { x: 0, y: 1, z: 0 },
    fovDeg: 45,
    near: 0.1,
    far: 1000,
  })

  // A donut on the ground: an outer rectangle and, strictly inside it, an
  // inner rectangle (a ring region with a hole, plus the inner square). The
  // outer and inner boundaries are SEPARATE islands. The inner corner is off
  // the outer's diagonal and axes so nothing snaps it to an outer feature.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0, 0, 0)
  await page.mouse.move(px(ctx, 1.4, 1.4, 0).x, px(ctx, 1.4, 1.4, 0).y)
  await page.keyboard.type('1.6,1.6')
  await page.keyboard.press('Enter')

  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0.5, 0.7, 0)
  await page.mouse.move(px(ctx, 1.0, 1.1, 0).x, px(ctx, 1.0, 1.1, 0).y)
  await page.keyboard.type('0.6,0.5')
  await page.keyboard.press('Enter')

  const sketch = (await page.evaluate(() => window.__hew_test!.getSketchIds()))[0]
  const source = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  expect(source).toHaveLength(8) // 4 outer + 4 inner corners, one sketch

  // Select the WHOLE sketch (both islands) and copy it up the Z axis.
  await page.keyboard.press(' ')
  await page.keyboard.press('Control+a')
  await page.waitForFunction(() => window.__hew_test!.getSelection().length >= 1)

  await page.keyboard.press('m')
  await clickWorld(page, ctx, 0, 0, 0)
  await page.keyboard.press('Alt')
  await expect(page.getByText(/Copy ·/)).toBeVisible()
  await page.keyboard.press('ArrowUp')
  await page.keyboard.type('0.5')
  await page.keyboard.press('Enter')

  // Exactly ONE new sketch — NOT one-per-island (the regression split the
  // outer and inner boundaries onto two sketches, losing the ring's hole).
  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 2, null, {
    timeout: 10_000,
  })
  const copySketch = (await page.evaluate(() => window.__hew_test!.getSketchIds())).find(
    (id) => id !== sketch,
  )!

  // That one copy sketch carries BOTH boundaries (8 corners), all lifted to
  // z=0.5 — the outer ring and its hole boundary land together, so the ring
  // re-derives with its hole intact.
  const copy = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), copySketch),
  )
  expect(copy).toHaveLength(8)
  for (const p of copy) expect(p[2]).toBeCloseTo(0.5, 5)
  // The source is untouched on the ground.
  const sourceAfter = uniquePoints(
    await page.evaluate((s) => window.__hew_test!.getSketchLines(s), sketch),
  )
  expect(sourceAfter).toHaveLength(8)
  for (const p of sourceAfter) expect(p[2]).toBeCloseTo(0, 6)

  // One undo removes the whole copy (one step for the whole sketch).
  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 1, null, {
    timeout: 10_000,
  })
})

// ---------------------------------------------------------------------------
// Finding 3 — Tape Measure drags a parallel guide off a world axis
// ---------------------------------------------------------------------------

test('Tape Measure: a guide off the red axis works at meter scale, cm scale (typed), and guide points still drop', async ({
  page,
}) => {
  const ctx = await setup(page)

  // ----- Meter scale: hover the red axis at x=2, pull to y≈1.2, click. ----
  await page.keyboard.press('t')
  const axisPx = px(ctx, 2, 0, 0)
  await hoverUntilCue(page, axisPx, 'On Axis')
  await page.mouse.down()
  await page.mouse.up()
  await clickWorld(page, ctx, 2, 1.2, 0)

  await page.waitForFunction(() => window.__hew_test!.getGuideIds().length === 1)
  const g1 = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getGuideIds()[0]
    return { kind: h.getGuideKind(id), geom: h.getGuideGeometry(id) }
  })
  expect(g1.kind).toBe('line')
  // Parallel to the red axis (either sign), offset off the ground X axis.
  expect(Math.abs(g1.geom![3])).toBeCloseTo(1, 9)
  expect(Math.abs(g1.geom![4])).toBeLessThan(1e-9)
  expect(Math.abs(g1.geom![5])).toBeLessThan(1e-9)
  expect(g1.geom![1]).toBeGreaterThan(0.5) // pulled well off the axis

  // ----- cm scale: zoom to arm's length near x=5cm and type the offset. ----
  // (The clamp that trims the rendered axis lines is most aggressive when
  // zoomed right in — the snap must come from the analytic axis, not the
  // clipped render geometry.)
  // Eye azimuth deliberately NOT aligned with the target's direction from
  // the world origin: rays from an aligned eye graze the Z axis (underground,
  // past the ground plane) and its OnAxis candidate hijacks the snap.
  const cmCam: CameraParams = {
    position: { x: -0.2, y: 0.25, z: 0.24 },
    target: { x: 0.05, y: 0, z: 0 },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.01,
    far: 1000,
  }
  const cmCtx = await pinCamera(page, cmCam)
  const cmAxisPx = px(cmCtx, 0.05, 0, 0)
  await hoverUntilCue(page, cmAxisPx, 'On Axis')
  await page.mouse.down()
  await page.mouse.up()
  // Pull off the axis to give the typed offset its side, then type 2 cm.
  await page.mouse.move(px(cmCtx, 0.05, 0.03, 0).x, px(cmCtx, 0.05, 0.03, 0).y)
  await page.keyboard.type('0.02')
  await page.keyboard.press('Enter')

  await page.waitForFunction(() => window.__hew_test!.getGuideIds().length === 2)
  const g2 = await page.evaluate(() => {
    const h = window.__hew_test!
    const ids = h.getGuideIds()
    const id = ids[ids.length - 1]
    return { kind: h.getGuideKind(id), geom: h.getGuideGeometry(id) }
  })
  expect(g2.kind).toBe('line')
  expect(Math.abs(g2.geom![3])).toBeCloseTo(1, 9)
  expect(g2.geom![1]).toBeCloseTo(0.02, 9) // exactly 2 cm off the axis
  expect(Math.abs(g2.geom![2])).toBeLessThan(1e-9)

  // ----- Guide point: measure between two empty ground points. ------------
  const farCtx = await pinCamera(page, CAMERA)
  await clickWorld(page, farCtx, -1, 2, 0)
  await clickWorld(page, farCtx, -2, 3, 0)
  await page.waitForFunction(() => window.__hew_test!.getGuideIds().length === 3)
  const g3 = await page.evaluate(() => {
    const h = window.__hew_test!
    const ids = h.getGuideIds()
    const id = ids[ids.length - 1]
    return { kind: h.getGuideKind(id), geom: h.getGuideGeometry(id) }
  })
  expect(g3.kind).toBe('point')
  // Raw ground clicks carry a pixel of slack (~1 cm at this camera).
  expect(g3.geom![0]).toBeCloseTo(-2, 1)
  expect(g3.geom![1]).toBeCloseTo(3, 1)
})
