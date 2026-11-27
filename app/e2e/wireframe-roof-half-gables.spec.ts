import { test, expect, type Page } from '@playwright/test'

/**
 * Kurt's roof defect (docs/design/wireframe-faces-and-solids.md section 16,
 * R24), driven through the REAL Line tool in Detached mode: a unit box
 * Object, a 0.5 m ridge post straight up from the midpoint of the top edge at
 * y = 0 and another at y = 1, four detached rafters (post top to a top box
 * corner, each its own gesture; near corner first at the y = 0 end, far
 * corner first at the y = 1 end), then the detached ridge from post top to
 * post top. After the second rafter at each end both half-gable triangles
 * must be faces; the ridge closes the roof into a second Object (a solid
 * triangular prism, 1 x 1 x 0.5 over the top of the box).
 *
 * Real pointer and key events; the harness only pins the camera and reads
 * kernel state back.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pixel-accurate picking is only stable on pinned SwiftShader')

type V3 = [number, number, number]

// From the front-right and above, so every top corner, both post tops and
// the whole ridge are in plain view.
const CAMERA: V3 = [3.2, -2.6, 3.0]
const TARGET: V3 = [0.5, 0.5, 1.2]

const CHIP =
  /^(Endpoint|Midpoint|Intersection|Quadrant|Tangent|Center|On Edge|On Face|On Guide|On Axis|On Plane|Ground|From Point)/

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(
    ({ position, target }) => {
      const t = window.__hew_test!
      t.setGridVisible(false)
      t.setAxesVisible(false)
      t.setCamera({ position, target, up: [0, 0, 1], fovDeg: 45 })
    },
    { position: CAMERA, target: TARGET },
  )
  await page.waitForTimeout(250)
})

async function toPage(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const p = await page.evaluate((world) => window.__hew_test!.worldToScreen(world as V3), w)
  return { x: canvas.x + p.x, y: canvas.y + p.y }
}

/** The inference chip's label under the cursor, or '' when none shows. */
async function chip(page: Page): Promise<string> {
  const texts = await page.locator('div:has(> span)').allTextContents()
  const hit = texts.map((t) => t.trim()).find((t) => CHIP.test(t))
  return hit === undefined ? '' : (CHIP.exec(hit) ?? [''])[0]
}

/** Hover `w` until the chip reads `label` (the idle snap settles over a few frames). */
async function hoverSnap(page: Page, w: V3, label: 'Endpoint' | 'Midpoint'): Promise<void> {
  const p = await toPage(page, w)
  await page.mouse.move(p.x + 5, p.y + 4)
  await expect
    .poll(
      async () => {
        await page.mouse.move(p.x + 1, p.y)
        await page.mouse.move(p.x, p.y)
        await page.waitForTimeout(60)
        return chip(page)
      },
      { timeout: 10_000, message: `the chip never read ${label} over ${JSON.stringify(w)}` },
    )
    .toBe(label)
}

async function press(page: Page): Promise<void> {
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
}

async function clickSnap(page: Page, w: V3, label: 'Endpoint' | 'Midpoint'): Promise<void> {
  await hoverSnap(page, w, label)
  await press(page)
}

/** Detached mode is a Ctrl/Cmd tap; it may or may not survive Esc, so tap
 *  only once the status hint is confirmed absent. */
async function ensureDetached(page: Page): Promise<void> {
  const hint = page.getByText(/Detach is on/).first()
  const shown = await hint.waitFor({ state: 'visible', timeout: 500 }).then(
    () => true,
    () => false,
  )
  if (!shown) {
    await page.keyboard.press('Control') // a real clean tap
  }
  await expect(hint).toBeVisible({ timeout: 5_000 })
}

/** A detached ridge post: click the edge midpoint, lock Blue, type 0.5 m, end the chain. */
async function post(page: Page, mid: V3): Promise<V3> {
  await ensureDetached(page)
  await clickSnap(page, mid, 'Midpoint')
  await page.keyboard.press('ArrowUp')
  const top: V3 = [mid[0], mid[1], mid[2] + 0.5]
  const p = await toPage(page, [mid[0], mid[1], mid[2] + 0.25])
  await page.mouse.move(p.x + 2, p.y)
  await page.mouse.move(p.x, p.y, { steps: 3 })
  await page.waitForTimeout(100)
  await page.keyboard.type('0.5m')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(120)
  await page.keyboard.press('Escape')
  await page.waitForTimeout(100)
  return top
}

