import { test, expect, type Locator, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * The view die and its pips (docs/design/view-die.md §2, §3), driven with real
 * mouse events. Camera assertions read the live camera state; the die is read
 * through its DOM (`data-*` attributes and the SVG polygons) and clicked by
 * the page position of those polygons.
 */

type V3 = [number, number, number]

const HOME: V3 = [1, -1, 1]

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    window.__hew_test!.drawBox([0, 0, 0], [1, 1, 1], 1)
  })
}

async function lookFrom(page: Page, dir: V3, dist = 9): Promise<void> {
  const len = Math.hypot(...dir)
  const position = dir.map((c) => (c / len) * dist) as V3
  await page.evaluate(
    (position) => window.__hew_test!.setCamera({ position, target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 }),
    position,
  )
  await expect(page.getByTestId('view-die')).toHaveAttribute('data-draw-active', /true|false/)
  await page.waitForTimeout(120)
}

interface Pose { dir: V3; dist: number }

async function pose(page: Page): Promise<Pose> {
  const s = await page.evaluate(() => window.__hew_test!.getCameraState())
  const d = [s.eye[0] - s.target[0], s.eye[1] - s.target[1], s.eye[2] - s.target[2]]
  const dist = Math.hypot(d[0]!, d[1]!, d[2]!)
  return { dir: d.map((c) => c / dist) as V3, dist }
}

function near(a: V3, b: V3, tol = 0.01): boolean {
  const lb = Math.hypot(...b)
  return a.every((c, i) => Math.abs(c - b[i]! / lb) < tol)
}

async function expectLookingFrom(page: Page, dir: V3): Promise<void> {
  await expect.poll(async () => near((await pose(page)).dir, dir), { timeout: 3000 }).toBe(true)
}

async function center(l: Locator): Promise<{ x: number; y: number }> {
  const b = (await l.boundingBox())!
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 }
}

const face = (page: Page, id: string): Locator => page.locator(`[data-testid="view-die-svg"] [data-face="${id}"]`)
const facet = (page: Page, c: string): Locator => page.locator(`[data-testid="view-die-svg"] [data-facet="${c}"]`)

async function clickLocator(page: Page, l: Locator, steps = 3): Promise<void> {
  const p = await center(l)
  await page.mouse.move(p.x, p.y, { steps })
  await page.mouse.down()
  await page.mouse.up()
}

