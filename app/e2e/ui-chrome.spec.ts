import { test, expect } from '@playwright/test'

/**
 * UI-chrome behavior spec — the DOM half of the app the semantic-harness
 * specs deliberately bypass (tools.spec.ts drives the kernel directly; this
 * file proves the chrome that *reaches* those kernel ops is wired).
 *
 * Covers the 2026-07 testing-fix surfaces that until now had only Vitest
 * component coverage:
 *   - Tool rail radio activation + bare-letter shortcuts.
 *   - Contextual dock: empty-context verbs incl. Arc, the honest
 *     active-tool highlight (aria-pressed), verb dispatch, and the
 *     selection-driven context swap ( +).
 *   - The unified File ▸ Export… dialog : single menu entry,
 *     format select, Cancel/Escape. The final Export click is NOT driven
 *     here — it enters the platform file-save layer (FSAA/anchor download),
 *     which is host-specific; the dialog's callback contract is unit-tested
 *     in dialogs.test.tsx.
 *   - Command palette: Ctrl+K opens, fuzzy-run activates a tool.
 *
 * Assertions go through ARIA state (role=radio aria-checked, aria-pressed)
 * — the same contract a screen reader gets, so the tests don't couple to
 * styling.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
})

// ---------------------------------------------------------------------------
// Tool rail
// ---------------------------------------------------------------------------

test('tool rail: Select is the default active tool', async ({ page }) => {
  const rail = page.getByRole('radiogroup', { name: 'Tools' })
  await expect(rail.getByRole('radio', { name: 'Select' })).toHaveAttribute('aria-checked', 'true')
})

test('tool rail: clicking a row activates that tool (radio semantics)', async ({ page }) => {
  const rail = page.getByRole('radiogroup', { name: 'Tools' })
  await rail.getByRole('radio', { name: 'Arc' }).click()
  await expect(rail.getByRole('radio', { name: 'Arc' })).toHaveAttribute('aria-checked', 'true')
  // Radio behavior: exactly one active — the previous tool went inactive.
  await expect(rail.getByRole('radio', { name: 'Select' })).toHaveAttribute('aria-checked', 'false')

  await rail.getByRole('radio', { name: 'Push/Pull' }).click()
  await expect(rail.getByRole('radio', { name: 'Push/Pull' })).toHaveAttribute('aria-checked', 'true')
  await expect(rail.getByRole('radio', { name: 'Arc' })).toHaveAttribute('aria-checked', 'false')
})

test('keyboard: bare-letter tool shortcuts switch tools; Space returns to Select', async ({
  page,
}) => {
  const rail = page.getByRole('radiogroup', { name: 'Tools' })
  // The canvas (not an input) must have focus for bare letters to fire.
  await page.locator('canvas').first().click({ position: { x: 10, y: 10 } })

  await page.keyboard.press('r')
  await expect(rail.getByRole('radio', { name: 'Rectangle' })).toHaveAttribute('aria-checked', 'true')

  await page.keyboard.press('a')
  await expect(rail.getByRole('radio', { name: 'Arc' })).toHaveAttribute('aria-checked', 'true')

  await page.keyboard.press('c')
  await expect(rail.getByRole('radio', { name: 'Circle' })).toHaveAttribute('aria-checked', 'true')

  await page.keyboard.press(' ')
  await expect(rail.getByRole('radio', { name: 'Select' })).toHaveAttribute('aria-checked', 'true')
})

// ---------------------------------------------------------------------------
// Contextual dock (es)
// ---------------------------------------------------------------------------

/** The dock container (`.hew-dock`) — scoping avoids matching rail rows with
 * the same accessible names. */
function dock(page: import('@playwright/test').Page) {
  return page.locator('.hew-dock')
}

test('dock: empty context shows the DRAW verb set including Arc', async ({ page }) => {
  await expect(dock(page)).toBeVisible()
  await expect(dock(page)).toContainText('DRAW')
  for (const verb of ['Rectangle', 'Line', 'Circle', 'Arc']) {
    await expect(dock(page).getByRole('button', { name: verb })).toBeVisible()
  }
})

