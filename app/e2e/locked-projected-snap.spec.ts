import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { type CameraParams } from './helpers/projectWorldToScreen'

/**
 * Axis-locked PROJECTED inference, on the maintainer's own repro file.
 *
 * The gesture: a rectangle sketch was nudged up off the ground by accident
 * and has to go back down. Move, click one of its floating corners, lock the
 * blue axis with ↑, then name the height by hovering a point that already
 * sits at the height you want — a corner of the desktop below. That hovered
 * point is nowhere near the blue guide line; its only contribution is the
 * station its PROJECTION onto that line names, which is the entire mechanism
 * a projected snap works by.
 *
 * The defect: the inference crate read the SIDE of the anchor the gesture
 * indicated from where the pick ray came closest to the lock line, and threw
 * away candidates on the other side. Off the lock line that station is set by
 * where the eye is, not by where the cursor is — for a vertical lock the ray's
 * closest approach passes ABOVE the anchor whenever the hovered geometry is on
 * the far side of it — so most points in the model stopped offering a snap at
 * all, and WHICH ones survived changed with every orbit. Both halves are
 * pinned here: the drop must commit at the hovered corner's own height, and
 * every corner of the desktop must stay reachable from every camera around it.
 *
 * The kernel-level property (and the narrow on-the-guide-line case that keeps
 * its original wrong-side rule) lives in the inference crate's specs; this
 * spec exists because the defect only ever showed up as a real gesture on a
 * real file.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

interface Ctx {
  rect: { left: number; top: number; width: number; height: number }
}

type P3 = [number, number, number]

const FIXTURE = fileURLToPath(new URL('./fixtures/inference-points.hew', import.meta.url))

/** The chip labels `InferenceTooltip` can render, longest-first so `On Edge`
 *  is never truncated to a prefix of another label. */
const CUE =
  /^(Endpoint|Midpoint|Intersection|Quadrant|Tangent|Center|On Edge|On Face|On Guide|On Axis|On Plane|Ground)/

/** The height the rectangle was accidentally nudged up to. */
const FLOATING_Z = 0.07
/** The floating rectangle's corner the Move grabs. */
const GRAB: P3 = [0.14841397106647491, -0.1391424536705017, FLOATING_Z]

/** The desktop slab's top face, z = 0.015 — the height to land on. */
const DESKTOP_TOP_Z = 0.015
/** The corner the reported gesture aimed at, over the world origin. */
const ORIGIN_CORNER: P3 = [0, 0, DESKTOP_TOP_Z]
/** A corner clear of both world axes — see the orbit test's note. */
const OFF_AXIS_CORNER: P3 = [0.24, 0.14, DESKTOP_TOP_Z]

const LOOK_AT = { x: 0.12, y: 0.02, z: 0.03 }
const ORBIT_RADIUS = 0.62

function orbit(deg: number): CameraParams {
  const th = (deg * Math.PI) / 180
  return {
    position: {
      x: LOOK_AT.x + ORBIT_RADIUS * Math.cos(th),
      y: LOOK_AT.y + ORBIT_RADIUS * Math.sin(th),
      z: LOOK_AT.z + 0.42,
    },
    target: LOOK_AT,
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.01,
    far: 1000,
  }
}

/**
 * Where `p` lands on the page, via the APP's own projection rather than a
 * second implementation of it here — the two disagreeing is indistinguishable
 * from the snap failing, and only one of them is the thing under test.
 * Correct only once `aim` has let the render loop refresh the camera matrices
 * (see its note).
 */
async function px(page: Page, ctx: Ctx, p: P3): Promise<{ x: number; y: number }> {
  const q = await page.evaluate(
    (w) => window.__hew_test!.worldToScreen(w),
    p as [number, number, number],
  )
  if (q.behind) throw new Error(`world ${p.join(',')} is behind the camera`)
  return { x: ctx.rect.left + q.x, y: ctx.rect.top + q.y }
}