/** A detached Line from `a` to `b`, both Endpoint snaps, one gesture. */
async function line(page: Page, a: V3, b: V3, endChain = true): Promise<void> {
  await ensureDetached(page)
  await clickSnap(page, a, 'Endpoint')
  await clickSnap(page, b, 'Endpoint')
  if (endChain) await page.keyboard.press('Escape')
  await page.waitForTimeout(100)
}

/** Faces (regions) over every live sketch: each half-gable triangle is one. */
function regionCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const t = window.__hew_test!
    return t.getSketchIds().reduce((n, id) => n + t.getSketchRegionCount(id), 0)
  })
}

function boxBounds(page: Page, id: string): Promise<number[]> {
  return page.evaluate((o) => [...window.__hew_test!.getObjectBounds(o)], id)
}

type Step = 'f-near' | 'f-far' | 'b-near' | 'b-far'

const ORDERS: { name: string; steps: Step[] }[] = [
  { name: 'front near-first, back far-first (the reported order)', steps: ['f-near', 'f-far', 'b-far', 'b-near'] },
  { name: 'front far-first, back near-first (the other order at each end)', steps: ['f-far', 'f-near', 'b-near', 'b-far'] },
  { name: 'rafters interleaved across the two ends', steps: ['f-near', 'b-far', 'f-far', 'b-near'] },
]

for (const order of ORDERS) {
  test(`Kurt's roof, ${order.name}: posts, four detached rafters and a ridge make a second solid Object`, async ({ page }) => {
    test.setTimeout(150_000)

    const boxId = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
    await page.waitForTimeout(200)
    expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
    const before = await boxBounds(page, boxId)

    await page.getByRole('radio', { name: 'Line' }).click()
    await page.waitForTimeout(100)

    const frontTop = await post(page, [0.5, 0, 1])
    const backTop = await post(page, [0.5, 1, 1])
    expect(await regionCount(page), 'two posts are lines, not faces').toBe(0)

    const target: Record<Step, [V3, V3]> = {
      'f-near': [frontTop, [0, 0, 1]],
      'f-far': [frontTop, [1, 0, 1]],
      'b-near': [backTop, [0, 1, 1]],
      'b-far': [backTop, [1, 1, 1]],
    }
    const done = new Set<Step>()
    for (const step of order.steps) {
      await line(page, ...target[step])
      done.add(step)
      expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
      // Each end fills the moment ITS second rafter lands, two triangles per end.
      const front = done.has('f-near') && done.has('f-far') ? 2 : 0
      const back = done.has('b-near') && done.has('b-far') ? 2 : 0
      expect(await regionCount(page), `faces after ${[...done].join(', ')}`).toBe(front + back)
    }
    expect(await page.evaluate(() => window.__hew_test!.getObjectCount()), 'four triangles are not yet a closed shape').toBe(1)

    // The ridge closes the roof into its own Object (no Esc: a closing Line ends its chain).
    await line(page, frontTop, backTop, false)
    await expect(page.getByText('The closed shape is now a solid.')).toBeVisible({ timeout: 10_000 })

    const result = await page.evaluate(() => {
      const t = window.__hew_test!
      return {
        error: t.getLastError(),
        objects: t.getObjectIds(),
        solid: t.getObjectIds().map((id) => t.isObjectSolid(id)),
        bounds: t.getObjectIds().map((id) => [...t.getObjectBounds(id)]),
      }
    })
    expect(result.error).toBeNull()
    expect(result.objects, 'the box and the roof, nothing else').toHaveLength(2)
    expect(result.solid).toEqual([true, true])

    const boxIdx = result.objects.indexOf(boxId)
    expect(boxIdx).toBeGreaterThanOrEqual(0)
    for (let i = 0; i < 6; i++) expect(result.bounds[boxIdx][i]).toBeCloseTo(before[i], 5)
    const roof = result.bounds[1 - boxIdx]
    const want = [0, 0, 1, 1, 1, 1.5]
    for (let i = 0; i < 6; i++) expect(roof[i]).toBeCloseTo(want[i], 5)
  })
}
