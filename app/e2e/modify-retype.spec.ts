import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * The "retype window" (tools/retypeWindow.ts) extended to the modify tools:
 * after a push/pull, offset, move, rotate, or scale COMMITS, typing an exact
 * value + Enter redoes that same operation at the new value, in place --
 * SketchUp's "do it roughly, then type the exact number", repeatable until
 * the next pointer action, Escape, tool switch, or other model change. One
 * Undo after any number of retypes removes the whole operation. Each tool's
 * own `RetypeSpec` doc comment (top of `PushPullTool.ts`, `OffsetTool.ts`,
 * `MoveTool.ts`, `RotateTool.ts`, `ScaleTool.ts`) is the source of truth for
 * the exact sign/direction rules exercised below.
 *
 * `rectangle-retype.spec.ts` and `shape-retype.spec.ts` cover the draw
 * tools' version of the same window; this file mirrors their conventions
 * (real pointer/keyboard events, a `CAMERA` pose, `pagePoint`/`clickWorld`
 * helpers) for the five modify tools.
 */

const CAMERA = {
  position: [7, -7, 6] as [number, number, number],
  target: [1, 1, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

async function setup(page: Page): Promise<void> {
  // These specs predate linked copies and assert on plain, independent
  // copies; the preference gives them exactly that. linked-copies.spec.ts
  // covers the default.
  await page.addInitScript(() => localStorage.setItem('hew.settings.copiesLinked', 'false'))
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  await page.waitForTimeout(100)
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function clickWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

async function moveToWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y)
}

/** Whether the Chromium instance driving this test reports a macOS
 * platform -- MoveTool/RotateTool's copy-toggle modifier is Option there
 * (a bare Alt keydown, handled in the tool's own `onKey`) and Control
 * everywhere else (a Viewport-level "clean tap", see `platform.ts`'s
 * `COPY_MODIFIER_KEY`). Checked live rather than assumed so this spec stays
 * correct on whatever OS actually runs it. */
async function isMacPlatform(page: Page): Promise<boolean> {
  return page.evaluate(() => /Mac|iPod|iPhone|iPad/.test(navigator.platform))
}

async function tapCopyModifier(page: Page): Promise<void> {
  const mac = await isMacPlatform(page)
  await page.keyboard.press(mac ? 'Alt' : 'Control')
}

async function boundsOf(page: Page, id: string): Promise<[number, number, number, number, number, number]> {
  return page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
}

async function soleObjectId(page: Page): Promise<string> {
  const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  return ids[0]
}

// ---------------------------------------------------------------------------
// Push/Pull
// ---------------------------------------------------------------------------

test.describe('Push/Pull retype', () => {
  test('typing a distance after a push/pull commit redoes it at that distance (negative flips direction); one undo removes the whole push/pull', async ({
    page,
  }) => {
    await setup(page)
    const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
    const before = await boundsOf(page, id)
    expect(before[5] - before[2]).toBeCloseTo(1, 6) // 1 m tall as drawn

    await page.keyboard.press('p')
    // Top face center -- no drag, so a positive typed distance pulls OUTWARD
    // (grows the box) per PushPullTool's `_commitFromTyped` no-drag default.
    await clickWorld(page, [1, 1, 1])
    await page.keyboard.type('0.5')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    let ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    let b = await boundsOf(page, ids[0])
    expect(b[5] - b[2]).toBeCloseTo(1.5, 5)
    expect(b[2]).toBeCloseTo(0, 6) // base stayed at z=0

    // Retype to 2 -- redone from the ORIGINAL (pre-push/pull) height, not
    // cumulative: 1 + 2 = 3, not 1.5 + 2.
    await page.keyboard.type('2')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    b = await boundsOf(page, ids[0])
    expect(b[5] - b[2]).toBeCloseTo(3, 5)
    expect(b[2]).toBeCloseTo(0, 6)

    // Retype a NEGATIVE distance -- an explicit typed sign flips the
    // committed (outward) direction to inward, recessing from the original
    // top: height 1 - 0.4 = 0.6.
    await page.keyboard.type('-0.4')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    b = await boundsOf(page, ids[0])
    expect(b[5] - b[2]).toBeCloseTo(0.6, 5)
    expect(b[2]).toBeCloseTo(0, 6)

    // One undo retracts the whole push/pull, however many times it was
    // retyped -- back to the box exactly as drawn.
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    b = await boundsOf(page, ids[0])
    expect(b).toEqual(before)
  })
})

// ---------------------------------------------------------------------------
// Offset
// ---------------------------------------------------------------------------

test.describe('Offset retype', () => {
  test('typing a distance after a face-offset commit redoes the inset loop at that distance; one undo removes it', async ({
    page,
  }) => {
    await setup(page)
    const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))

    await page.keyboard.press('f')
    // Top face center -- no drag, so a positive typed distance insets
    // INWARD (the only direction that can land on a face) per OffsetTool's
    // `_commitFromTyped` no-drag default.
    await clickWorld(page, [1, 1, 1])
    await page.keyboard.type('0.4')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    // Probe the imprinted loop's extent with vertical rays: the box center
    // always lands on the new INNER (inset) face; a point closer to the
    // edge than the offset distance lands on the outer annulus instead --
    // a different face handle.
    let probe = await page.evaluate(() => {
      const h = window.__hew_test!
      return {
        center: h.pickFace([1, 1, 5], [0, 0, -1])?.face ?? null,
        outside: h.pickFace([0.3, 1, 5], [0, 0, -1])?.face ?? null, // 0.3 m from the edge, < 0.4 offset
        inside: h.pickFace([0.5, 1, 5], [0, 0, -1])?.face ?? null, // 0.5 m from the edge, > 0.4 offset
      }
    })
    expect(probe.center).not.toBeNull()
    expect(probe.outside).not.toBe(probe.center) // 0.3 m in: still the outer annulus
    expect(probe.inside).toBe(probe.center) // 0.5 m in: inside the 0.4 m inset

    // Retype to a deeper inset (0.6) -- the boundary moves accordingly.
    await page.keyboard.type('0.6')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    probe = await page.evaluate(() => {
      const h = window.__hew_test!
      return {
        center: h.pickFace([1, 1, 5], [0, 0, -1])?.face ?? null,
        outside: h.pickFace([0.5, 1, 5], [0, 0, -1])?.face ?? null, // now < 0.6: outside
        inside: h.pickFace([0.7, 1, 5], [0, 0, -1])?.face ?? null, // now > 0.6: inside
      }
    })
    expect(probe.center).not.toBeNull()
    expect(probe.outside).not.toBe(probe.center)
    expect(probe.inside).toBe(probe.center)

    // One undo retracts the whole offset -- back to a plain, solid box.
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toEqual([id])
    expect(await page.evaluate((id) => window.__hew_test!.isObjectSolid(id), id)).toBe(true)
    const b = await boundsOf(page, id)
    expect(b[5] - b[2]).toBeCloseTo(1, 6)
  })
})