/**
 * Point the camera and wait until the app is really using that pose.
 *
 * Retried rather than set once: undo restores the DOCUMENT's own stored
 * camera, and it lands a frame or two later — long enough to clobber a
 * `setCamera` issued right after it. Each attempt also lets the render loop
 * run, because `setCamera` leaves three.js's cached `matrixWorld` /
 * `matrixWorldInverse` stale until the next render traversal recomputes them,
 * and BOTH `worldToScreen` and the app's own pick-ray construction read those
 * caches — project or hover before that and the pointer lands on empty space
 * through the PREVIOUS pose (the harness's `pixelColorAt` documents the same
 * hazard).
 */
async function aim(page: Page, camera: CameraParams): Promise<Ctx> {
  await expect
    .poll(
      async () => {
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
        await page.evaluate(
          () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))),
        )
        return page.evaluate((cam) => {
          const c = window.__hew_test!.getCamera()
          const near = (a: number[], b: number[]) => a.every((v, i) => Math.abs(v - b[i]) < 1e-6)
          return (
            near(c.position, [cam.position.x, cam.position.y, cam.position.z]) &&
            near(c.target, [cam.target.x, cam.target.y, cam.target.z])
          )
        }, camera)
      },
      { timeout: 10_000, message: 'the camera never settled on the requested pose' },
    )
    .toBe(true)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  return { rect: { left: box.x, top: box.y, width: box.width, height: box.height } }
}

/** Load the maintainer's file through the app's real Open path. */
async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await page.evaluate((arr) => window.__hew_test!.load(arr), Array.from(readFileSync(FIXTURE)))
  // The load settles asynchronously (re-tessellation, viewport notify). Wait
  // for the document to actually BE the fixture before anything hovers it —
  // on a warm dev server the first hover otherwise beats the load and lands
  // on an empty scene.
  await expect.poll(async () => await sketchHeight(page)).toBeCloseTo(FLOATING_Z, 5)
}

/** Select the sketch, start Move, grab `GRAB`, and lock the blue axis. */
async function grabAndLockBlue(page: Page, ctx: Ctx): Promise<void> {
  await page.keyboard.press('Escape')
  // ONLY the sketch — the reported gesture moves the floating rectangle, not
  // the desk under it. (A Select-all would take the four objects too, and the
  // whole model sliding together happens to satisfy a height assertion while
  // testing a different gesture entirely.)
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.selectNodes([{ kind: 'sketch', id: h.getSketchIds()[0] }])
  })
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('m')
  // Poll the corner's own cue before pressing: a bare `page.mouse.move` can
  // land before the tool switch has taken, and a Move whose base came from a
  // different point (or from the ground fallback) silently invalidates every
  // station this spec goes on to assert.
  await hoverCue(
    page,
    await px(page, ctx, GRAB),
    (c) => c === 'Endpoint',
    'the Move never offered the floating rectangle corner as its base',
  )
  await page.mouse.down()
  await page.mouse.up()
  await page.keyboard.press('ArrowUp')
}

/**
 * Hover `p` until its inference chip settles on a label `want` accepts, and
 * return that label. Each poll wiggles the pointer one pixel and back so a
 * fresh pointermove resolves against the CURRENT camera and the CURRENT
 * render — a single move can land before either has caught up (the
 * `hoverUntilCue` precedent in `playtest-fixes`), and the first cue a hover
 * produces is routinely the ground/plane fallback on the way to the real one.
 *
 * Timing out IS this spec's failure signal: under the lock, a point that
 * offers nothing leaves the chip on the bare `On Axis` fallback forever.
 */
async function hoverCue(
  page: Page,
  p: { x: number; y: number },
  want: (cue: string) => boolean,
  what: string,
): Promise<string> {
  let seen = ''
  await expect
    .poll(
      async () => {
        await page.mouse.move(p.x + 1, p.y)
        await page.mouse.move(p.x, p.y)
        const texts = await page.locator('div:has(> span)').allTextContents()
        const hit = texts.find((t) => CUE.test(t.trim()))
        seen = hit === undefined ? '' : (CUE.exec(hit.trim()) ?? [''])[0]
        return want(seen)
      },
      // `message` is built once, before polling, so it deliberately does not
      // quote `seen` — that would always read empty and misreport the cue.
      { timeout: 10_000, message: what },
    )
    .toBe(true)
  return seen
}