test.describe('view die', () => {
  test('sits top right with four pips and no old view chips', async ({ page }) => {
    await setup(page)
    await expect(page.getByTestId('view-die')).toBeVisible()
    await expect(page.getByTestId('viewport-hud')).toHaveCount(0)
    for (const pip of ['red', 'green', 'blue', 'custom']) {
      await expect(page.getByTestId(`view-die-pip-${pip}`)).toBeVisible()
      await expect(page.getByTestId(`view-die-pip-${pip}`)).toHaveAttribute('data-state', 'idle')
    }
    const die = (await page.getByTestId('view-die-svg').boundingBox())!
    const vp = (await page.locator('canvas').first().boundingBox())!
    expect(die.width).toBe(76)
    expect(die.y - vp.y).toBeCloseTo(16, 0)
  })

  test('turns with the camera and draws a dashed outline off a standard view', async ({ page }) => {
    await setup(page)
    await lookFrom(page, [0.4, -1, 0.5])
    await expect(page.getByTestId('view-die-silhouette')).toHaveAttribute('data-solid', 'false')
    await lookFrom(page, [0, -1, 0])
    await expect(page.getByTestId('view-die-silhouette')).toHaveAttribute('data-solid', 'true')
    await expect(page.getByTestId('view-die')).toHaveAttribute('data-view', 'front')
  })

  test('a face click turns to that view, keeps the distance, and a second click flips', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    const before = await pose(page)
    await clickLocator(page, face(page, 'front'))
    await expectLookingFrom(page, [0, -1, 0])
    expect((await pose(page)).dist).toBeCloseTo(before.dist, 2)
    await expect(page.getByTestId('view-die')).toHaveAttribute('data-view', 'front')
    // Second click on the face the camera is on: the opposite face.
    await clickLocator(page, face(page, 'front'))
    await expectLookingFrom(page, [0, 1, 0])
  })

  test('the step triangles walk to the adjacent face', async ({ page }) => {
    await setup(page)
    await lookFrom(page, [0, -1, 0])
    await clickLocator(page, page.getByTestId('view-die-step-right'))
    await expectLookingFrom(page, [1, 0, 0])
  })

  test('a facet click reaches every one of the eight corner views', async ({ page }) => {
    await setup(page)
    for (const sx of [-1, 1]) {
      for (const sy of [-1, 1]) {
        for (const sz of [-1, 1]) {
          // A view that shows that corner's facet, off the corner direction.
          await lookFrom(page, [sx * 1.0, sy * 1.5, sz * 0.7])
          await clickLocator(page, facet(page, `${sx},${sy},${sz}`))
          await expectLookingFrom(page, [sx, sy, sz])
          await expect(page.getByTestId('view-die-silhouette')).toHaveAttribute('data-solid', 'true')
        }
      }
    }
  })

  test('the pole views stay upright (Top keeps +Z up)', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    await clickLocator(page, face(page, 'top'))
    await expectLookingFrom(page, [0, 0, 1])
    // The tween's last frame lands on the exact state; mid-flight the up
    // vector is a slerp residue, so wait for the landing rather than read it
    // the moment the direction is within tolerance.
    await expect
      .poll(async () => (await page.evaluate(() => window.__hew_test!.getCameraState())).up, { timeout: 3000 })
      .toEqual([0, 0, 1])
  })

  test('hovering names the target in a pill, neutrally highlighted', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    const p = await center(face(page, 'front'))
    await page.mouse.move(p.x, p.y, { steps: 3 })
    await expect(page.getByTestId('view-die-tooltip')).toHaveText('Front')
    await expect(page.getByTestId('view-die-hover')).toBeVisible()
    const c = await center(facet(page, '1,-1,1'))
    await page.mouse.move(c.x, c.y, { steps: 3 })
    await expect(page.getByTestId('view-die-tooltip')).toHaveText('Iso · front right')
  })

  test('a drag on the die orbits; a 2 px press is a click', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    const p = await center(face(page, 'top'))
    const before = (await pose(page)).dir
    await page.mouse.move(p.x, p.y)
    await page.mouse.down()
    await page.mouse.move(p.x + 40, p.y + 5, { steps: 8 })
    await page.mouse.up()
    await page.waitForTimeout(400)
    const after = (await pose(page)).dir
    expect(Math.hypot(after[0] - before[0], after[1] - before[1])).toBeGreaterThan(0.2)
    await expect(page.getByTestId('view-die-silhouette')).toHaveAttribute('data-solid', 'false')

    // A press that moves 2 px is a click: it turns to the face under it.
    await lookFrom(page, HOME)
    const q = await center(face(page, 'right'))
    await page.mouse.move(q.x, q.y)
    await page.mouse.down()
    await page.mouse.move(q.x + 2, q.y, { steps: 2 })
    await page.mouse.up()
    await expectLookingFrom(page, [1, 0, 0])
  })

  test('a drag that starts on the die and ends over the canvas leaves orbit usable', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    const p = await center(face(page, 'top'))
    await page.mouse.move(p.x, p.y)
    await page.mouse.down()
    await page.mouse.move(p.x - 300, p.y + 100, { steps: 10 })
    await page.mouse.up()
    // A middle-button orbit on the canvas still works afterwards.
    const c = (await page.locator('canvas').first().boundingBox())!
    const mid = { x: c.x + c.width / 2, y: c.y + c.height / 2 }
    const b = (await pose(page)).dir
    await page.mouse.move(mid.x, mid.y)
    await page.mouse.down({ button: 'middle' })
    await page.mouse.move(mid.x + 60, mid.y, { steps: 6 })
    await page.mouse.up({ button: 'middle' })
    await page.waitForTimeout(400)
    const a = (await pose(page)).dir
    expect(Math.hypot(a[0] - b[0], a[1] - b[1])).toBeGreaterThan(0.05)
  })

  test('a click on the die closes an open menu', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    await page.getByRole('button', { name: 'Camera' }).click()
    await expect(page.getByText('Parallel Projection')).toBeVisible()
    await clickLocator(page, face(page, 'top'))
    await expect(page.getByText('Parallel Projection')).toHaveCount(0)
  })

  test('the transparent corners of the die box belong to the canvas', async ({ page }) => {
    await setup(page)
    await lookFrom(page, [0, -1, 0])
    const box = (await page.getByTestId('view-die-svg').boundingBox())!
    // A face-on cube leaves the box corners empty.
    const tag = await page.evaluate(
      ([x, y]) => document.elementFromPoint(x!, y!)?.tagName,
      [box.x + 2, box.y + 2],
    )
    expect(tag).toBe('CANVAS')
    const centre = await page.evaluate(
      ([x, y]) => document.elementFromPoint(x!, y!)?.closest('[data-testid="view-die-svg"]') !== null,
      [box.x + box.width / 2, box.y + box.height / 2],
    )
    expect(centre).toBe(true)
  })

  test('a click on the die does not take keyboard focus', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
    const before = await page.evaluate(() => document.activeElement?.tagName)
    await clickLocator(page, face(page, 'top'))
    expect(await page.evaluate(() => document.activeElement?.tagName)).toBe(before)
    const pip = await center(page.getByTestId('view-die-pip-red'))
    await page.mouse.click(pip.x, pip.y)
    expect(await page.evaluate(() => document.activeElement?.tagName)).toBe(before)
  })
})

