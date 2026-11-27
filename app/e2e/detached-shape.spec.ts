/**
 * Detached shapes (docs/design/separate-shapes.md §6.9): a clean tap of
 * Ctrl or Cmd on a draw tool keeps the next shape detached from the face or
 * drawing under it — a free sketch in no plane's arrangement that Push/Pull
 * turns into a new Object. Everything here is driven with REAL pointer and
 * key events (`page.keyboard.press('Control')` is the genuine tap); the
 * harness only pins the camera and reads kernel state back.
 */
import { test, expect, type Page } from '@playwright/test'
import { segmentsPerTurn } from '../src/tools/arcMath'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

async function pagePoint(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

/** Hover a world point the way a person does: arrive, settle, then act. */
async function hover(page: Page, w: V3): Promise<void> {
  const at = await pagePoint(page, w)
  await page.mouse.move(at.x - 3, at.y + 2)
  await page.waitForTimeout(60)
  await page.mouse.move(at.x, at.y)
  await page.waitForTimeout(150)
}

async function click(page: Page, w: V3): Promise<void> {
  await hover(page, w)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
}

async function boot(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setGridVisible(false)
    h.setAxesVisible(false)
  })
}

/** The tool by name (the tool rail's radio). */
async function chooseTool(page: Page, name: string): Promise<void> {
  await page.getByRole('radio', { name }).click()
  await page.waitForTimeout(100)
}

/** The Detached cues that remain (there is no button): the status hint
 *  sentence, and the "+" badge on the tool's cursor. */
const detachedHint = (page: Page) => page.getByText(/Detach is on/).first()
async function expectDetached(page: Page, on: boolean): Promise<void> {
  if (on) await expect(detachedHint(page)).toBeVisible()
  else await expect(page.getByText(/Detach is on/)).toHaveCount(0)
  // cursorFor draws the badge as the path "M26 3v8M21.5 8.5h9" (URL-encoded in the data URL).
  await expect.poll(() => page.evaluate(() => {
    const c = document.querySelector('canvas')
    return c !== null && decodeURIComponent(c.style.cursor).includes('M26 3v8M21.5 8.5h9')
  })).toBe(on)
}

/** What the kernel holds: sketch handles with their segment and region counts. */
async function sketches(page: Page): Promise<{ id: string; segments: number; regions: number }[]> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    return h.getSketchIds().map((id) => ({
      id,
      segments: h.getSketchSegmentsExact(id).length,
      regions: h.getSketchRegionCount(id),
    }))
  })
}

const imprints = (page: Page, object: string) =>
  page.evaluate((o) => window.__hew_test!.getImprints(o), object)

/** A bin-like slab: 7 cm deep (x), 20 cm wide (y), 10 cm tall (z). Its right
 *  face lies on x = 0.07 with outward normal +X; the camera looks straight at
 *  that face, close enough that a 2 cm circle is a comfortable target. The
 *  face's top-front corner is (0.07, 0, 0.1). */
async function binScene(page: Page): Promise<{ box: string; corner: V3 }> {
  await boot(page)
  const box = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([0, 0, 0], [0.07, 0.2, 0], 0.1)
    h.setCamera({ position: [0.67, 0.1, 0.05], target: [0.07, 0.1, 0.05], up: [0, 0, 1], fovDeg: 45 })
    return id
  })
  await page.waitForTimeout(250)
  return { box, corner: [0.07, 0, 0.1] }
}

/** Circle tool, hover the side face, optionally tap Ctrl (Detached), click the
 *  corner, type the 2 cm radius. */
