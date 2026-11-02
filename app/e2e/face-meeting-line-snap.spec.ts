import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { test, expect, type Page } from '@playwright/test'

/**
 * Where a face of one Object meets a face of another, the line they make
 * is a virtual edge: On Intersection Line along it, Midpoint at its middle,
 * Intersection at its ends. A slab passing through a wall draws four such
 * lines on the wall's face; before this, hovering any of them offered only
 * On Face until the cursor reached one of the slab's real edges (the
 * reported "you need to get very close to the real edge"). Real pointer
 * events over a pinned camera, modeled on edge-pierce-snap.spec.ts; the
 * second test replays the maintainer's own file.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pointer-driven spec, chromium only')

type P3 = [number, number, number]

interface Ctx {
  rect: { x: number; y: number; width: number; height: number }
}

/** Aim at `target` from `dist` metres along `dir` (pointing from the target
 *  toward the eye), and wait for the render loop to adopt the pose. */
async function aim(page: Page, target: P3, dir: P3, dist: number): Promise<Ctx> {
  const n = Math.hypot(dir[0], dir[1], dir[2])
  const position = target.map((c, i) => c + (dir[i] / n) * dist) as P3
  await page.evaluate(
    (cam) => window.__hew_test!.setCamera(cam),
    { position, target, up: [0, 0, 1] as P3, fovDeg: 45 },
  )
  await page.waitForTimeout(150)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  return { rect: box }
}

async function px(page: Page, ctx: Ctx, w: P3): Promise<{ x: number; y: number }> {
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: ctx.rect.x + p.x, y: ctx.rect.y + p.y }
}

async function hover(page: Page, ctx: Ctx, w: P3, dx = 0, dy = 0): Promise<void> {
  const p = await px(page, ctx, w)
  await page.mouse.move(p.x - 6 + dx, p.y - 4 + dy)
  await page.waitForTimeout(50)
  await page.mouse.move(p.x + dx, p.y + dy)
  await page.waitForTimeout(160)
}

async function chip(page: Page): Promise<string> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll('div'))
      .filter((d) => d.style.zIndex === '20' && d.style.pointerEvents === 'none')
      .map((e) => e.textContent ?? '')
      .join(' | '),
  )
}

async function ready(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
  })
}

/** A point `f` of the way from `a` to `b`. */
const along = (a: P3, b: P3, f: number): P3 => [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]

