import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * Follow Me E2E — the three maintainer-playtest flows that shipped broken
 * while harness-driven equivalents stayed green, each driven end to end
 * with REAL pointer/keyboard input (docs/DEVELOPMENT.md strategy 2, the
 * input-pipeline.spec.ts pattern). The harness (`window.__hew_test`) is
 * used ONLY to pin the camera and to observe.
 *
 * 1. Path pickup: one click on one Line-tool segment of an L picks up the
 *    whole connected run (preselect flow — a Select click yields a single
 *    sketch-edge ref, which the tool now expands to its island).
 * 2. Face-boundary molding: the guide's Scenario 2 (profile stood against
 *    the box top's edge) with the placement selection deliberately left
 *    active — the stale preselection silently became the path and made
 *    every face click a dead no-op; a solid-face click at the profile
 *    stage now re-picks the path.
 * 3. Lathe: a drawn ground circle as path, an upright profile circle on
 *    its rim — refused [ProfileNotPerpendicular] for every placement while
 *    perpendicularity was measured against facet chords; the kernel now
 *    measures curve-attributed segments against the analytic tangent.
 *
 * A fourth spec pins the placement SNAP that Scenario 2 stopped covering
 * when it moved to typed-VCB moves for determinism: a Move drag onto a box
 * top-face rim MIDPOINT, its drop gated on the Midpoint inference chip being
 * resolved (not a blind jump-move), landing the profile exactly on the rim.
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

async function setup(page: Page, camera: CameraParams = CAMERA): Promise<Ctx> {
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
    camera,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(camera, rect.width / rect.height), rect }
}

/** Move (so tools see a snap-resolved hover first), then click. */
async function clickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

/** Hover with a settle pause (snap resolution), then click — for clicks
 * whose SNAPPED position matters (a Move drop point). */
async function clickWorldSettled(
  page: Page,
  ctx: Ctx,
  x: number,
  y: number,
  z: number,
): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(300)
  await page.mouse.down()
  await page.mouse.up()
}

/** No Follow Me refusal toast is on screen. */
async function expectNoRefusalToast(page: Page): Promise<void> {
  for (const code of [
    '[ProfileNotPerpendicular]',
    '[PathDetachedFromProfile]',
    '[PathBranches]',
    '[PathDisconnected]',
  ]) {
    await expect(page.getByText(code)).toHaveCount(0)
  }
}

test('Follow Me: one click on one Line-tool segment sweeps the whole L run (preselect flow)', async ({
  page,
}) => {
  const ctx = await setup(page)

  // Profile: a 0.6 m square drawn flat, stood upright with the REAL Rotate
  // gesture (arrow lock, two clicks, typed 90) onto the y = 0 plane —
  // square across a path that will leave the origin along +y.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, -0.3, 0.4, 0)
  await page.mouse.move(px(ctx, 0.2, 0.9, 0).x, px(ctx, 0.2, 0.9, 0).y)
  await page.keyboard.type('0.6,0.6')
  await page.keyboard.press('Enter')
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0, 0.4, 0) // bottom edge of the square
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('q')
  await page.keyboard.press('ArrowRight') // lock X
  await clickWorld(page, ctx, 0, 0, 0)
  await clickWorld(page, ctx, 0, 1, 0)
  await page.keyboard.type('90')
  await page.keyboard.press('Enter')
  // Standing: some endpoint left the ground.
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h
      .getSketchIds()
      .some((s) => h.getSketchLines(s).some((v, i) => i % 3 === 2 && Math.abs(v) > 0.3))
  })

  // Path: an L of two Line-tool segments on the ground.
  await page.keyboard.press('l')
  await clickWorld(page, ctx, 0, 0, 0)
  await clickWorld(page, ctx, 0, 2, 0)
  await clickWorld(page, ctx, 2, 2, 0)
  await page.keyboard.press('Escape')

  // The kernel welded the two segments into ONE island — the invariant the
  // one-click promise rides on.
  const lIsland = await page.evaluate(() => {
    const h = window.__hew_test!
    for (const s of h.getSketchIds()) {
      for (const island of h.getSketchIslands(s)) {
        if (island.edges.length === 2) return island.edges.length
      }
    }
    return 0
  })
  expect(lIsland).toBe(2)

  // ONE Select click on ONE segment…
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0, 1, 0) // midpoint of the first leg
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind === 'sketch-edge'
  })

  // …then Follow Me (no shortcut — the tool rail), then the profile.
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  // Gate on the tool actually being live before clicking the canvas —
  // a click racing the tool switch would drive the OUTGOING tool.
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
  await clickWorld(page, ctx, 0, 0, 0.7) // center of the standing square
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  // The sweep covers BOTH legs: a face stands over each of them.
  const probes = await page.evaluate(() => {
    const h = window.__hew_test!
    const ids = h.getObjectIds()
    return {
      solid: h.isObjectSolid(ids[0]),
      leg1: h.pickFace([0, 1, 5], [0, 0, -1]) !== null,
      leg2: h.pickFace([1.5, 2, 5], [0, 0, -1]) !== null,
    }
  })
  expect(probes.solid).toBe(true)
  expect(probes.leg1).toBe(true)
  expect(probes.leg2).toBe(true)
  await expectNoRefusalToast(page)
})

