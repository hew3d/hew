import { test, expect } from '@playwright/test'

/**
 * Library chrome spec — the browser-reachable half of the Library feature,
 * driven with real events. The web build's storage backend is honestly
 * unavailable (desktop-only for now), so this file proves the CHROME:
 * every entry point opens the browser dialog, the dialog reports the
 * unavailable state instead of a broken one, and Escape closes it without
 * leaking into the viewport's own Escape handling. The full save/insert
 * loop is covered by kernel specs (`crates/kernel/tests/library_specs.rs`),
 * the wasm record/replay test, and the LibraryDialog component tests —
 * this spec pins the wiring between them and the shell.
 *
 * Assertions go through ARIA state, matching ui-chrome.spec.ts.
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

test('tool rail: the Library row opens the browser dialog', async ({ page }) => {
  const row = page.getByRole('button', { name: 'Library' })
  await row.click()
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await expect(dialog).toBeVisible()
  await expect(row).toHaveAttribute('aria-pressed', 'true')
  // The modal scrim covers the rail while open (by design), so closing
  // goes through Escape; the row's pressed state follows.
  await page.keyboard.press('Escape')
  await expect(dialog).not.toBeVisible()
  await expect(row).toHaveAttribute('aria-pressed', 'false')
})

test('keyboard: Shift+L toggles the dialog; Escape closes it without reaching the viewport', async ({
  page,
}) => {
  await page.keyboard.press('Shift+L')
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await expect(dialog).toBeVisible()

  // Escape closes the dialog only — the Select tool stays active and no
  // context pop happens underneath (the dialog's handler stops
  // propagation, the shared modal contract).
  await page.keyboard.press('Escape')
  await expect(dialog).not.toBeVisible()
  const rail = page.getByRole('radiogroup', { name: 'Tools' })
  await expect(rail.getByRole('radio', { name: 'Select' })).toHaveAttribute('aria-checked', 'true')

  // And Shift+L while typing in a field must NOT open it (isTyping guard).
  await page.keyboard.press('Shift+L')
  await expect(dialog).toBeVisible()
  const search = page.getByPlaceholder('Search name or keyword…')
  await search.click()
  await search.press('Shift+L')
  await expect(search).toHaveValue('L')
  await page.keyboard.press('Escape')
})

test('web build: the dialog reports the library honestly unavailable', async ({ page }) => {
  await page.keyboard.press('Shift+L')
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByText(/isn.t available in the browser/i)).toBeVisible()
})

test('command palette: Library entry opens the dialog', async ({ page }) => {
  await page.keyboard.press('ControlOrMeta+K')
  const palette = page.getByRole('dialog', { name: /command palette/i })
  await expect(palette).toBeVisible()
  await palette.getByPlaceholder(/search/i).fill('library')
  await page.keyboard.press('Enter')
  await expect(page.getByRole('dialog', { name: 'Library' })).toBeVisible()
})

test('menu bar: Window ▸ Library opens the dialog', async ({ page }) => {
  // The in-app menu primitives render plain rows (no menuitem roles — a
  // pre-existing pattern), so this drives them the way a user does: open
  // the Window menu, click the Library row's text.
  await page.getByRole('button', { name: 'Window' }).click()
  // Two exact "Library" texts exist once the menu is open (the rail row
  // and the menu row); the menu's mounts later in the DOM.
  await page.getByText('Library', { exact: true }).last().click()
  await expect(page.getByRole('dialog', { name: 'Library' })).toBeVisible()
})