/** Chip labels that name real geometry, as opposed to a fallback onto the
 *  ground, the active plane, or — the tell this spec is written around — the
 *  lock's own guide line when no candidate survived. */
const NAMES_GEOMETRY = (cue: string) =>
  cue !== '' && !['Ground', 'On Plane', 'On Axis'].includes(cue)

/** Every distinct z among a sketch's line soup. */
function heights(lines: number[]): number[] {
  const out: number[] = []
  for (let i = 2; i < lines.length; i += 3) if (!out.includes(lines[i])) out.push(lines[i])
  return out
}

/** The sketch's single height, or null when it is not planar in z. The
 *  readback is f32-quantized, so heights are compared, never equated. */
async function sketchHeight(page: Page): Promise<number | null> {
  const zs = await sketchHeights(page)
  return zs.length === 1 ? zs[0] : null
}

async function sketchHeights(page: Page): Promise<number[]> {
  return heights(
    await page.evaluate(() => {
      const h = window.__hew_test!
      return h.getSketchLines(h.getSketchIds()[0])
    }),
  )
}

test('a blue-locked Move drops the floating sketch onto a hovered desktop corner', async ({
  page,
}) => {
  await setup(page)
  // The camera the defect was reported from: the desktop's near-left corner
  // sits on the FAR side of the grabbed corner, which is exactly the
  // configuration the wrong-side gate used to discard.
  const ctx = await aim(page, orbit(320))

  await grabAndLockBlue(page, ctx)

  // The corner must ANNOUNCE itself as an endpoint — pre-fix the chip stayed
  // on the bare "On Axis" fallback, the visible face of the dropped candidate.
  await hoverCue(
    page,
    await px(page, ctx, ORIGIN_CORNER),
    (c) => c === 'Endpoint',
    'the desktop corner offered no projected endpoint under the blue lock',
  )
  await page.mouse.down()
  await page.mouse.up()

  await expect.poll(async () => await sketchHeight(page)).not.toBeNull()
  expect(await sketchHeight(page)).toBeCloseTo(DESKTOP_TOP_Z, 5)
})

test('the desktop corner names the same height from every camera around it', async ({ page }) => {
  await setup(page)

  // Asserted on the committed HEIGHT rather than the chip's label, because
  // the label is legitimately camera-dependent where the corner's own edges
  // run nearly through it (`Endpoint` from most azimuths, `On Edge` from the
  // one sighting down an edge) while the height those name is identically
  // 0.015. It is the height the gesture is for, and the height is what the
  // defect destroyed — pre-fix the resolve fell through to the bare axis and
  // committed the cursor's meaningless slide along the guide line instead.
  //
  // `OFF_AXIS_CORNER` deliberately avoids the desktop's two corners that sit
  // ON a world axis: the red/green axis line through them is a real competing
  // inference at the same pixel, so which of the two wins there is a ranking
  // question, not this one.
  // 270° is omitted deliberately: from due south the objects standing on the
  // desk sit between the eye and this corner, so the occlusion cull hides it —
  // correctly, and for a reason that has nothing to do with the lock.
  for (const deg of [20, 70, 170, 220, 320]) {
    const ctx = await aim(page, orbit(deg))
    await grabAndLockBlue(page, ctx)

    await hoverCue(
      page,
      await px(page, ctx, OFF_AXIS_CORNER),
      NAMES_GEOMETRY,
      `azimuth ${deg}°: the desktop corner offered nothing under the blue lock`,
    )
    await page.mouse.down()
    await page.mouse.up()

    await expect
      .poll(async () => await sketchHeight(page), {
        message: `azimuth ${deg}°: the blue-locked drop did not land on the \
hovered desktop corner's height — the locked resolve discarded the candidate \
the cursor was squarely on`,
      })
      .toBeCloseTo(DESKTOP_TOP_Z, 5)

    // Back to the floating original for the next azimuth.
    await page.keyboard.press('Control+z')
    await expect.poll(async () => await sketchHeight(page)).toBeCloseTo(FLOATING_Z, 5)
  }
})