test('Follow Me: molding around a box top face with the placement selection still active (guide Scenario 2)', async ({
  page,
}) => {
  const ctx = await setup(page)

  // A 1 m box through the real tools.
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 0, 0, 0)
  await page.mouse.move(px(ctx, 0.7, 0.7, 0).x, px(ctx, 0.7, 0.7, 0).y)
  await page.keyboard.type('1,1')
  await page.keyboard.press('Enter')
  await page.keyboard.press('p')
  await clickWorld(page, ctx, 0.5, 0.5, 0)
  await page.keyboard.type('1')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  // Profile: a 0.2 m square beside the box, stood up (Rotate, Y lock,
  // typed -90) onto the x = 2.4 plane…
  await page.keyboard.press('r')
  await clickWorld(page, ctx, 2.4, 0.5, 0)
  await page.mouse.move(px(ctx, 2.55, 0.65, 0).x, px(ctx, 2.55, 0.65, 0).y)
  await page.keyboard.type('0.2,0.2')
  await page.keyboard.press('Enter')
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 2.5, 0.5, 0)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('q')
  await page.keyboard.press('ArrowLeft') // lock Y
  await clickWorld(page, ctx, 2.4, 0.5, 0)
  await clickWorld(page, ctx, 2.6, 0.5, 0)
  await page.keyboard.type('-90')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h
      .getSketchIds()
      .some((s) => h.getSketchLines(s).some((v, i) => i % 3 === 2 && v > 0.15))
  })

  // …then MOVED against the top face's front edge — the guide's "Move it
  // against the edge" — placed with three axis-locked typed-VCB moves, the
  // deterministic real-keyboard equivalent this suite uses everywhere it
  // needs an exact landing (Rectangle/Push-Pull/Arc/Rotate all commit
  // through the typed VCB). A rim snap-drag lands the same profile for a
  // human — the continuous hover locks the midpoint snap — but a single
  // synthetic jump-move races the ~100 ms snap throttle and the
  // endpoint/midpoint priority, so it is not a sound thing to pin a
  // regression on; the snap-drag itself is verified out of band. Steps: up
  // 1 (z→1..1.2), toward the box 1.9 (x→0.5), onto the edge 0.5 (y→0..0.2).
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 2.4, 0.6, 0) // the standing square's bottom edge
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('m')
  const typedMove = async (
    grab: [number, number, number],
    lock: 'ArrowRight' | 'ArrowLeft' | 'ArrowUp',
    toward: [number, number, number],
    dist: string,
  ): Promise<void> => {
    await clickWorldSettled(page, ctx, grab[0], grab[1], grab[2])
    await page.keyboard.press(lock)
    await page.mouse.move(px(ctx, toward[0], toward[1], toward[2]).x, px(ctx, toward[0], toward[1], toward[2]).y)
    await page.waitForTimeout(150)
    await page.keyboard.type(dist)
    await page.keyboard.press('Enter')
    await page.waitForTimeout(150)
  }
  await typedMove([2.4, 0.5, 0], 'ArrowUp', [2.4, 0.5, 0.8], '1')
  await typedMove([2.4, 0.5, 1], 'ArrowRight', [1.2, 0.5, 1], '1.9')
  await typedMove([0.5, 0.5, 1], 'ArrowLeft', [0.5, 0.1, 1], '0.5')
  // The profile now stands on the rim plane (x = 0.5, y 0..0.2, z 1..1.2)
  // with its plane crossing the boundary edge mid-run. Fail loudly if the
  // placement missed.
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().some((s) => {
      const lines = h.getSketchLines(s)
      if (lines.length === 0) return false
      let minZ = Infinity
      let xPlane = null as number | null
      for (let i = 0; i < lines.length; i += 3) {
        minZ = Math.min(minZ, lines[i + 2])
        xPlane = lines[i]
      }
      return xPlane !== null && xPlane > 0.05 && xPlane < 0.95 && Math.abs(minZ - 1) < 1e-3
    })
  })

  // The Move left the profile's edge SELECTED — the maintainer's state.
  // Follow Me silently adopts it as the path; before the fix, clicking the
  // top face here was a dead no-op and the profile click swept the
  // profile's own outline ([ProfileNotPerpendicular], forever).
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  // Gate on the tool actually being live before clicking the canvas —
  // a click racing the tool switch would drive the OUTGOING tool.
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
  await clickWorldSettled(page, ctx, 0.75, 0.75, 1) // the box's top face → re-picks the path
  await clickWorldSettled(page, ctx, 0.5, 0.1, 1.1) // the standing profile → commit
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 2)

  const after = await page.evaluate(() => {
    const h = window.__hew_test!
    return { solids: h.getObjectIds().map((id) => h.isObjectSolid(id)) }
  })
  expect(after.solids).toEqual([true, true])
  await expectNoRefusalToast(page)
})

