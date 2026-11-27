import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * View-only isolation ("Showing N objects · Show everything") — the real UI,
 * real wasm kernel. Model Health's row click isolates its object: everything
 * else stops drawing and cannot be picked; Show everything and Escape bring
 * it back. A component member shown through one instance hides the other
 * placement and fades the instance's other members. None of it is a document
 * change.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type Vec3 = [number, number, number]

const fixture = (name: string): number[] =>
  Array.from(readFileSync(resolve(process.cwd(), `e2e/fixtures/stl/${name}`)))

async function armFilePicker(page: Page, name: string, bytes: number[]): Promise<void> {
  await page.addInitScript(
    ([n, b]) => {
      const buf = new Uint8Array(b as number[])
      // @ts-expect-error test override of the File System Access API
      window.showOpenFilePicker = async () => [
        { getFile: async () => ({ name: n as string, arrayBuffer: async () => buf.buffer }) },
      ]
    },
    [name, bytes] as [string, number[]],
  )
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

async function importOpenCube(page: Page): Promise<void> {
  await armFilePicker(page, 'cube_open.stl', fixture('cube_open.stl'))
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
  await page.getByRole('dialog', { name: /stl import units/i }).getByRole('button', { name: /^import$/i }).click()
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

type Rgb = { r: number; g: number; b: number } | null
const dist = (a: Rgb, b: Rgb): number =>
  a === null || b === null ? 999 : Math.abs(a.r - b.r) + Math.abs(a.g - b.g) + Math.abs(a.b - b.b)

/** A camera that looks at `center` from the front and above, `d` meters away. */
async function lookAt(page: Page, center: Vec3, d: number): Promise<void> {
  await page.evaluate(
    ([c, dist]) => {
      const cc = c as Vec3
      window.__hew_test!.setCamera({
        position: [cc[0], cc[1] - (dist as number), cc[2] + (dist as number) * 0.7],
        target: cc,
      })
    },
    [center, d] as [Vec3, number],
  )
  await page.waitForTimeout(100)
}

const pixel = (page: Page, p: Vec3) => page.evaluate((w) => window.__hew_test!.pixelColorAt(w), p)

/** Click the canvas where world point `p` projects. */
async function clickWorld(page: Page, p: Vec3): Promise<void> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const s = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), p)
  await page.mouse.click(box.x + s.x, box.y + s.y)
  await page.waitForTimeout(100)
}

/** Object ▸ Model Health…, then shrink the window and park it in the bottom-left corner
 *  with the real mouse, so it does not sit over the model the test clicks on. */
async function openHealthWindowParked(page: Page): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object' }).click()
  await page.getByTestId('menu-bar').getByText('Model Health…', { exact: true }).click()
  const win = page.getByRole('dialog', { name: 'Model Health' })
  await expect(win).toBeVisible()
  const grip = (await win.locator('[data-floating-window-grip]').boundingBox())!
  await page.mouse.move(grip.x + 8, grip.y + 8)
  await page.mouse.down()
  await page.mouse.move(grip.x - 400, grip.y - 600, { steps: 5 })
  await page.mouse.up()
  const bar = (await win.locator('[data-floating-window-title]').boundingBox())!
  await page.mouse.move(bar.x + 30, bar.y + 10)
  await page.mouse.down()
  await page.mouse.move(bar.x - 3000, bar.y + 3000, { steps: 5 })
  await page.mouse.up()
}

const selection = (page: Page) => page.evaluate(() => window.__hew_test!.getSelection().map((n) => `${n.kind}:${n.id}`))
const bar = (page: Page) => page.locator('[data-isolation-bar]')