test('dock: only the ACTUAL active tool is highlighted — never the first verb by default', async ({
  page,
}) => {
  // Fresh app: active tool is Select, which has no dock verb → nothing pressed.
  for (const verb of ['Rectangle', 'Line', 'Circle', 'Arc']) {
    await expect(dock(page).getByRole('button', { name: verb })).toHaveAttribute(
      'aria-pressed',
      'false',
    )
  }

  // Activate Arc via its rail row: Arc lights up; Rectangle (the first verb —
  // the old phantom-"primary" bug) stays unpressed.
  await page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Arc' }).click()
  await expect(dock(page).getByRole('button', { name: 'Arc' })).toHaveAttribute('aria-pressed', 'true')
  await expect(dock(page).getByRole('button', { name: 'Rectangle' })).toHaveAttribute(
    'aria-pressed',
    'false',
  )
})

test('dock: clicking a verb dispatches the tool action (same path as menus)', async ({ page }) => {
  await dock(page).getByRole('button', { name: 'Circle' }).click()
  await expect(
    page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Circle' }),
  ).toHaveAttribute('aria-checked', 'true')
  await expect(dock(page).getByRole('button', { name: 'Circle' })).toHaveAttribute(
    'aria-pressed',
    'true',
  )
})

test('dock: selecting an object swaps the context chip and verb set', async ({ page }) => {
  // Build + select an object through the harness (object creation itself is
  // tools.spec.ts territory; the dock's reaction to selection is the subject).
  const boxId = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), boxId)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  await expect(dock(page)).toContainText('OBJECT')
  for (const verb of ['Push/Pull', 'Move', 'Paint', 'Erase']) {
    await expect(dock(page).getByRole('button', { name: verb })).toBeVisible()
  }
  // The DRAW set is gone.
  await expect(dock(page).getByRole('button', { name: 'Arc' })).toHaveCount(0)

  // Clearing the selection returns the DRAW context.
  await page.evaluate(() => window.__hew_test!.selectObjects([]))
  await expect(dock(page)).toContainText('DRAW')
})

// ---------------------------------------------------------------------------
// Unified Export dialog
// ---------------------------------------------------------------------------

test('export: File ▸ Export… opens ONE dialog with a glTF/STL/3MF format select', async ({ page }) => {
  const menuBar = page.getByTestId('menu-bar')
  await menuBar.getByRole('button', { name: 'File' }).click()

  // Exactly one export entry — the old separate "Export STL…" item is gone.
  await expect(page.getByText('Export STL…')).toHaveCount(0)
  await page.getByText('Export…', { exact: true }).click()

  const dialog = page.getByRole('dialog', { name: 'Export' })
  await expect(dialog).toBeVisible()

  // Every format is offered in the select; glTF is the default.
  const select = dialog.locator('#export-format-select')
  await expect(select).toHaveValue('glb')
  const options = select.locator('option')
  await expect(options).toHaveCount(3)
  await expect(options.nth(0)).toContainText('glTF')
  await expect(options.nth(1)).toContainText('STL')
  await expect(options.nth(2)).toContainText('3MF')

  // Format is switchable.
  await select.selectOption('stl')
  await expect(select).toHaveValue('stl')
  await select.selectOption('3mf')
  await expect(select).toHaveValue('3mf')

  // Cancel closes without exporting.
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).not.toBeVisible()
})

test('export: Escape closes the dialog', async ({ page }) => {
  const menuBar = page.getByTestId('menu-bar')
  await menuBar.getByRole('button', { name: 'File' }).click()
  await page.getByText('Export…', { exact: true }).click()

  const dialog = page.getByRole('dialog', { name: 'Export' })
  await expect(dialog).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(dialog).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

test('palette: the resting search field at the top of the tool rail opens it', async ({
  page,
}) => {
  await page.getByRole('button', { name: 'Search tools, actions, help' }).click()
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog', { name: 'Command palette' })).not.toBeVisible()
})

test('palette: Ctrl+K opens it; running a tool entry activates the tool', async ({ page }) => {
  await page.keyboard.press('Control+k')
  const palette = page.getByRole('dialog', { name: 'Command palette' })
  await expect(palette).toBeVisible()

  await palette.getByRole('textbox', { name: 'Search' }).fill('arc')
  // The Arc tool entry surfaces; run the top match.
  await expect(palette.getByRole('option', { name: /Arc/ }).first()).toBeVisible()
  await page.keyboard.press('Enter')

  await expect(palette).not.toBeVisible()
  await expect(
    page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Arc' }),
  ).toHaveAttribute('aria-checked', 'true')
})
