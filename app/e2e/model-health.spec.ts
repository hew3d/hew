import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * Model Health, Stage 1 ("see it") — the real UI, real wasm kernel.
 *
 * An open STL goes in through File ▸ Import…; the report says it in plain
 * neutral words (no "leaky"), with OK and Make Solid… beside it; the Model
 * Health window (Object ▸ Model Health…) lists the
 * object, clicking the row selects it, and the status badge and Object Info
 * follow. The panel's per-object repairs and batches run here too.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const fixture = (name: string): number[] =>
  Array.from(readFileSync(resolve(process.cwd(), `e2e/fixtures/stl/${name}`)))

/** Same arming as stl-import.spec.ts: a stub File System Access picker. */
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

const healthWindow = (page: Page) => page.getByRole('dialog', { name: 'Model Health' })

/** Object ▸ Model Health…: opens the window (also the way to show it after an import). */
async function openHealthPanel(page: Page): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object' }).click()
  await page.getByTestId('menu-bar').getByText('Model Health…', { exact: true }).click()
  await expect(healthWindow(page)).toBeVisible()
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('an open STL import: report copy, the panel, the row, selection, badge, Object Info', async ({ page }) => {
  await importOpenCube(page)

  // The report: plain, neutral words, no "leaky"; OK is the primary action with Make Solid… beside it.
  const report = page.getByRole('dialog', { name: /import report/i })
  await expect(report).toBeVisible()
  await expect(report).toContainText('Imported 1 object.')
  await expect(report).toContainText('It works as it is.')
  await expect(report).not.toContainText(/leaky|manifold|rim\b|fix/i)
  await expect(report.getByRole('button', { name: 'OK' })).toBeFocused()
  await expect(report.getByRole('button', { name: 'Make Solid…' })).toBeVisible()
  await report.getByRole('button', { name: 'OK' }).click()
  await expect(report).toBeHidden()
  // The wizard never opens on its own.
  await expect(page.getByRole('dialog', { name: 'Make Solid' })).toHaveCount(0)

  // Model Health starts closed; Object ▸ Model Health… opens it (whole model).
  await expect(healthWindow(page)).toHaveCount(0)
  await openHealthPanel(page)
  await expect(page.getByLabel('Scope')).toHaveValue('document')
  await expect(page.getByRole('button', { name: /Has openings \(1\)/ })).toBeVisible()
  const row = page.locator('[data-health-row]')
  await expect(row).toHaveCount(1)
  await expect(row).toContainText('1 opening')

  // Clicking the row selects the object.
  expect(await page.evaluate(() => window.__hew_test!.getSelection().length)).toBe(0)
  await row.click()
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.kind))).toEqual(['object'])
  await expect(row).toHaveAttribute('aria-pressed', 'true')

  // Object Info says what is wrong, in plain words, with a Show in list link.
  await expect(page.getByText('Open: 1 opening')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Show in list' })).toBeVisible()

  // The status badge counts what isn't solid, in neutral words.
  const badge = page.locator('[data-health-badge="open"]')
  await expect(badge).toHaveText('1 object · 1 not solid')

  // Closing the window, then clicking the badge, brings the list back
  // (the badge never opens the wizard).
  await healthWindow(page).getByTitle('Close', { exact: true }).click()
  await expect(healthWindow(page)).toHaveCount(0)
  await badge.click()
  await expect(healthWindow(page)).toBeVisible()
  await expect(page.getByRole('dialog', { name: 'Make Solid' })).toHaveCount(0)
})

test('Object ▸ Model Health… opens the window; an all-solid document says so', async ({ page }) => {
  await expect(healthWindow(page)).toHaveCount(0)
  await openHealthPanel(page)
  await expect(page.getByLabel('Scope')).toHaveValue('document')
  await expect(page.getByText('Every object in the model is solid.')).toBeVisible()
})

test('Object Info\'s Show link opens Model Health on that object', async ({ page }) => {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  // Select the imported object from the Outliner row, then Show.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const ids = h.getObjectIds()
    h.selectObjects(ids)
  })
  await page.getByRole('button', { name: 'Show in list' }).click()
  await expect(healthWindow(page)).toBeVisible()
  await expect(page.getByRole('button', { name: 'Show all objects' })).toBeVisible()
  await expect(page.locator('[data-health-row]')).toHaveCount(1)
})