/**
 * Hover the pixel where `world` renders, let the snap throttle settle, and
 * wait until the named inference chip (`InferenceTooltip`) is actually
 * showing — the deterministic gate that replaces the removed blind
 * single-jump drag. The pre-nudge guarantees a fresh pointermove lands ON
 * the target rather than being coalesced away by the ~100 ms snap throttle.
 */
async function hoverUntilSnap(
  page: Page,
  ctx: Ctx,
  x: number,
  y: number,
  z: number,
  label: string,
): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 5, p.y - 5)
  await page.waitForTimeout(60)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(160)
  await expect(page.getByText(label, { exact: true })).toBeVisible()
}

test('Follow Me setup: a Move snap-drag lands the profile on a box top-face rim midpoint', async ({
  page,
}) => {
  // Scenario 2's placement (313c157) traded its real snap-drag for typed-VCB
  // moves to kill synthetic-input flakiness — sound for the tool regression,
  // but it left the drag-to-snap-onto-rim-midpoint workflow (the guide's
  // "Move it against the edge; the midpoint and edge snaps put it exactly on
  // the rim") with no automated coverage. This pins that snap path WITHOUT
  // the old flakiness: the stage is built through the harness so the one
  // thing under test is the pointer-driven Move, and the drop is gated on the
  // Midpoint chip being genuinely resolved rather than a blind jump-move
  // racing the throttle and the endpoint/midpoint priority.
  const ctx = await setup(page, {
    position: { x: 5, y: -4, z: 4 },
    target: { x: 1, y: 0.3, z: 0.4 },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.01,
    far: 1000,
  })

  // A 1 m box at the origin and a 0.2 m profile square off to the side, both
  // built through the harness (deterministic, and NOT what is under test).
  const profileSketch = await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [1, 1, 0], 1)
    return h.drawRectangle([2.0, 0.5, 0], [2.2, 0.7, 0]).sketch
  })
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  // Select the profile with a real Select click on its edge, then Move.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 2.1, 0.5, 0) // the profile's front edge
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.keyboard.press('m')

  // Grab the profile's near corner — an Endpoint snap makes the base exact…
  await hoverUntilSnap(page, ctx, 2.0, 0.5, 0, 'Endpoint')
  await page.mouse.down()
  await page.mouse.up()

  // …then drag onto the box top-face front-edge MIDPOINT. The Midpoint chip
  // must be resolved before the drop — that assertion IS the snap coverage.
  await hoverUntilSnap(page, ctx, 0.5, 0, 1, 'Midpoint')
  await page.mouse.down()
  await page.mouse.up()

  // The grabbed corner rode the snap exactly onto the rim midpoint (0.5,0,1).
  const nearestToMidpoint = await page.evaluate((sketch) => {
    const lines = window.__hew_test!.getSketchLines(sketch)
    let best = Infinity
    for (let i = 0; i < lines.length; i += 3) {
      best = Math.min(best, Math.hypot(lines[i] - 0.5, lines[i + 1] - 0, lines[i + 2] - 1))
    }
    return best
  }, profileSketch)
  expect(nearestToMidpoint).toBeLessThan(1e-6)
})

