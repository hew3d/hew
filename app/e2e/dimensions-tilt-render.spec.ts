import { test, expect } from '@playwright/test'

/**
 * Linear dimension rendering under a VERTICALLY TILTED drawing camera —
 * end-to-end, driven through the real DimensionTool click-drag-click gesture
 * (not `addLinearDimension`'s exact-coordinate shortcut), with real WebGL
 * pixel checks (`pixelColorAt`) that the committed dimension's line AND
 * label actually render on screen.
 *
 * This closes a real coverage gap: `dimensions-depth.spec.ts`'s rendering
 * checks all place annotations directly via `addLinearDimension`, bypassing
 * `DimensionTool`'s own `viewFacingPlaneNormal`/`computeLineLabelLayout`
 * gesture-plane and gap-layout math entirely; `dimensions-text.spec.ts`
 * drives the real gesture but only asserts the committed annotation's LABEL
 * TEXT, never that it actually rendered a pixel. Neither test would catch a
 * defect where the real gesture, run under a tilted camera, committed a
 * geometrically valid annotation that nonetheless failed to render. This
 * file drives the real gesture AND samples real pixels, at a sweep of
 * camera-tilt angles (camera Z vs. orbit-target Z), to cover exactly that
 * combination.
 *
 * `nextFrame` (two real `requestAnimationFrame`s) is required after every
 * camera/scene change before sampling pixels: `pixelColorAt`'s underlying
 * `captureFrame` renders out-of-band and does NOT itself run
 * `SceneRenderer.updateAnnotationBillboards` (the per-frame pass that lays
 * out a linear dimension's line/label gap) — see `dimensions-depth.spec.ts`'s
 * own `nextFrame` doc comment for the full mechanism and the prior test that
 * shipped without it.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pixel counts are only stable on pinned SwiftShader')

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
})

function nextFrame(page: import('@playwright/test').Page): Promise<void> {
  return page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      }),
  )
}

/** The first of `candidates` that projects on-screen (and reads back a real
 * pixel) at the CURRENT camera — used as the "definitely background, not
 * annotation ink" reference for a delta-from-background check, since a
 * fixed guessed point can legitimately fall outside the frustum at some of
 * this sweep's tilt angles. */
async function findOnscreenBackground(
  page: import('@playwright/test').Page,
  candidates: [number, number, number][],
): Promise<[number, number, number] | null> {
  for (const c of candidates) {
    const proj = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), c)
    if (proj.behind || proj.x < 5 || proj.y < 5) continue
    const px = await page.evaluate((w) => window.__hew_test!.pixelColorAt(w), c)
    if (px !== null) return c
  }
  return null
}