test('a Model Health row isolates its object; the rest is not drawn or pickable until Show everything', async ({ page }) => {
  // Two solids and an imported open cube.
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  await openHealthWindowParked(page)
  const cube = await page.evaluate(() => window.__hew_test!.getObjectIds()[0])
  const cb = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), cube)
  const size = Math.max(cb[3] - cb[0], cb[4] - cb[1], cb[5] - cb[2])
  const cy = (cb[1] + cb[4]) / 2
  const a = await page.evaluate(
    ([x, y, s]) => window.__hew_test!.drawBox([x, y, 0], [x + s, y + s, 0], s),
    [cb[3] + size, cy, size] as [number, number, number],
  )
  const b = await page.evaluate(
    ([x, y, s]) => window.__hew_test!.drawBox([x, y, 0], [x + s, y + s, 0], s),
    [cb[0] - 2 * size, cy, size] as [number, number, number],
  )
  const aTop: Vec3 = [cb[3] + 1.5 * size, cy + size / 2, size]
  const bTop: Vec3 = [cb[0] - 1.5 * size, cy + size / 2, size]
  const wideCenter: Vec3 = [(cb[0] + cb[3]) / 2, cy, size / 2]
  const wide = size * 7

  await lookAt(page, wideCenter, wide)
  const aBefore = await pixel(page, aTop)
  const empty = await pixel(page, [wideCenter[0], cy + 6 * size, 0])
  expect(dist(aBefore, empty)).toBeGreaterThan(20) // the box is drawn

  // Control: before isolation a click on A selects it.
  await clickWorld(page, aTop)
  expect(await selection(page)).toEqual([`object:${a}`])

  // Model Health is open on the import; its one row isolates the cube.
  const row = page.locator('[data-health-row]')
  await expect(row).toHaveCount(1)
  const rev = await page.evaluate(() => window.__hew_test!.getModelStateHash())
  await row.click({ position: { x: 6, y: 6 } }) // the selected row grows action buttons
  await expect(bar(page)).toContainText('Showing 1 object')
  await expect(page.getByRole('button', { name: 'Show everything' })).toBeVisible()
  const iso = await page.evaluate(() => window.__hew_test!.getIsolation())
  expect(iso?.hiddenObjects.sort()).toEqual([a, b].sort())
  // Isolation is no document change: the document is byte-for-byte the same.
  expect(await page.evaluate(() => window.__hew_test!.getModelStateHash())).toBe(rev)

  // The others are not drawn: their pixels now read as background.
  await lookAt(page, wideCenter, wide)
  expect(dist(await pixel(page, aTop), empty)).toBeLessThan(12)
  expect(dist(await pixel(page, bTop), empty)).toBeLessThan(12)
  // …and a click where A was selects nothing (the cube's row selection clears).
  expect(await selection(page)).toEqual([`object:${cube}`])
  await clickWorld(page, aTop)
  expect(await selection(page)).toEqual([])

  // Show everything restores them.
  await page.getByRole('button', { name: 'Show everything' }).click()
  await expect(bar(page)).toHaveCount(0)
  expect(await page.evaluate(() => window.__hew_test!.getIsolation())).toBeNull()
  await lookAt(page, wideCenter, wide)
  expect(dist(await pixel(page, aTop), aBefore)).toBeLessThan(12)
  await clickWorld(page, aTop)
  expect(await selection(page)).toEqual([`object:${a}`])

  // Escape ends it too, and the Outliner's hidden state was never touched.
  await row.click({ position: { x: 6, y: 6 } }) // the selected row grows action buttons
  await expect(bar(page)).toContainText('Showing 1 object')
  await page.keyboard.press('Escape')
  await expect(bar(page)).toHaveCount(0)
  expect(await page.evaluate((id) => window.__hew_test!.isNodeHidden({ kind: 'object', id }), a)).toBe(false)

  // A document change while isolated keeps the targets that still exist: undo
  // removes box B (the last thing drawn), the isolation stays on the cube.
  await row.click({ position: { x: 6, y: 6 } }) // the selected row grows action buttons
  await expect(bar(page)).toContainText('Showing 1 object')
  await page.evaluate(() => window.__hew_test!.undo())
  await expect(bar(page)).toContainText('Showing 1 object')
  await expect
    .poll(async () => (await page.evaluate(() => window.__hew_test!.getIsolation()))?.hiddenObjects)
    .toEqual([a])
  // Deleting the isolated object itself leaves no target: isolation ends.
  await page.evaluate((id) => window.__hew_test!.deleteObject(id), cube)
  await expect(bar(page)).toHaveCount(0)
  expect(await page.evaluate(() => window.__hew_test!.getIsolation())).toBeNull()
})

test('closing the Model Health window ends isolation', async ({ page }) => {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object' }).click()
  await page.getByTestId('menu-bar').getByText('Model Health…', { exact: true }).click()
  await page.evaluate(() => window.__hew_test!.drawBox([5, 5, 0], [6, 6, 0], 1))
  await page.locator('[data-health-row]').click()
  await expect(bar(page)).toContainText('Showing 1 object')
  await page.getByRole('dialog', { name: 'Model Health' }).getByTitle('Close', { exact: true }).click()
  await expect(bar(page)).toHaveCount(0)
  expect(await page.evaluate(() => window.__hew_test!.getIsolation())).toBeNull()
})