test('the line where a slab’s top meets a wall’s face snaps as a virtual edge, with a midpoint and intersection ends', async ({ page }, info) => {
  test.setTimeout(120_000)
  await ready(page)
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.drawBox([1, 0, 0], [1.5, 2, 0], 1) // the wall, near face x = 1
    t.drawBox([0.5, 0.6, 0], [1.2, 1.4, 0], 0.5) // a block half a metre tall, 20 cm into the wall
  })
  await page.getByRole('radio', { name: 'Line' }).click()

  // The block's top (z = 0.5) meets the wall's near face along
  // (1, 0.6..1.4, 0.5). Seen from in front of the wall and above.
  const a: P3 = [1.0, 0.6, 0.5]
  const b: P3 = [1.0, 1.4, 0.5]
  const ctx = await aim(page, along(a, b, 0.5), [-2, 0, 0.9], 2.2)

  // A third of the way along: the line, and the cue line is drawn.
  await hover(page, ctx, along(a, b, 1 / 3))
  expect(await chip(page), 'Line, a third along').toContain('On Intersection Line')
  await page.screenshot({ path: info.outputPath('meeting-line-third.png') })
  // Nudged a few pixels off the line, on the wall's face and on the block's
  // top: still the line, never On Face.
  for (const [dx, dy] of [[0, 3], [0, -3], [3, 2], [-3, -2]]) {
    await hover(page, ctx, along(a, b, 0.4), dx, dy)
    const c = await chip(page)
    expect(c, `Line ${dx},${dy} px off`).toContain('On Intersection Line')
    expect(c, `Line ${dx},${dy} px off`).not.toContain('On Face')
  }
  // Its middle is a Midpoint; its end is the piercing (Intersection).
  await hover(page, ctx, along(a, b, 0.5))
  expect(await chip(page), 'the middle').toContain('Midpoint')
  await page.screenshot({ path: info.outputPath('meeting-line-midpoint.png') })
  await hover(page, ctx, a)
  expect(await chip(page), 'the end').toContain('Intersection')
  expect(await chip(page), 'the end').not.toContain('On Intersection Line')
  // Well above the line, mid-wall: just the face.
  await hover(page, ctx, [1.0, 1.0, 0.8])
  expect(await chip(page), 'above the block').toContain('On Face')

  // Tape Measure treats the line like an edge: a click starts a parallel
  // guide along it, Esc cancels it, and a second click places the guide.
  await page.getByRole('radio', { name: 'Tape Measure' }).click()
  await hover(page, ctx, along(a, b, 0.3))
  expect(await chip(page), 'Tape Measure').toContain('On Intersection Line')
  await page.mouse.down()
  await page.mouse.up()
  await expect(page.getByText('Click to place the parallel guide', { exact: false })).toBeVisible()
  await page.keyboard.press('Escape')
  expect(await page.evaluate(() => window.__hew_test!.getGuideIds()), 'Esc leaves no guide').toHaveLength(0)
  await hover(page, ctx, along(a, b, 0.3))
  await page.mouse.down()
  await page.mouse.up()
  await hover(page, ctx, [1.0, 1.0, 0.8]) // pull straight up the wall, 0.3 m off the line
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  const guide = await page.evaluate(() => {
    const t = window.__hew_test!
    return t.getGuideIds().map((id) => t.getGuideGeometry(id))
  })
  expect(guide, 'one parallel guide').toHaveLength(1)
  const [gx, , gz, , gdy] = [guide[0]![0], guide[0]![1], guide[0]![2], guide[0]![3], guide[0]![4]]
  expect(Math.abs(gdy), 'parallel to the line (along Y)').toBeCloseTo(1, 6)
  expect(gx, 'stays in the wall plane').toBeCloseTo(1.0, 3)
  expect(gz, 'offset from the line (z = 0.5) by the pull').toBeCloseTo(0.8, 1)
  await page.evaluate(() => window.__hew_test!.undo())
  expect(await page.evaluate(() => window.__hew_test!.getGuideIds()), 'undo removes it').toHaveLength(0)

  // Line: a click on the line starts the line exactly on it (x = 1, z = 0.5).
  await page.getByRole('radio', { name: 'Line' }).click()
  const before = await page.evaluate(() => window.__hew_test!.getSketchIds())
  await hover(page, ctx, along(a, b, 0.3))
  expect(await chip(page)).toContain('On Intersection Line')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  await hover(page, ctx, [1.0, 1.0, 0.8])
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  await page.keyboard.press('Escape')
  const points = await page.evaluate((before) => {
    const t = window.__hew_test!
    const sketched = t.getSketchIds().filter((id) => !before.includes(id))
      .flatMap((id) => t.getSketchSegmentsExact(id)).flatMap((s) => [s.slice(0, 3), s.slice(3, 6)])
    const imprinted = t.getObjectIds().flatMap((o) => t.getImprints(o).flatMap((i) => i.loop))
    return [...sketched, ...imprinted]
  }, before)
  const onLine = points.some((p) => Math.abs(p[0] - 1.0) < 1e-9 && Math.abs(p[2] - 0.5) < 1e-9 && p[1] > 0.6 && p[1] < 1.4)
  expect(onLine, 'the drawn line starts on the meeting line').toBe(true)
})

const FIXTURE = fileURLToPath(new URL('./fixtures/intersecting-faces.hew', import.meta.url))

/**
 * The maintainer's file: a vertical plate (x 0..0.0195) and a horizontal
 * plate (z 0.02429..0.02889, y 0.03355..0.07355, x 0.00067..0.03837)
 * crossing it. The horizontal plate's four faces meet the vertical plate's
 * +x face (x = 0.0195) along four lines; each is hovered a quarter of the
 * way along from a camera that sees it, close enough that the quarter
 * station is clear of both the end's and the middle's point reach (the
 * side lines are 4.6 mm long: from further out the whole line is inside
 * the reach of its ends and middle, which is the correct answer there).
 */