test('the Model Health window: closed at launch, opens from the Object menu, drags by its title bar, remembers where it was left', async ({ page }) => {
  await expect(healthWindow(page)).toHaveCount(0)
  await openHealthPanel(page)
  const win = healthWindow(page)
  const box0 = (await win.boundingBox())!
  expect(box0.width).toBeGreaterThanOrEqual(280)
  // The window itself takes focus (not its scope list or search box), so a
  // tool key typed next still picks the tool.
  await expect(win).toBeFocused()
  await page.keyboard.press('r')
  await expect(page.locator('[aria-label="Rectangle"][aria-checked="true"]')).toBeVisible()
  await page.keyboard.press('Escape')

  // Drag the title bar with the real mouse.
  const bar = win.locator('[data-floating-window-title]')
  const b = (await bar.boundingBox())!
  const sx = b.x + 40
  const sy = b.y + b.height / 2
  await page.mouse.move(sx, sy)
  await page.mouse.down()
  await page.mouse.move(sx - 120, sy - 30, { steps: 6 })
  await page.mouse.up()
  const box1 = (await win.boundingBox())!
  expect(Math.round(box1.x - box0.x)).toBe(-120)
  expect(Math.round(box1.y - box0.y)).toBe(-30)

  // The ✕ closes it; reopening puts it where it was left.
  await win.getByTitle('Close', { exact: true }).click()
  await expect(healthWindow(page)).toHaveCount(0)
  await openHealthPanel(page)
  const box2 = (await healthWindow(page).boundingBox())!
  expect(Math.round(box2.x)).toBe(Math.round(box1.x))
  expect(Math.round(box2.y)).toBe(Math.round(box1.y))
  expect(Math.round(box2.width)).toBe(Math.round(box1.width))
  expect(Math.round(box2.height)).toBe(Math.round(box1.height))

  // Leave the shared browser state tidy for the next test.
  await page.evaluate(() => localStorage.removeItem('hew.ui.modelHealthWindow'))
})

/** Import the open cube and open its row in Model Health, selected. */
async function reviewOpenCube(page: Page) {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  await openHealthPanel(page)
  const row = page.locator('[data-health-row]')
  await expect(row).toHaveCount(1)
  await row.click()
  await expect(row).toHaveAttribute('aria-pressed', 'true')
  return row
}

test('Stage 2: Close makes the open cube a solid; Edit ▸ Undo and Redo bring it back and away', async ({ page }) => {
  const row = await reviewOpenCube(page)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')

  await row.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
  // Still selected in the viewport, now a solid.
  expect(await page.evaluate(() => window.__hew_test!.getSelection().length)).toBe(1)
  await expect(page.getByText('Solid', { exact: true })).toBeVisible()

  // Edit ▸ Undo restores the open row and the badge.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText(/^Undo\b/).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(1)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')

  // Edit ▸ Redo closes it again.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText(/^Redo\b/).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
})

test('Stage 2: Close from Object Info closes the selected open object', async ({ page }) => {
  await reviewOpenCube(page)
  await page.getByRole('button', { name: 'Close', exact: true }).last().click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
})

test('Stage 2: Keep as is hides the row; Show kept panels and sheets brings it back with Don\'t keep', async ({ page }) => {
  const row = await reviewOpenCube(page)
  await row.getByRole('button', { name: 'Keep as is' }).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid') // kept ones still count

  await page.getByRole('checkbox', { name: 'Show kept panels and sheets' }).check()
  const back = page.locator('[data-health-row]')
  await expect(back).toHaveCount(1)
  // Click the name: the selected row's center can land on an action button.
  await back.getByText('cube_open', { exact: true }).click()
  await back.getByRole('button', { name: "Don't keep" }).click()
  await page.getByRole('checkbox', { name: 'Show kept panels and sheets' }).uncheck()
  await expect(page.locator('[data-health-row]')).toHaveCount(1)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')
})

// ---------------------------------------------------------------------------
// Stage 3 — fix many
// ---------------------------------------------------------------------------

/** The File ▸ Import… stub for a `.skp` fixture (same arming as the STL one). */
async function importFaceWithHole(page: Page): Promise<void> {
  const bytes = Array.from(readFileSync(resolve(process.cwd(), 'e2e/fixtures/skp/face-with-hole.skp')))
  await armFilePicker(page, 'face-with-hole.skp', bytes)
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
}

const objectBounds = (page: Page) =>
  page.evaluate(() => {
    const h = window.__hew_test!
    return h.getObjectIds().map((id) => h.getObjectBounds(id))
  })