test('a component member through one instance: the other instance is hidden, the rest of this one fades', async ({ page }) => {
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    const left = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const right = h.drawBox([1.5, 0, 0], [2.5, 1, 0], 1)
    const made = h.makeComponent([left, right])
    const second = h.placeInstance(made.component, 0, 4, 0)
    return { first: made.instance, second, members: h.getComponentMemberObjects(made.component) }
  })
  expect(ids.members).toHaveLength(2)
  // Which member sits at the left: the one whose bounds start at x = 0 in the first instance.
  const m0 = ids.members[0]
  const tops = {
    firstLeft: [0.5, 0.5, 1] as Vec3,
    firstRight: [2, 0.5, 1] as Vec3,
    second: [0.5, 4.5, 1] as Vec3,
  }
  await lookAt(page, [1.25, 2, 0.5], 9)
  const full = await pixel(page, tops.firstRight)
  const empty = await pixel(page, [1.25, 9, 0])
  expect(dist(full, empty)).toBeGreaterThan(20)

  const shown = await page.evaluate(
    ([m, i]) => window.__hew_test!.isolate([{ kind: 'member', object: m, instance: i }]),
    [m0, ids.first] as [string, string],
  )
  expect(shown).toBe(1)
  await expect(bar(page)).toContainText('Showing 1 object')
  const iso = await page.evaluate(() => window.__hew_test!.getIsolation())
  expect(iso?.hiddenInstances).toContain(ids.second)
  expect(iso?.hiddenInstances).not.toContain(ids.first)

  await lookAt(page, [1.25, 2, 0.5], 9)
  // The other placement is gone; of the isolated instance one member is full, the other faded.
  expect(dist(await pixel(page, tops.second), empty)).toBeLessThan(12)
  const l = await pixel(page, tops.firstLeft)
  const r = await pixel(page, tops.firstRight)
  const [isolatedPx, fadedPx] = dist(l, empty) > dist(r, empty) ? [l, r] : [r, l]
  expect(dist(isolatedPx, fadedPx)).toBeGreaterThan(15)
  expect(dist(fadedPx, empty)).toBeGreaterThan(5) // still drawn, just lighter

  // Clicking the other placement selects nothing.
  await clickWorld(page, tops.second)
  expect(await selection(page)).toEqual([])

  await page.getByRole('button', { name: 'Show everything' }).click()
  await expect(bar(page)).toHaveCount(0)
  await lookAt(page, [1.25, 2, 0.5], 9)
  expect(dist(await pixel(page, tops.second), empty)).toBeGreaterThan(20)
  expect(dist(await pixel(page, tops.firstRight), full)).toBeLessThan(12)
  expect(dist(await pixel(page, tops.firstLeft), full)).toBeLessThan(25)
})

test('drawing while isolated: the new box stays visible and pickable (real Rectangle + Push/Pull)', async ({ page }) => {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  const cube = await page.evaluate(() => window.__hew_test!.getObjectIds()[0])
  const cb = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), cube)
  const size = Math.max(cb[3] - cb[0], cb[4] - cb[1], cb[5] - cb[2])

  // Isolate the cube: it is the only thing shown.
  const shown = await page.evaluate((id) => window.__hew_test!.isolate([{ kind: 'object', id }]), cube)
  expect(shown).toBe(1)
  await expect(bar(page)).toContainText('Showing 1 object')

  // Draw a square on the ground beside the cube with the real Rectangle tool.
  const s = Math.max(size, 0.05)
  const x0 = cb[3] + s
  const y0 = (cb[1] + cb[4]) / 2 - s / 2
  const center: Vec3 = [x0 + s / 2, y0 + s / 2, 0]
  await lookAt(page, [(cb[0] + x0 + s) / 2, y0 + s / 2, 0], s * 8)
  const canvas = (await page.locator('canvas').first().boundingBox())!
  const toPage = async (w: Vec3) => {
    const p = await page.evaluate((v) => window.__hew_test!.worldToScreen(v), w)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  const click = async (p: { x: number; y: number }) => {
    await page.mouse.move(p.x, p.y)
    await page.mouse.down()
    await page.mouse.up()
  }
  await page.getByRole('radio', { name: 'Rectangle' }).click()
  await click(await toPage([x0, y0, 0]))
  await click(await toPage([x0 + s, y0 + s, 0]))

  // Pull it up into a box with the real Push/Pull tool.
  await page.getByRole('radio', { name: 'Push/Pull' }).click()
  await click(await toPage(center))
  await page.keyboard.type(String(s))
  await page.keyboard.press('Enter')

  const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(2)
  const box = ids.find((id) => id !== cube)!
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  // Still isolated on the cube, but the box (made while isolated) is not hidden…
  expect(await page.evaluate(() => window.__hew_test!.getIsolation()?.hiddenObjects ?? [])).not.toContain(box)
  // …it is drawn…
  const top: Vec3 = [center[0], center[1], s]
  const empty = await pixel(page, [center[0], center[1] + 5 * s, 0])
  expect(dist(await pixel(page, top), empty)).toBeGreaterThan(20)
  // …and a real click on it picks it.
  await page.getByRole('radio', { name: 'Select' }).click()
  await clickWorld(page, top)
  expect(await selection(page)).toEqual([`object:${box}`])
})