test('Follow Me: lathe — upright profile circle on a drawn ground circle sweeps a closed ring solid', async ({
  page,
}) => {
  // Close framing: the whole scene lives within ~0.5 m of the origin.
  const ctx = await setup(page, {
    position: { x: 1.1, y: 0.8, z: 0.9 },
    target: { x: 0.15, y: 0, z: 0 },
    up: { x: 0, y: 0, z: 1 },
    fovDeg: 45,
    near: 0.01,
    far: 1000,
  })

  // Path: a 10 cm-radius circle about a center ON the red axis (the axis
  // snap pins the center's y to exactly 0), radius typed, rim direction
  // along +X — so a facet vertex sits exactly on the axis.
  await page.keyboard.press('c')
  await clickWorldSettled(page, ctx, 0, 0, 0)
  await page.mouse.move(px(ctx, 0.06, 0, 0).x, px(ctx, 0.06, 0, 0).y)
  await page.waitForTimeout(200)
  await page.keyboard.type('0.1')
  await page.keyboard.press('Enter')

  // Profile: a 2 cm circle drawn beside the path (clear of it), its center
  // on the same axis.
  await page.keyboard.press('c')
  await clickWorldSettled(page, ctx, 0.35, 0, 0)
  await page.mouse.move(px(ctx, 0.4, 0, 0).x, px(ctx, 0.4, 0, 0).y)
  await page.waitForTimeout(200)
  await page.keyboard.type('0.02')
  await page.keyboard.press('Enter')

  // Stand it up: Rotate, X lock, pivot ON the red axis (the axis snap pins
  // the standing plane's y to exactly 0 — a rim-vertex pivot would tilt
  // it), reference off-axis, typed 90°.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0.37, 0, 0) // the profile circle's rim
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('q')
  await page.keyboard.press('ArrowRight')
  await clickWorldSettled(page, ctx, 0.5, 0, 0)
  await clickWorld(page, ctx, 0.5, 0.1, 0)
  await page.keyboard.type('90')
  await page.keyboard.press('Enter')
  // Upright: the profile detached to its own sketch standing in z.
  await page.waitForFunction(() => window.__hew_test!.getSketchIds().length === 2)

  // Place it on the rim: Move, X lock, typed 0.25 toward the axis.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0.37, 0, 0)
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.keyboard.press('m')
  await clickWorldSettled(page, ctx, 0.37, 0, 0)
  await page.keyboard.press('ArrowRight')
  await page.mouse.move(px(ctx, 0.2, 0, 0).x, px(ctx, 0.2, 0, 0).y)
  await page.keyboard.type('0.25')
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => {
    const h = window.__hew_test!
    return h.getSketchIds().some((s) => {
      const lines = h.getSketchLines(s)
      return lines.length > 0 && lines.every((v, i) => i % 3 !== 0 || v < 0.2)
    })
  })

  // The maintainer's own flow: select the path circle (one click grabs the
  // whole drawn circle), Follow Me, click the profile.
  await page.keyboard.press(' ')
  await clickWorld(page, ctx, 0, 0.1, 0) // the path circle's rim, a quarter turn away
  await page.waitForFunction(() => {
    const sel = window.__hew_test!.getSelection()
    return sel.length === 1 && sel[0].kind !== 'object'
  })
  await page.getByRole('radio', { name: 'Follow Me' }).click()
  // Gate on the tool actually being live before clicking the canvas —
  // a click racing the tool switch would drive the OUTGOING tool.
  await expect(page.getByText('Click the profile to sweep along')).toBeVisible()
  await clickWorld(page, ctx, 0.1, 0, 0.005) // the upright profile disk
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)

  // A closed ring: solid, watertight badge, and material all the way
  // around — straight-down probes at three arbitrary angles (off the facet
  // vertices and the seam, where a ray would graze an edge exactly).
  const probes = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.getObjectIds()[0]
    const at = (deg: number) => {
      const a = (deg * Math.PI) / 180
      return h.pickFace([0.1 * Math.cos(a), 0.1 * Math.sin(a), 5], [0, 0, -1]) !== null
    }
    return { solid: h.isObjectSolid(id), a20: at(20), a140: at(140), a260: at(260) }
  })
  expect(probes.solid).toBe(true)
  expect(probes.a20).toBe(true)
  expect(probes.a140).toBe(true)
  expect(probes.a260).toBe(true)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
  await expectNoRefusalToast(page)
})
