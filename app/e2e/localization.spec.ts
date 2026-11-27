import { test, expect } from '@playwright/test'

/**
 * Localization smoke (docs/LOCALIZATION.md): the real UI booted in the
 * pseudo-locale (Debug Mode on, Language = pseudo-LOCALE) renders its
 * chrome through the catalogs — bracketed, padded text everywhere a person
 * reads — and switching the Language setting live re-renders without a
 * reload. Also pins that the command palette still finds a tool by its
 * English name when the labels are not English.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const PSEUDO_OPEN = '⟦'

test.describe('pseudo-locale boot', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem('hew.settings.debugMode', 'true')
      localStorage.setItem('hew.settings.language', 'pseudo-LOCALE')
      // Skip the welcome screen so the editor chrome is on screen.
      localStorage.setItem('hew.settings.showWelcome', 'false')
    })
    await page.goto('/')
    await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  })

  test('menu bar, tool rail, and dialogs read from the catalog', async ({ page }) => {
    await expect(page.locator('html')).toHaveAttribute('lang', 'pseudo-LOCALE')
    const menuBar = page.getByTestId('menu-bar')
    // Every top-level menu title is bracketed.
    // Icon-only buttons (the settings gear) have no text; every titled one
    // is bracketed.
    const titles = (await menuBar.getByRole('button').allTextContents()).filter((t) => t.trim() !== '')
    expect(titles.length).toBeGreaterThan(3)
    for (const title of titles) expect(title, `menu title "${title}"`).toContain(PSEUDO_OPEN)
    // The tool rail's rows carry pseudo names in their accessible labels
    // (the group is a radiogroup whose own label is pseudo too).
    const rail = page.getByRole('radiogroup', { name: /⟦/ }).first()
    await expect(rail).toBeVisible()
    const rows = await rail.getByRole('radio').all()
    expect(rows.length).toBeGreaterThan(5)
    const labels = await Promise.all(rows.map((r) => r.getAttribute('aria-label')))
    const bare = labels.filter((l) => l !== null && !l.includes(PSEUDO_OPEN))
    expect(bare, 'tool rail labels still in English').toEqual([])
    await page.screenshot({ path: 'test-results/pseudo-locale-editor.png', fullPage: true })
  })

  test('the command palette finds Push/Pull by its English name under pseudo labels', async ({ page }) => {
    await page.keyboard.press('ControlOrMeta+/')
    const dialog = page.getByRole('dialog')
    await expect(dialog).toBeVisible()
    await page.keyboard.type('push')
    const first = dialog.getByRole('option').first()
    await expect(first).toContainText(/Ƥũśĥ|Push/)
    await page.keyboard.press('Escape')
  })

  test('switching the Language setting back to English re-renders live', async ({ page }) => {
    const menuBar = page.getByTestId('menu-bar')
    await expect(menuBar.getByRole('button').first()).toContainText(PSEUDO_OPEN)
    await page.evaluate(() => {
      localStorage.setItem('hew.settings.language', 'en')
      window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.language', newValue: 'en' }))
    })
    await expect(menuBar.getByRole('button', { name: 'File', exact: true })).toBeVisible()
    await expect(page.locator('html')).toHaveAttribute('lang', 'en')
    const titles = await menuBar.getByRole('button').allTextContents()
    for (const title of titles) expect(title).not.toContain(PSEUDO_OPEN)
  })
})