async function cornerCircle(page: Page, corner: V3, detached: boolean): Promise<void> {
  await chooseTool(page, 'Circle')
  await hover(page, [0.07, 0.1, 0.05]) // the face itself
  if (detached) {
    await page.keyboard.press('Control') // a real clean tap
    await expectDetached(page, true)
  }
  await click(page, corner)
  await page.keyboard.type('2cm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
}

test.describe('detached shapes', () => {
  test('1. Getting Started corner circle: a detached disk stays whole, pushes into a cylinder, and the bin is untouched', async ({ page }) => {
    const { box, corner } = await binScene(page)
    const before = (await sketches(page)).length
    expect(await imprints(page, box)).toEqual([])

    await chooseTool(page, 'Circle')
    await hover(page, [0.07, 0.1, 0.05])
    await page.keyboard.press('Control')
    await expectDetached(page, true)
    await expect(page.getByText(/Detach is on/).first()).toBeVisible()
    await click(page, corner)
    await page.keyboard.type('2cm')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

    // The bin gained no imprint; one new sketch holds the whole circle.
    expect(await imprints(page, box)).toEqual([])
    const after = await sketches(page)
    expect(after.length).toBe(before + 1)
    const disk = after[after.length - 1]
    expect(disk.segments).toBe(segmentsPerTurn(0.02))
    expect(disk.regions).toBe(1)
    const flagged = await page.evaluate((id) => {
      const h = window.__hew_test!
      const stores = h.getPlaneUnits(JSON.stringify({ sketch: Number(id) })).units.flatMap((u) => u.edges.map((e) => e.store))
      return stores.every((s) => (s as { detached?: boolean }).detached === true)
    }, disk.id)
    expect(flagged).toBe(true)

    // Push/Pull the disk across the bin: the tie with the face it lies on
    // goes to the detached shape, and the push makes a new Object.
    await chooseTool(page, 'Push/Pull')
    await click(page, [0.07, 0.009, 0.091])
    await page.keyboard.type('-7cm')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

    const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids.length).toBe(2)
    expect(await imprints(page, box)).toEqual([])
    const bounds = await page.evaluate((all) => all.map((id) => window.__hew_test!.getObjectBounds(id)), ids)
    const cutter = bounds.find((b) => b[3] - b[0] < 0.0701 && b[4] - b[1] < 0.05)
    expect(cutter, 'a cylinder about 4 cm across was born').toBeDefined()
    expect(cutter![3] - cutter![0]).toBeCloseTo(0.07, 4) // spans the bin's depth
    expect(cutter![4] - cutter![1]).toBeCloseTo(0.04, 3)
  })

  test('2. Without the tap the default split still happens: the bin gains imprints and a free crescent remains', async ({ page }) => {
    const { box, corner } = await binScene(page)
    const before = (await sketches(page)).length

    await cornerCircle(page, corner, false)
    await expectDetached(page, false)

    expect((await imprints(page, box)).length).toBeGreaterThan(0)
    const after = await sketches(page)
    expect(after.length).toBe(before + 1)
    const crescent = after[after.length - 1]
    expect(crescent.regions).toBeGreaterThan(0)
    // The crescent is NOT a whole circle: the quarter inside the face is face topology.
    expect(crescent.segments).toBeLessThan(segmentsPerTurn(0.02))
    const detached = await page.evaluate((id) => {
      const h = window.__hew_test!
      return h.getPlaneUnits(JSON.stringify({ sketch: Number(id) })).units
        .some((u) => u.edges.some((e) => (e.store as { detached?: boolean }).detached === true))
    }, crescent.id)
    expect(detached).toBe(false)
  })

  test('3. A detached disk moved onto a drawn rectangle on a face stays whole and leaves the rectangle alone', async ({ page }) => {
    await boot(page)
    const box = await page.evaluate(() => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [1, 1, 0], 0.5)
      h.setCamera({ position: [0.5, 0.5, 3], target: [0.5, 0.5, 0.5], up: [0, 1, 0], fovDeg: 45 })
      return id
    })
    await page.waitForTimeout(250)

    // A plain rectangle on the top (default: it becomes face topology).
    await chooseTool(page, 'Rectangle')
    await click(page, [0.55, 0.2, 0.5])
    await click(page, [0.9, 0.5, 0.5])
    expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
    const facePlane = await page.evaluate((o) => {
      const h = window.__hew_test!
      const pick = h.pickFace([0.2, 0.2, 4], [0, 0, -1])
      return pick === null ? null : h.ctxOfFace(o, pick.face)
    }, box)
    expect(facePlane).not.toBeNull()
    const rectBefore = await page.evaluate((c) => JSON.stringify(window.__hew_test!.getPlaneUnits(c!)), facePlane)

    // A detached disk beside it, on the same face, then Move it onto the rectangle.
    await chooseTool(page, 'Circle')
    await hover(page, [0.25, 0.35, 0.5])
    await page.keyboard.press('Control')
    await expectDetached(page, true)
    await click(page, [0.25, 0.35, 0.5])
    await page.keyboard.type('5cm')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
    const diskSketches = (await sketches(page)).filter((s) => s.segments === segmentsPerTurn(0.05))
    expect(diskSketches.length).toBe(1)
    // The face's own drawing (the rectangle's) is exactly what it was.
    expect(await page.evaluate((c) => JSON.stringify(window.__hew_test!.getPlaneUnits(c!)), facePlane)).toBe(rectBefore)

    // Select the disk (a click inside it), then Move it across the rectangle's left edge:
    // base on its centre, X lock, type the distance.
    await chooseTool(page, 'Select')
    await click(page, [0.25, 0.37, 0.5])
    await chooseTool(page, 'Move')
    await click(page, [0.25, 0.35, 0.5])
    await page.keyboard.press('ArrowRight') // lock to the red axis
    await page.keyboard.type('0.33')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(300)
    expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

    // Still one sketch for the disk, still whole, and now straddling the
    // rectangle's edge without having joined it: the rectangle's units are unchanged.
    const moved = (await sketches(page)).filter((s) => s.segments === segmentsPerTurn(0.05))
    expect(moved.length).toBe(1)
    expect(moved[0].regions).toBe(1)
    const centre = await page.evaluate((id) => {
      const segs = window.__hew_test!.getSketchSegmentsExact(id)
      const xs = segs.flatMap((s) => [s[0], s[3]])
      return (Math.min(...xs) + Math.max(...xs)) / 2
    }, moved[0].id)
    expect(centre).toBeCloseTo(0.58, 2)
    expect(await page.evaluate((c) => JSON.stringify(window.__hew_test!.getPlaneUnits(c!)), facePlane)).toBe(rectBefore)
    expect((await imprints(page, box)).length).toBe(1)
  })

  test('4. The key toggles Detached on the draw tools (hint + cursor badge), and switching tools clears it', async ({ page }) => {
    await binScene(page)
    await chooseTool(page, 'Rectangle')
    await expectDetached(page, false)

    await page.keyboard.press('Control')
    await expectDetached(page, true)
    // The same tap turns it back off.
    await page.keyboard.press('Control')
    await expectDetached(page, false)
    await page.keyboard.press('Control')
    await expectDetached(page, true)

    await chooseTool(page, 'Circle')
    await expectDetached(page, false)
    await chooseTool(page, 'Select')
    await expectDetached(page, false)
  })

  test('4b. Releasing Ctrl while Alt is still down (the precision chord) does not toggle Detached; a plain tap does', async ({ page }) => {
    await binScene(page)
    await chooseTool(page, 'Rectangle')
    await expectDetached(page, false)
    await page.keyboard.down('Alt')
    await page.keyboard.down('Control')
    await page.keyboard.up('Control')
    await page.keyboard.up('Alt')
    await expectDetached(page, false)
    await page.keyboard.press('Control')
    await expectDetached(page, true)
  })

  test('5. A detached Line chain across a face edge is one sketch with one region and no imprints', async ({ page }) => {
    await boot(page)
    const box = await page.evaluate(() => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [1, 1, 0], 0.5)
      h.setCamera({ position: [0.5, 0.5, 3], target: [0.5, 0.5, 0.5], up: [0, 1, 0], fovDeg: 45 })
      return id
    })
    await page.waitForTimeout(250)
    const before = (await sketches(page)).length

    await chooseTool(page, 'Line')
    await hover(page, [0.8, 0.2, 0.5])
    await page.keyboard.press('Control')
    await expectDetached(page, true)
    await click(page, [0.8, 0.2, 0.5])
    await click(page, [1.3, 0.2, 0.5]) // past the face's edge
    await click(page, [1.05, 0.8, 0.5])
    await click(page, [0.8, 0.2, 0.5]) // back to the start: the loop closes
    expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

    const after = await sketches(page)
    expect(after.length).toBe(before + 1)
    const tri = after[after.length - 1]
    expect(tri.segments).toBe(3)
    expect(tri.regions).toBe(1)
    expect(await imprints(page, box)).toEqual([])
  })

  test('6. Copy-moving a detached disk makes a second whole sketch, not a joined one', async ({ page }) => {
    await boot(page)
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [0, 0, 3], target: [0, 0, 0], up: [0, 1, 0], fovDeg: 45 })
    })
    await page.waitForTimeout(250)

    await chooseTool(page, 'Circle')
    await hover(page, [0, 0, 0])
    await page.keyboard.press('Control')
    await expectDetached(page, true)
    await click(page, [0, 0, 0])
    await page.keyboard.type('20cm')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    const whole = segmentsPerTurn(0.2)
    expect((await sketches(page)).map((s) => [s.segments, s.regions])).toEqual([[whole, 1]])

    // Move + the platform's copy toggle: the copy overlaps the original.
    await chooseTool(page, 'Select')
    await click(page, [0, 0.05, 0])
    await chooseTool(page, 'Move')
    const mac = await page.evaluate(() => /Mac/.test(navigator.platform))
    await hover(page, [0, 0, 0])
    await page.keyboard.press(mac ? 'Alt' : 'Control') // copy toggle (a clean tap)
    await click(page, [0, 0, 0])
    await page.keyboard.press('ArrowRight')
    await page.keyboard.type('0.15')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(300)
    expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

    // Two sketches, both whole circles that did not arrange with each other.
    expect((await sketches(page)).map((s) => [s.segments, s.regions])).toEqual([[whole, 1], [whole, 1]])
  })
})

