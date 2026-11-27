import { test, expect, type Page } from '@playwright/test'

/**
 * Where an edge of one Object passes through a face of another, the
 * piercing snaps as an Intersection. A thin panel set into a wall with
 * nothing imprinted on the wall: hovering where the panel's top edge enters
 * the wall's face used to flicker between On Face and On Edge and never
 * offer a point. Real pointer events over a pinned camera; Tape Measure
 * measures from the point (a plain edge point still starts a parallel
 * guide, as in edge-crossing-snap.spec.ts).
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pointer-driven spec, chromium only')

/** The panel's top-front edge (y = 0.9, z = 0.6) enters the wall's near
 *  face (x = 1) here, well inside the face and far from every vertex. */
const PIERCE: [number, number, number] = [1.0, 0.9, 0.6]

interface Ctx {
  rect: { x: number; y: number; width: number; height: number }
}

/** In front of the wall, left of the panel and above it, `dist` metres
 *  from the piercing: nothing stands between the eye and the point. */
async function aim(page: Page, dist: number): Promise<Ctx> {
  const dir = [-2, -0.6, 0.9]
  const n = Math.hypot(dir[0], dir[1], dir[2])
  const position = PIERCE.map((c, i) => c + (dir[i] / n) * dist) as [number, number, number]
  await page.evaluate(
    (cam) => window.__hew_test!.setCamera(cam),
    { position, target: PIERCE, up: [0, 0, 1] as [number, number, number], fovDeg: 45 },
  )
  await page.waitForTimeout(150)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  return { rect: box }
}

async function px(page: Page, ctx: Ctx, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: ctx.rect.x + p.x, y: ctx.rect.y + p.y }
}

async function hover(page: Page, ctx: Ctx, w: [number, number, number], dx = 0, dy = 0): Promise<void> {
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

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const t = window.__hew_test!
    t.setGridVisible(false)
    t.setAxesVisible(false)
    t.drawBox([1, 0, 0], [1.5, 2, 0], 1) // the wall, near face x = 1
    t.drawBox([0, 0.9, 0], [1.3, 1.0, 0], 0.6) // the panel, 30 cm into the wall
  })
})

test('where a panel’s edge enters a wall’s face the piercing snaps, and Tape Measure measures from it', async ({ page }) => {
  test.setTimeout(120_000)
  await page.getByRole('radio', { name: 'Line' }).click()

  // Dead on, and a ring of cursor positions a few pixels around the point
  // over the wall's face and the panel's edge: every one of them is the
  // piercing, none is On Face or On Edge (the reported flicker).
  const normal = await aim(page, 2.0)
  await hover(page, normal, PIERCE)
  expect(await chip(page), 'Line dead on').toContain('Intersection')
  for (const [dx, dy] of [[4, 0], [-4, 0], [0, 4], [0, -4], [3, 3], [-3, -3], [3, -3], [-3, 3]]) {
    await hover(page, normal, PIERCE, dx, dy)
    const c = await chip(page)
    expect(c, `Line ${dx},${dy} px off`).toContain('Intersection')
    expect(c, `Line ${dx},${dy} px off`).not.toContain('On Face')
    expect(c, `Line ${dx},${dy} px off`).not.toContain('On Edge')
  }
  // A piercing reaches as far as any named point (14 px against the 8 px
  // pick radius): 10 px off it, the piercing still wins over the face.
  await hover(page, normal, PIERCE, 8, -6)
  expect(await chip(page), 'Line 10 px off the piercing').toContain('Intersection')

  // Tape Measure: the piercing snaps the same way, the hint names what the
  // person sees — an edge meeting a face, not two edges crossing (the
  // edge × edge wording is edge-crossing-snap.spec.ts's) — and the click
  // starts a measurement rather than a parallel guide off the panel's edge.
  await page.getByRole('radio', { name: 'Tape Measure' }).click()
  await hover(page, normal, PIERCE, 2, 1)
  expect(await chip(page), 'Tape Measure at normal zoom').toContain('Intersection')
  await expect(page.getByText('Click to measure from where the edge meets the face.', { exact: true })).toBeVisible()
  expect(await page.getByText('Click to measure from the crossing.', { exact: true }).count()).toBe(0)
  await page.mouse.down()
  await page.mouse.up()
  await expect(page.getByText('Click the second point to read the distance', { exact: false })).toBeVisible()
  // Second point on the wall's face, clear of the panel: a measurement, no guide.
  const onWall: [number, number, number] = [1.0, 0.6, 0.8]
  await hover(page, normal, onWall)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
  expect(await page.evaluate(() => window.__hew_test!.getGuideIds())).toHaveLength(0)
  await expect(page.getByText('Type a distance and press Enter to resize the model', { exact: false })).toBeVisible()
  await page.keyboard.press('Escape')

  // A plain point on the panel's top edge, away from the piercing, still
  // starts a parallel guide along that edge.
  const onEdge: [number, number, number] = [0.6, 0.9, 0.6]
  await hover(page, normal, onEdge)
  expect(await chip(page)).not.toContain('Intersection')
  await page.mouse.down()
  await page.mouse.up()
  await expect(page.getByText('Click to place the parallel guide', { exact: false })).toBeVisible()
  await page.keyboard.press('Escape')

  // Line: a click on the piercing starts the line exactly there.
  await page.getByRole('radio', { name: 'Line' }).click()
  const before = await page.evaluate(() => window.__hew_test!.getSketchIds())
  await hover(page, normal, PIERCE, 2, 1)
  expect(await chip(page)).toContain('Intersection')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(120)
  await hover(page, normal, onWall)
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
  const hit = points.some((p) => Math.hypot(p[0] - PIERCE[0], p[1] - PIERCE[1], p[2] - PIERCE[2]) < 1e-9)
  expect(hit, 'the drawn line starts exactly at the piercing').toBe(true)
})
