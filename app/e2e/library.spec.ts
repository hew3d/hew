import { test, expect } from '@playwright/test'

/**
 * Library spec — the browser half of the Library feature, driven with real
 * events. Chromium (this suite's browser) has a real origin-private file
 * system, so alongside the CHROME (every entry point opens the dialog,
 * Escape closes it without leaking into the viewport) this now proves the
 * real browser STORE: save a selection, see it listed, survive a full page
 * reload, and delete it — against actual OPFS, not a mock. The
 * bound-folder (File System Access) mode can't be driven here — its picker
 * needs a real user gesture — and is covered by webLibraryStore.test.ts
 * against fake handles instead. Kernel-side save/insert invariants stay in
 * `crates/kernel/tests/library_specs.rs`.
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

test('web build: an empty library invites a first save (no unavailable state)', async ({ page }) => {
  await page.keyboard.press('Shift+L')
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByText(/save a selection/i)).toBeVisible()
  await expect(dialog.getByText(/isn.t available/i)).toHaveCount(0)
  // The footer names the storage backend honestly.
  await expect(dialog.getByText(/browser storage/i)).toBeVisible()
})

test('web build: save to the library, survive a reload, download offered, delete', async ({ page }) => {
  // A box, selected — the dock's "Save to Library" verb needs a selection.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.selectNodes([{ kind: 'object', id: box }])
  })
  await page.getByRole('button', { name: 'Save to Library' }).click()
  const prompt = page.getByRole('dialog', { name: 'Save to Library' })
  await expect(prompt).toBeVisible()
  await prompt.getByRole('textbox').fill('E2E Test Box')
  await prompt.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(prompt).not.toBeVisible()

  // Listed in the dialog, with the item's bytes in real OPFS.
  await page.keyboard.press('Shift+L')
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await expect(dialog.getByText('E2E Test Box').first()).toBeVisible()

  // A full page reload proves persistence (same origin, same OPFS).
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.keyboard.press('Shift+L')
  await expect(dialog).toBeVisible()
  const tile = dialog.getByText('E2E Test Box').first()
  await expect(tile).toBeVisible()

  // Selecting it offers Download… (the web escape hatch; no Reveal here).
  await tile.click()
  await expect(dialog.getByRole('button', { name: 'Download…' })).toBeVisible()
  await expect(dialog.getByRole('button', { name: /reveal|show in/i })).toHaveCount(0)

  // Delete, confirm, and the library is empty again.
  await dialog.getByRole('button', { name: 'Delete from library…' }).click()
  await dialog.getByRole('button', { name: /^Delete$/ }).click()
  await expect(dialog.getByText('E2E Test Box')).toHaveCount(0)
  await expect(dialog.getByText(/save a selection/i)).toBeVisible()
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