test.describe('draw color language (store decides the color)', () => {
  test('a detached disk and a ground sketch are free (blue); a loose line on a face is owned by the solid (orange)', async ({ page }) => {
    await boot(page)
    const box = await page.evaluate(() => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [1, 1, 0], 0.5)
      h.setCamera({ position: [0.5, 0.5, 3], target: [0.5, 0.5, 0.5], up: [0, 1, 0], fovDeg: 45 })
      return id
    })
    await page.waitForTimeout(250)
    const owner = (id: string) => page.evaluate((s) => window.__hew_test!.getSketchOwnerObject(s), id)

    // A rectangle on the ground (beside the box): free.
    await chooseTool(page, 'Rectangle')
    await click(page, [1.5, 0, 0])
    await click(page, [1.8, 0.3, 0])
    let all = await sketches(page)
    expect(all.length).toBe(1)
    expect(await owner(all[0].id)).toBeNull()

    // A loose line on the top face: owned by the box.
    await chooseTool(page, 'Line')
    await click(page, [0.2, 0.2, 0.5])
    await click(page, [0.6, 0.4, 0.5])
    await page.keyboard.press('Escape')
    await page.waitForTimeout(150)
    all = await sketches(page)
    expect(all.length).toBe(2)
    const owners = await Promise.all(all.map((s) => owner(s.id)))
    expect(owners.filter((o) => o === box)).toHaveLength(1)
    expect(owners.filter((o) => o === null)).toHaveLength(1)

    // A detached disk on the same face: free.
    await chooseTool(page, 'Circle')
    await hover(page, [0.5, 0.8, 0.5])
    await page.keyboard.press('Control')
    await click(page, [0.5, 0.8, 0.5])
    await page.keyboard.type('5cm')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(250)
    all = await sketches(page)
    expect(all.length).toBe(3)
    const owners3 = await Promise.all(all.map((s) => owner(s.id)))
    expect(owners3.filter((o) => o === null)).toHaveLength(2)
    expect(owners3.filter((o) => o === box)).toHaveLength(1)
  })
})