test('the four lines where the maintainer’s plates meet each snap as a virtual edge, and Tape Measure guides off one', async ({ page }, info) => {
  test.setTimeout(120_000)
  await ready(page)
  await page.evaluate((arr) => window.__hew_test!.load(arr), Array.from(readFileSync(FIXTURE)))
  await expect.poll(async () => page.evaluate(() => window.__hew_test!.getObjectIds().length)).toBe(2)
  await page.getByRole('radio', { name: 'Line' }).click()

  const X = 0.0195
  const [Y0, Y1] = [0.03355001504416445, 0.07355001504416445]
  const [Z0, Z1] = [0.02429174593526217, 0.02889174593526217]
  const lines: { name: string; a: P3; b: P3; dir: P3; dist: number }[] = [
    { name: 'top', a: [X, Y0, Z1], b: [X, Y1, Z1], dir: [2, 0, 0.9], dist: 0.12 },
    { name: 'bottom', a: [X, Y0, Z0], b: [X, Y1, Z0], dir: [2, 0, -0.9], dist: 0.12 },
    { name: 'near side', a: [X, Y0, Z0], b: [X, Y0, Z1], dir: [2, -1.2, 0], dist: 0.05 },
    { name: 'far side', a: [X, Y1, Z0], b: [X, Y1, Z1], dir: [2, 1.2, 0], dist: 0.05 },
  ]
  for (const { name, a, b, dir, dist } of lines) {
    const ctx = await aim(page, along(a, b, 0.5), dir, dist)
    await hover(page, ctx, along(a, b, 0.25))
    expect(await chip(page), name).toContain('On Intersection Line')
    await page.screenshot({ path: info.outputPath(`intersecting-faces-${name.replace(' ', '-')}.png`) })
    await hover(page, ctx, along(a, b, 0.5))
    expect(await chip(page), `${name} middle`).toContain('Midpoint')
    // Anywhere along the line between its ends' and middle's point reach,
    // and a few pixels to either side of it (inside the 8 px acquire
    // radius), the cursor is on the line, not the face it crosses.
    const pa = await px(page, ctx, a)
    const pb = await px(page, ctx, b)
    const len = Math.hypot(pb.x - pa.x, pb.y - pa.y)
    const [nx, ny] = [-(pb.y - pa.y) / len, (pb.x - pa.x) / len]
    for (const t of [0.2, 0.3, 0.7, 0.8]) {
      for (const off of [-5, 5]) {
        await hover(page, ctx, along(a, b, t), nx * off, ny * off)
        const c = await chip(page)
        expect(c, `${name} at ${t}, ${off} px across`).toContain('On Intersection Line')
        expect(c, `${name} at ${t}, ${off} px across`).not.toContain('On Face')
      }
    }
  }

  // Tape Measure on the maintainer's top line: click on it, pull the cursor
  // 1 cm up the plate, click: a guide parallel to the line (along Y) at that
  // offset.
  await page.getByRole('radio', { name: 'Tape Measure' }).click()
  const top = lines[0]
  const ctxTop = await aim(page, along(top.a, top.b, 0.5), top.dir, top.dist)
  await hover(page, ctxTop, along(top.a, top.b, 0.25))
  expect(await chip(page), 'Tape Measure on the top line').toContain('On Intersection Line')
  await page.mouse.down()
  await page.mouse.up()
  await expect(page.getByText('Click to place the parallel guide', { exact: false })).toBeVisible()
  await hover(page, ctxTop, [X, (Y0 + Y1) / 2, Z1 + 0.01])
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  const g = await page.evaluate(() => {
    const t = window.__hew_test!
    return t.getGuideIds().map((id) => t.getGuideGeometry(id))
  })
  expect(g, 'one guide').toHaveLength(1)
  expect(Math.abs(g[0]![4]), 'parallel to the top line (along Y)').toBeCloseTo(1, 6)
  expect(g[0]![2], 'offset 1 cm above the line').toBeCloseTo(Z1 + 0.01, 3)
  expect(g[0]![0], 'in the plate’s face').toBeCloseTo(X, 4)
})
