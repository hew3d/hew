import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * The Make Solid wizard — the real UI, real wasm kernel, real file import.
 *
 * (a) An open STL → the import report's neutral text → Make Solid… → step by
 *     step → Openings defaults to Leave them open (Recommended) → Close the
 *     openings → Ready → Make Solid → Done → badge solid → Edit ▸ Undo.
 * (b) The same import, object selected → Object ▸ Make Solid… → Use
 *     recommended choices closes it (a selection's default).
 * (c) A flat panel with a hole → Object ▸ Make Solid… → step by step → Flat
 *     panels → Add thickness 3 mm, Centered → Ready → Make Solid → solid.
 * (d) Object ▸ Make Solid… opens it; Escape closes it with no change.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const fixture = (rel: string): number[] => Array.from(readFileSync(resolve(process.cwd(), `e2e/fixtures/${rel}`)))

/** The stub File System Access picker the other import specs use. */
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
  await armFilePicker(page, 'cube_open.stl', fixture('stl/cube_open.stl'))
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
  await page.getByRole('dialog', { name: /stl import units/i }).getByRole('button', { name: /^import$/i }).click()
}

async function importFaceWithHole(page: Page): Promise<void> {
  await armFilePicker(page, 'face-with-hole.skp', fixture('skp/face-with-hole.skp'))
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
}

const wizard = (page: Page) => page.getByRole('dialog', { name: 'Make Solid' })

async function chooseFromMenu(page: Page, menu: string, item: string): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: menu }).click()
  await page.getByText(item, { exact: true }).click()
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('an open STL: Make Solid… from the import report, step by step, solid, one Undo', async ({ page }) => {
  await importOpenCube(page)

  const report = page.getByRole('dialog', { name: /import report/i })
  await expect(report).toContainText('Imported 1 object.')
  await expect(report).toContainText('It works as it is.')
  await expect(report).not.toContainText(/fix|leaky/i)
  await report.getByRole('button', { name: 'Make Solid…' }).click()
  await expect(report).toBeHidden()

  // The overview: what Hew checked and what it recommends for an import.
  await expect(wizard(page)).toBeVisible()
  await expect(wizard(page).getByRole('heading', { name: 'Make this import solid' })).toBeVisible()
  await expect(wizard(page)).toContainText('Hew checked the object in this import.')
  await expect(wizard(page)).toContainText('1 shape has openings: leave them open.')
  await wizard(page).getByRole('button', { name: 'Go step by step' }).click()

  // Openings: Leave them open is selected and marked Recommended.
  await expect(wizard(page).getByRole('heading', { name: 'Openings' })).toBeVisible()
  await expect(wizard(page).getByRole('radio', { name: /Leave them open/ })).toBeChecked()
  await expect(wizard(page).getByText('(Recommended)')).toBeVisible()
  await wizard(page).getByRole('radio', { name: 'Close the openings' }).check()
  await wizard(page).getByRole('button', { name: 'Accept' }).click()

  await expect(wizard(page).getByRole('heading', { name: 'Ready' })).toBeVisible()
  await expect(wizard(page)).toContainText('Close the openings in 1 shape.')
  await wizard(page).getByRole('button', { name: 'Make Solid' }).click()

  await expect(wizard(page).getByRole('heading', { name: 'Done' })).toBeVisible()
  await expect(wizard(page)).toContainText('1 object is solid now')
  await expect(wizard(page)).toContainText('To undo all of it, choose Edit ▸ Undo.')
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()

  await wizard(page).getByRole('button', { name: 'Done' }).click()
  await expect(wizard(page)).toHaveCount(0)

  // One Undo takes the whole thing back.
  await chooseFromMenu(page, 'Edit', 'Undo Make Solid')
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')
})

test('a selected open STL: Object ▸ Make Solid… and Use recommended choices closes it', async ({ page }) => {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.selectObjects(h.getObjectIds())
  })

  await chooseFromMenu(page, 'Object', 'Make Solid…')
  await expect(wizard(page)).toBeVisible()
  await expect(wizard(page).getByRole('heading', { name: 'Make the selection solid' })).toBeVisible()
  await expect(wizard(page)).toContainText('1 shape has openings: close them.')
  await wizard(page).getByRole('button', { name: 'Use recommended choices' }).click()

  await expect(wizard(page).getByRole('heading', { name: 'Done' })).toBeVisible()
  await expect(wizard(page)).toContainText('1 object is solid now')
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
})

test('a flat panel with a hole: step by step, Add thickness 3 mm Centered, Ready, Make Solid', async ({ page }) => {
  await importFaceWithHole(page)
  const report = page.getByRole('dialog', { name: /import report/i })
  await report.getByRole('button', { name: 'OK' }).click()
  const before = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getObjectBounds(h.getObjectIds()[0])
  })
  expect(before[5]).toBeCloseTo(0, 6) // a sheet: no height

  await chooseFromMenu(page, 'Object', 'Make Solid…')
  await expect(wizard(page)).toBeVisible()
  await wizard(page).getByRole('button', { name: 'Go step by step' }).click()

  // The only page with anything on it: Flat panels (the others are skipped).
  await expect(wizard(page).getByRole('heading', { name: 'Flat panels' })).toBeVisible()
  await expect(wizard(page).getByRole('radio', { name: /Leave them as panels/ })).toBeChecked()
  await wizard(page).getByRole('radio', { name: 'Add thickness' }).check()
  const field = wizard(page).getByRole('textbox', { name: 'Thickness' })
  await expect(field).toHaveValue('3 mm')
  await field.fill('3 mm')
  await wizard(page).getByRole('radio', { name: 'Centered' }).check()
  await wizard(page).getByRole('button', { name: 'Accept' }).click()

  await expect(wizard(page).getByRole('heading', { name: 'Ready' })).toBeVisible()
  await expect(wizard(page)).toContainText('Give 1 flat panel a thickness of 3 mm.')
  // Nothing has changed yet.
  expect((await page.evaluate(() => { const h = window.__hew_test!; return h.getObjectBounds(h.getObjectIds()[0]) }))[5]).toBeCloseTo(0, 6)
  await wizard(page).getByRole('button', { name: 'Make Solid' }).click()

  await expect(wizard(page).getByRole('heading', { name: 'Done' })).toBeVisible()
  await expect(wizard(page)).toContainText('1 object is solid now')
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
  const solid = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getObjectBounds(h.getObjectIds()[0])
  })
  expect(solid[5] - solid[2]).toBeCloseTo(0.003, 5) // 3 mm, centered on the sheet
  expect(solid[2]).toBeCloseTo(-0.0015, 5)
})

test('Object ▸ Make Solid… opens the wizard; Escape closes it with no change', async ({ page }) => {
  await importOpenCube(page)
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  const hash = await page.evaluate(() => window.__hew_test!.getModelStateHash())

  await chooseFromMenu(page, 'Object', 'Make Solid…')
  await expect(wizard(page)).toBeVisible()
  await expect(wizard(page).getByRole('heading', { name: 'Make this model solid' })).toBeVisible()
  // Non-modal: the viewport still takes real input behind it (the badge is still there).
  await expect(wizard(page).getByRole('button', { name: 'Use recommended choices' })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(wizard(page)).toHaveCount(0)
  expect(await page.evaluate(() => window.__hew_test!.getModelStateHash())).toBe(hash)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')
  // And the close button does the same.
  await chooseFromMenu(page, 'Object', 'Make Solid…')
  await wizard(page).getByRole('button', { name: 'Close', exact: true }).first().click()
  await expect(wizard(page)).toHaveCount(0)
})