// ---------------------------------------------------------------------------
// Move
// ---------------------------------------------------------------------------

test.describe('Move retype', () => {
  test('typing a distance after a move commit redoes it along the same locked axis; one undo removes it', async ({
    page,
  }) => {
    await setup(page)
    const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
    const before = await boundsOf(page, id)
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), id)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

    await page.keyboard.press('m')
    await clickWorld(page, [0.5, 0.5, 0]) // base point
    await page.keyboard.press('ArrowRight') // lock the X axis
    await moveToWorld(page, [1.2, 0, 0]) // nudge toward +X to give the typed distance its sign
    await page.keyboard.type('3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    let ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    let b = await boundsOf(page, ids[0])
    expect(b[0]).toBeCloseTo(before[0] + 3, 5)
    expect(b[3]).toBeCloseTo(before[3] + 3, 5)
    expect(b[1]).toBeCloseTo(before[1], 5)
    expect(b[2]).toBeCloseTo(before[2], 5)

    // Retype to 5 -- redone from the ORIGINAL base point, not cumulative.
    await page.keyboard.type('5')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    b = await boundsOf(page, ids[0])
    expect(b[0]).toBeCloseTo(before[0] + 5, 5)
    expect(b[3]).toBeCloseTo(before[3] + 5, 5)

    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    b = await boundsOf(page, await soleObjectId(page))
    expect(b).toEqual(before)
  })

  test('copy toggle + typed distance places a copy, and the ×N array window still resolves after it', async ({
    page,
  }) => {
    await setup(page)
    const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
    const before = await boundsOf(page, id)
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), id)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

    await page.keyboard.press('m')
    await clickWorld(page, [0.5, 0.5, 0])
    await tapCopyModifier(page) // toggle copy mode on
    await expect(page.getByText(/Copy ·/)).toBeVisible()
    await page.keyboard.press('ArrowRight') // lock the X axis
    await moveToWorld(page, [1.2, 0, 0]) // nudge toward +X to give the typed distance its sign
    await page.keyboard.type('2')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    let ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(2) // original + one copy
    let minXs = (await Promise.all(ids.map((i) => boundsOf(page, i)))).map((b) => b[0]).sort((a, b) => a - b)
    expect(minXs[0]).toBeCloseTo(before[0], 5) // original untouched
    expect(minXs[1]).toBeCloseTo(before[0] + 2, 5) // copy 2 m away

    // The array window is hot right after a copy commit: `3x` resolves 3
    // TOTAL copies at the same 2 m spacing, replacing the single copy above
    // as ONE history entry (retracts it first).
    await page.keyboard.type('3x')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(4) // original + 3 array copies
    minXs = (await Promise.all(ids.map((i) => boundsOf(page, i)))).map((b) => b[0]).sort((a, b) => a - b)
    expect(minXs.map((x) => Math.round((x - before[0]) * 1000) / 1000)).toEqual([0, 2, 4, 6])

    // A distance typed AFTER the array re-spaces the whole array (SketchUp):
    // the three copies now sit 3 m apart.
    await page.keyboard.type('3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(4)
    minXs = (await Promise.all(ids.map((i) => boundsOf(page, i)))).map((b) => b[0]).sort((a, b) => a - b)
    expect(minXs.map((x) => Math.round((x - before[0]) * 1000) / 1000)).toEqual([0, 3, 6, 9])

    // And a further count continues from the NEW spacing; then a distance
    // again re-spaces that array.
    await page.keyboard.type('5x')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(6)
    minXs = (await Promise.all(ids.map((i) => boundsOf(page, i)))).map((b) => b[0]).sort((a, b) => a - b)
    expect(minXs.map((x) => Math.round((x - before[0]) * 1000) / 1000)).toEqual([0, 3, 6, 9, 12, 15])
    await page.keyboard.type('1.5')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(6)
    minXs = (await Promise.all(ids.map((i) => boundsOf(page, i)))).map((b) => b[0]).sort((a, b) => a - b)
    expect(minXs.map((x) => Math.round((x - before[0]) * 1000) / 1000)).toEqual([0, 1.5, 3, 4.5, 6, 7.5])

    // The whole array is ONE history entry -- a single undo removes all 5
    // copies and leaves only the original.
    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    const afterUndo = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(afterUndo).toHaveLength(1)
    expect(await boundsOf(page, afterUndo[0])).toEqual(before)
  })
})