test.describe('view die pips', () => {
  async function toPage(page: Page, w: V3): Promise<{ x: number; y: number }> {
    const canvas = (await page.locator('canvas').first().boundingBox())!
    const p = await page.evaluate((world) => window.__hew_test!.worldToScreen(world), w)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  async function hover(page: Page, w: V3): Promise<void> {
    const p = await toPage(page, w)
    await page.mouse.move(p.x + 5, p.y + 5)
    await page.mouse.move(p.x, p.y, { steps: 3 })
    await page.waitForTimeout(100)
  }
  const pipState = (page: Page, pip: string): Locator => page.getByTestId(`view-die-pip-${pip}`)

  test('a Select-tool pip click does nothing and shows not-allowed', async ({ page }) => {
    await setup(page)
    await lookFrom(page, HOME)
    const pip = pipState(page, 'blue')
    await expect(pip).toHaveCSS('cursor', 'not-allowed')
    await clickLocator(page, pip)
    for (const id of ['red', 'green', 'blue', 'custom']) await expect(pipState(page, id)).toHaveAttribute('data-state', 'idle')
  })

  test('a pip click locks, shows the padlock, survives the pointer on the widget, and releases', async ({ page }) => {
    await setup(page)
    await lookFrom(page, [0, -1, 0.15])
    await page.keyboard.press('r')
    await hover(page, [3, 0, 0.5])
    await expect(pipState(page, 'green')).toHaveAttribute('data-state', 'inferred')
    await expect(page.getByTestId('view-die-fill-front')).toBeVisible()

    await clickLocator(page, pipState(page, 'blue'))
    await expect(pipState(page, 'blue')).toHaveAttribute('data-state', 'locked')
    await expect(page.getByTestId('view-die-lock-blue')).toBeVisible()
    await expect(page.getByTestId('view-die-ring-blue')).toBeVisible()
    // The pointer is still on the widget: the lock is still shown.
    await expect(pipState(page, 'blue')).toHaveAttribute('data-state', 'locked')
    await expect(page.getByTestId('view-die-tooltip')).toContainText('Release')

    // The arrow keys still work with no extra click in the viewport.
    await page.keyboard.press('ArrowLeft')
    await expect(pipState(page, 'green')).toHaveAttribute('data-state', 'locked')
    await clickLocator(page, pipState(page, 'green'))
    await expect(pipState(page, 'green')).not.toHaveAttribute('data-state', 'locked')
  })

  test('an arrow mid-gesture updates the chip\'s plane word and padlock at once, with the pointer still', async ({ page }) => {
    await setup(page)
    await lookFrom(page, [0, -1, 0.15])
    await page.keyboard.press('r')
    const a = await toPage(page, [3, 0, 0.5])
    await page.mouse.move(a.x, a.y)
    await page.mouse.click(a.x, a.y)
    await hover(page, [4, 0, 1.2])
    const word = page.getByTestId('inference-tip-plane')
    await expect(word).toContainText('green')
    await expect(page.getByTestId('inference-tip-padlock')).toHaveCount(0)
    await page.keyboard.press('ArrowUp') // blue, through the first corner
    await expect(word).toContainText('blue')
    await expect(page.getByTestId('inference-tip-padlock')).toBeVisible()
    await page.keyboard.press('Escape')
  })

  test('a pip click during a gesture keeps the gesture live', async ({ page }) => {
    await setup(page)
    await lookFrom(page, [0, -1, 0.15])
    await page.keyboard.press('r')
    const a = await toPage(page, [3, 0, 0.5])
    await page.mouse.move(a.x, a.y)
    await page.mouse.click(a.x, a.y)
    await hover(page, [4, 0, 1.5])
    await clickLocator(page, face(page, 'top'))
    await expect(pipState(page, 'green')).toHaveAttribute('data-state', /anchored|locked|pinned/)
  })

  test('the custom pip pins a rotated cube\'s face and releases it', async ({ page }) => {
    await setup(page)
    const id = await page.evaluate(() => {
      const h = window.__hew_test!
      const b = h.drawBox([4, 0, 0], [5, 1, 1], 1)
      h.rotateObject(b, 30, [0, 0, 1])
      return b
    })
    expect(id).toBeTruthy()
    await page.evaluate(() =>
      window.__hew_test!.setCamera({ position: [6, -6, 3.5], target: [3.9, 2.25, 0.5], up: [0, 0, 1], fovDeg: 45 }),
    )
    await page.waitForTimeout(150)
    await page.keyboard.press('r')
    // The cube's centre (4.5, 0.5) turned 30 degrees about the origin, then
    // half an edge out along the rotated -Y normal (sin 30, -cos 30): the
    // middle of the side face nearest the camera.
    const c = Math.cos(Math.PI / 6)
    const sn = Math.sin(Math.PI / 6)
    const mid: V3 = [4.5 * c - 0.5 * sn + 0.5 * sn, 4.5 * sn + 0.5 * c - 0.5 * c, 0.5]
    await hover(page, mid)
    await expect(pipState(page, 'custom')).toHaveAttribute('data-state', 'inferred')
    // One jump from the face onto the pip: the tracked plane is the face's,
    // not whatever lies on a path across the canvas.
    await clickLocator(page, pipState(page, 'custom'), 1)
    await expect(pipState(page, 'custom')).toHaveAttribute('data-state', 'pinned')
    await clickLocator(page, pipState(page, 'custom'))
    await expect(pipState(page, 'custom')).not.toHaveAttribute('data-state', 'pinned')
  })

  /** Cube B turned 30° about blue beside cube A, a camera that sees its
   *  near side face, the Rectangle tool, and that face's middle. */
  async function turnedCube(page: Page): Promise<V3> {
    await setup(page)
    await page.evaluate(() => {
      const h = window.__hew_test!
      const b = h.drawBox([4, 0, 0], [5, 1, 1], 1)
      h.rotateObject(b, 30, [0, 0, 1])
    })
    await page.evaluate(() =>
      window.__hew_test!.setCamera({ position: [6, -6, 3.5], target: [3.9, 2.25, 0.5], up: [0, 0, 1], fovDeg: 45 }),
    )
    await page.waitForTimeout(150)
    await page.keyboard.press('r')
    const c = Math.cos(Math.PI / 6)
    const sn = Math.sin(Math.PI / 6)
    return [4.5 * c - 0.5 * sn + 0.5 * sn, 4.5 * sn + 0.5 * c - 0.5 * c, 0.5]
  }

  test('D4: from the turned face across empty space to the custom pip, which is still clickable and pins the face', async ({ page }) => {
    const mid = await turnedCube(page)
    await hover(page, mid)
    await expect(pipState(page, 'custom')).toHaveAttribute('data-state', 'inferred')
    // Across the canvas in real steps, over empty space (this low view
    // infers the green plane there), within the 2 s window. The positions
    // are read first so the crossing itself is one quick run of moves.
    const space = await toPage(page, [6, 6, 0])
    const pip = await center(pipState(page, 'custom'))
    await page.mouse.move(space.x, space.y, { steps: 2 })
    await page.mouse.move(pip.x, pip.y, { steps: 3 })
    // Honest display on arrival: the filled pip is the last hover plane's,
    // the custom pip empty but clickable.
    await expect(pipState(page, 'green')).toHaveAttribute('data-state', 'inferred')
    await expect(pipState(page, 'custom')).toHaveAttribute('data-state', 'idle')
    await expect(pipState(page, 'custom')).toHaveAttribute('data-enabled', 'true')
    await expect(pipState(page, 'custom')).toHaveCSS('cursor', 'pointer')
    await page.waitForTimeout(2300) // dwelling on the pip: no timeout off the canvas
    await expect(pipState(page, 'custom')).toHaveAttribute('data-enabled', 'true')
    await page.mouse.down()
    await page.mouse.up()
    await expect(pipState(page, 'custom')).toHaveAttribute('data-state', 'pinned')
    const state = await page.evaluate(() => window.__hew_test!.getDrawPlaneState())
    expect(state?.hold).toBe('pinned')
    expect(Math.abs(state!.normal[0])).toBeCloseTo(Math.sin(Math.PI / 6), 3)
    expect(Math.abs(state!.normal[1])).toBeCloseTo(Math.cos(Math.PI / 6), 3)
    // A second click releases; the candidate is spent with it (D5).
    await page.mouse.down()
    await page.mouse.up()
    await expect(pipState(page, 'custom')).not.toHaveAttribute('data-state', 'pinned')
    await expect(pipState(page, 'custom')).toHaveAttribute('data-enabled', 'false')
  })

  test('D5: lingering over empty space for more than 2 s before the pip leaves it disabled', async ({ page }) => {
    const mid = await turnedCube(page)
    await hover(page, mid)
    await hover(page, [6, 6, 0])
    expect(await page.evaluate(() => window.__hew_test!.pipEnabled('custom'))).toBe(true)
    // The window closes with the pointer resting on the canvas.
    await expect.poll(() => page.evaluate(() => window.__hew_test!.pipEnabled('custom')), { timeout: 3500 }).toBe(false)
    const pip = await center(pipState(page, 'custom'))
    await page.mouse.move(pip.x, pip.y, { steps: 3 })
    await expect(pipState(page, 'custom')).toHaveAttribute('data-enabled', 'false')
    await expect(pipState(page, 'custom')).toHaveCSS('cursor', 'not-allowed')
    await page.mouse.down()
    await page.mouse.up()
    await expect(pipState(page, 'custom')).not.toHaveAttribute('data-state', 'pinned')
  })
})

test.describe('measurement box next to the die', () => {
  test('top-right docks under the pips without touching them', async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('hew.settings.measurementsPosition', 'top-right'))
    await setup(page)
    await lookFrom(page, [0, -1, 0.15])
    await page.keyboard.press('l')
    const c = (await page.locator('canvas').first().boundingBox())!
    await page.mouse.click(c.x + c.width / 2, c.y + c.height / 2)
    await page.mouse.move(c.x + c.width / 2 + 80, c.y + c.height / 2 - 20, { steps: 5 })
    const box = page.getByTestId('measurement-box')
    await expect(box).toBeVisible()
    const pips = (await page.getByTestId('view-die-pips').boundingBox())!
    const b = (await box.boundingBox())!
    expect(b.y).toBeGreaterThanOrEqual(pips.y + pips.height + 8)
    expect(b.x + b.width).toBeCloseTo(c.x + c.width - 16, 0)
  })

  test('defaults to the bottom right, clear of the dock', async ({ page }) => {
    await setup(page)
    await lookFrom(page, [0, -1, 0.15])
    await page.keyboard.press('l')
    const c = (await page.locator('canvas').first().boundingBox())!
    await page.mouse.click(c.x + c.width / 2, c.y + c.height / 2)
    await page.mouse.move(c.x + c.width / 2 + 80, c.y + c.height / 2 - 20, { steps: 5 })
    const box = page.getByTestId('measurement-box')
    await expect(box).toBeVisible()
    const b = (await box.boundingBox())!
    expect(b.y + b.height).toBeGreaterThan(c.y + c.height / 2)
  })
})

test.describe('neighbors at 1024 px', () => {
  test('the isolation bar and the die do not overlap', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 700 })
    await setup(page)
    const id = await page.evaluate(() => window.__hew_test!.getObjectIds()[0]!)
    await page.evaluate((i) => window.__hew_test!.isolate([{ kind: 'object', id: i }]), id)
    const bar = page.locator('[data-isolation-bar]')
    await expect(bar).toBeVisible()
    const a = (await bar.boundingBox())!
    const d = (await page.getByTestId('view-die').boundingBox())!
    const overlap = a.x < d.x + d.width && d.x < a.x + a.width && a.y < d.y + d.height && d.y < a.y + a.height
    expect(overlap).toBe(false)
  })
})