test('Stage 3: Add thickness… (centered) turns an imported flat surface with a hole into a solid; Cmd+Z restores it', async ({ page }) => {
  await importFaceWithHole(page)
  const report = page.getByRole('dialog', { name: /import report/i })
  await expect(report).toContainText('It works as it is.')
  await expect(report.getByRole('button', { name: 'OK' })).toBeVisible()
  await report.getByRole('button', { name: 'OK' }).click()
  await openHealthPanel(page)

  const flat = page.locator('[data-health-group="flatPanels"]')
  await expect(flat.getByRole('button', { name: /Flat panels \(1\)/ })).toBeVisible()
  await expect(page.locator('[data-health-row]')).toContainText('Flat panel')
  expect((await objectBounds(page))[0][5]).toBeCloseTo(0, 6) // a sheet: no height

  await flat.getByRole('button', { name: 'Add thickness…' }).click()
  const dialog = page.getByRole('dialog', { name: 'Add thickness' })
  await expect(dialog).toBeVisible()
  await dialog.getByRole('textbox', { name: 'Thickness' }).fill('2cm')
  await dialog.getByRole('radio', { name: 'Centered' }).check()
  await dialog.getByRole('button', { name: 'Add thickness' }).click()

  await expect(dialog).toBeHidden()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
  const solid = (await objectBounds(page))[0]
  expect(solid[5] - solid[2]).toBeCloseTo(0.02, 5) // 2 cm, centered on the sheet
  expect(solid[2]).toBeCloseTo(-0.01, 5)

  await page.keyboard.press('ControlOrMeta+z')
  await expect(page.locator('[data-health-row]')).toHaveCount(1)
  expect((await objectBounds(page))[0][5]).toBeCloseTo(0, 6)
})

test('Stage 3: Push/Pull dragged on a flat surface pulls it into a solid', async ({ page }) => {
  await importFaceWithHole(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  await openHealthPanel(page)
  await expect(page.locator('[data-health-row]')).toHaveCount(1)

  // The sheet is the unit square on the ground plane; view it obliquely so a
  // drag along its normal moves on screen, then drag with real mouse events.
  await page.evaluate(() => window.__hew_test!.setCamera({ position: [3, -3, 3], target: [0.5, 0.5, 0], fovDeg: 45 }))
  await page.waitForTimeout(250)
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const toPage = async (world: [number, number, number]) => {
    const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), world)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  await page.getByRole('radio', { name: 'Push/Pull' }).click()
  const start = await toPage([0.1, 0.1, 0])
  await page.mouse.move(start.x - 6, start.y - 4)
  await page.mouse.move(start.x, start.y)
  await page.waitForTimeout(200)
  await page.mouse.down()
  await page.mouse.up()
  const up = await toPage([0.1, 0.1, 0.4])
  await page.mouse.move(up.x, up.y, { steps: 10 })
  await page.mouse.down()
  await page.mouse.up()

  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
  const solid = (await objectBounds(page))[0]
  expect(Math.abs(solid[5] - solid[2])).toBeGreaterThan(0.05)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
})

test('Stage 3: Close all closes two open objects in one undo entry', async ({ page }) => {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  // File ▸ Import… on a dirty web document asks to discard (the web build
  // cannot add a window), so the second cube comes in through the harness's
  // import path — the same kernel import the menu runs.
  await page.evaluate((bytes) => {
    window.__hew_test!.importStl(bytes, 0.002) // a bigger cube: its rim is its own, not a seam
  }, fixture('cube_open.stl'))

  // Open Model Health on the whole document: both cubes are listed.
  await openHealthPanel(page)
  await expect(page.getByLabel('Scope')).toHaveValue('document')
  await expect(page.getByRole('button', { name: /Has openings \(2\)/ })).toBeVisible()
  await expect(page.locator('[data-health-row]')).toHaveCount(2)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('2 objects · 2 not solid')

  await page.locator('[data-health-group="openings"]').getByRole('button', { name: 'Close all' }).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('2 objects ✓ solid')).toBeVisible()

  // ONE Edit ▸ Undo brings both back.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText(/^Undo\b/).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(2)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('2 objects · 2 not solid')

  // …and one Redo closes both again.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText(/^Redo\b/).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
})

test('Stage 3: a split shape\'s Rejoin closes its pieces in one undo entry', async ({ page }) => {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  // The same cube imported again sits exactly on the first: the two share
  // their open rim, which the diagnosis reads as one model split at import.
  await page.evaluate((bytes) => {
    window.__hew_test!.importStl(bytes, 0.001)
  }, fixture('cube_open.stl'))
  await openHealthPanel(page)
  const family = page.locator('[data-health-group^="family-"]')
  await expect(family).toHaveCount(1)
  await expect(page.locator('[data-health-row]')).toHaveCount(2)

  await family.getByRole('button', { name: 'Rejoin', exact: true }).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('2 objects ✓ solid')).toBeVisible()

  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText(/^Undo\b/).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(2)
})