// ---------------------------------------------------------------------------
// Rotate
// ---------------------------------------------------------------------------

test.describe('Rotate retype', () => {
  test('typing degrees after a rotation commit redoes it about the same pivot/axis; one undo removes it', async ({
    page,
  }) => {
    await setup(page)
    // A non-square 2x1 footprint so a rotation's effect on the bounding box
    // is unambiguous.
    const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 1, 0], 1))
    const before = await boundsOf(page, id)
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), id)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

    await page.keyboard.press('q')
    await page.keyboard.press('ArrowUp') // force-lock the Z axis (works from idle onward)
    await clickWorld(page, [1, 0.5, 0]) // pivot: the box's own XY center
    await clickWorld(page, [4, 4, 0]) // reference point (irrelevant to a typed commit)
    await page.keyboard.type('90')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    let ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    let b = await boundsOf(page, ids[0])
    // 90 deg about the box's own center swaps the X/Y extents; Z is untouched.
    expect(b[3] - b[0]).toBeCloseTo(1, 5)
    expect(b[4] - b[1]).toBeCloseTo(2, 5)
    expect(b[2]).toBeCloseTo(before[2], 6)
    expect(b[5]).toBeCloseTo(before[5], 6)

    // Retype to 45 deg -- redone from the ORIGINAL (unrotated) box, not
    // cumulative from the 90 deg result: a 2x1 rectangle rotated 45 deg has
    // BOTH extents equal to (2+1)*cos(45) =~ 2.1213, unlike either the
    // original (2x1) or the 90 deg result (1x2).
    await page.keyboard.type('45')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    b = await boundsOf(page, ids[0])
    const expected45 = (2 + 1) * Math.cos(Math.PI / 4)
    expect(b[3] - b[0]).toBeCloseTo(expected45, 3)
    expect(b[4] - b[1]).toBeCloseTo(expected45, 3)

    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    b = await boundsOf(page, await soleObjectId(page))
    expect(b[0]).toBeCloseTo(before[0], 5)
    expect(b[1]).toBeCloseTo(before[1], 5)
    expect(b[3]).toBeCloseTo(before[3], 5)
    expect(b[4]).toBeCloseTo(before[4], 5)
  })
})