test('a linear dimension drawn via the real gesture renders its line and label across a tilt sweep', async ({ page }) => {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const toPage = async (world: [number, number, number]) => {
    const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w as [number, number, number]), world)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  const click = async (pt: { x: number; y: number }) => {
    await page.mouse.move(pt.x, pt.y)
    await page.mouse.down()
    await page.mouse.up()
  }

  // Each scenario dimensions the same vertical box edge, offset along X so
  // the annotations never interact (mirrors dimensions-depth.spec.ts's own
  // xOff convention) — one page load for the whole sweep. `tiltDeg` is the
  // camera's elevation off dead-level at the drawing moment: 0 is the
  // control (camera Z == orbit-target Z); the rest cover a real range on
  // both sides of level, from barely off to steep.
  const tilts = [-45, -20, -1, -0.1, 0, 0.1, 1, 20, 45]
  const results: { tiltDeg: number; lineDelta: number; labelDelta: number }[] = []

  for (let i = 0; i < tilts.length; i++) {
    const tiltDeg = tilts[i]
    const xOff = i * 6

    const target: [number, number, number] = [xOff + 1, 1, 0.5]
    const dx = 5
    const dy = -9
    const horiz = Math.hypot(dx, dy)
    const z = target[2] + horiz * Math.tan((tiltDeg * Math.PI) / 180)
    const position: [number, number, number] = [target[0] + dx, target[1] + dy, z]

    await page.evaluate(
      ([pos, tgt]) =>
        window.__hew_test!.setCamera({
          position: pos as [number, number, number],
          target: tgt as [number, number, number],
          fovDeg: 45,
        }),
      [position, target],
    )
    await page.evaluate((xo) => window.__hew_test!.drawBox([xo, 0, 0], [xo + 2, 2, 0], 1.5), xOff)
    await nextFrame(page)

    await page.getByRole('radio', { name: 'Dimension' }).click()
    const aPt = await toPage([xOff, 0, 0])
    const bPt = await toPage([xOff, 0, 1.5])
    await click(aPt)
    await click(bPt)
    // Drag the offset out by a pure screen-space perpendicular displacement,
    // AWAY from the box (mirrors dimensions-text.spec.ts's leader-drag
    // methodology) — robust regardless of which 3D plane the gesture
    // actually constrains to under this camera's tilt.
    const ldx = bPt.x - aPt.x
    const ldy = bPt.y - aPt.y
    const llen = Math.hypot(ldx, ldy) || 1
    const dragPt = { x: (aPt.x + bPt.x) / 2 + (ldy / llen) * 80, y: (aPt.y + bPt.y) / 2 + (-ldx / llen) * 80 }
    await click(dragPt)
    await nextFrame(page)

    const ids = await page.evaluate(() => window.__hew_test!.getAnnotationIds())
    const id = ids[ids.length - 1]
    expect(id, `tilt ${tiltDeg}: a dimension was committed`).toBeTruthy()
    expect(await page.evaluate((i) => window.__hew_test!.getAnnotationKind(i), id)).toBe('linear')

    const endpoints = await page.evaluate((i) => window.__hew_test!.getLinearDimensionEndpoints(i), id)
    expect(endpoints, `tilt ${tiltDeg}: committed endpoints are readable`).not.toBeNull()
    const { a1, b1 } = endpoints!

    const bg = await findOnscreenBackground(page, [
      [xOff + 3, 3, 3],
      [xOff, 0, 2],
      [xOff, 0, 4],
      [xOff + 6, 6, 6],
    ])
    expect(bg, `tilt ${tiltDeg}: found an on-screen background reference`).not.toBeNull()
    const bgColor = await page.evaluate((w) => window.__hew_test!.pixelColorAt(w), bg!)
    expect(bgColor, `tilt ${tiltDeg}: background reference reads back a pixel`).not.toBeNull()

    // The LINE: scan along the whole a1-b1 segment (covers both a 'broken'
    // gap layout's two visible pieces and an 'outside' layout's whole line)
    // for the point that most clearly differs from background.
    let lineDelta = -Infinity
    for (let k = 1; k < 24; k++) {
      const f = k / 24
      const p: [number, number, number] = [
        a1[0] + (b1[0] - a1[0]) * f,
        a1[1] + (b1[1] - a1[1]) * f,
        a1[2] + (b1[2] - a1[2]) * f,
      ]
      const c = await page.evaluate((w) => window.__hew_test!.pixelColorAt(w), p)
      if (c === null) continue
      const d = Math.abs(c.r - bgColor!.r) + Math.abs(c.g - bgColor!.g) + Math.abs(c.b - bgColor!.b)
      if (d > lineDelta) lineDelta = d
    }

    // The LABEL: the world-space position `SceneRenderer` currently holds
    // the label billboard at — the real render-time anchor, not a guess at
    // the line's midpoint, so this is correct in BOTH the 'broken'
    // (label centered on the line) and 'outside' (label pushed past an end)
    // layout modes.
    const labelWorld = await page.evaluate((i) => window.__hew_test!.getAnnotationTextWorldPosition(i), id)
    let labelDelta = -Infinity
    if (labelWorld !== null) {
      const c = await page.evaluate((w) => window.__hew_test!.pixelColorAt(w), labelWorld)
      if (c !== null) {
        labelDelta = Math.abs(c.r - bgColor!.r) + Math.abs(c.g - bgColor!.g) + Math.abs(c.b - bgColor!.b)
      }
    }

    results.push({ tiltDeg, lineDelta, labelDelta })
  }

  for (const { tiltDeg, lineDelta, labelDelta } of results) {
    expect(lineDelta, `tilt ${tiltDeg}: the dimension line rendered (differs from background)`).toBeGreaterThan(30)
    expect(labelDelta, `tilt ${tiltDeg}: the dimension label rendered (differs from background)`).toBeGreaterThan(30)
  }
})