// ---------------------------------------------------------------------------
// Scale
// ---------------------------------------------------------------------------

test.describe('Scale retype', () => {
  test('typing a factor or a dimension after a scale commit redoes it about the same pivot/axis; one undo removes it', async ({
    page,
  }) => {
    await setup(page)
    const id = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
    const before = await boundsOf(page, id)
    await page.evaluate((id) => window.__hew_test!.selectObjects([id]), id)
    await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

    await page.keyboard.press('s')
    await page.locator('text=Drag a grip').first().waitFor({ timeout: 5000 })

    // Grab the +Z face grip (a bare click grabs it and enters 'dragging'
    // immediately -- no drag needed since a typed value commits directly).
    await clickWorld(page, [1, 1, 1])
    await page.keyboard.type('2') // bare number -> FACTOR
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)

    let ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    let b = await boundsOf(page, ids[0])
    expect(b[5] - b[2]).toBeCloseTo(2, 5) // 1 m * factor 2
    expect(b[2]).toBeCloseTo(0, 6) // anchored at the opposite (bottom) face
    expect(b[3] - b[0]).toBeCloseTo(2, 5) // X untouched
    expect(b[4] - b[1]).toBeCloseTo(2, 5) // Y untouched

    // Retype with a bare factor again, from IDLE (no re-grab needed) --
    // redone from the ORIGINAL (pre-scale) extent, not cumulative: 1 m * 3 =
    // 3, not 2 * 3.
    await page.keyboard.type('3')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    b = await boundsOf(page, ids[0])
    expect(b[5] - b[2]).toBeCloseTo(3, 5)

    // Retype again with a LENGTH (target dimension), still from idle --
    // 0.75 m target against the ORIGINAL 1 m extent is factor 0.75.
    await page.keyboard.type('0.75m')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(100)
    ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    b = await boundsOf(page, ids[0])
    expect(b[5] - b[2]).toBeCloseTo(0.75, 5)

    await page.evaluate(() => window.__hew_test!.undo())
    await page.waitForTimeout(100)
    b = await boundsOf(page, await soleObjectId(page))
    expect(b).toEqual(before)
  })
})
