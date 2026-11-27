import { expect, test, type Page } from '@playwright/test'
import { requiresPreview } from './helpers/preview'
import { bootWithPlugins, call, examplePackage, installPackage, methodOf, observe, waitForState } from './helpers/pluginRealm'

/**
 * What Settings > Plugins says about a plugin that runs, prints and stops:
 * its row, the detail view's Console (newest first, as text, cleaned) and
 * Activity (in plain words), "Clear view" clearing only the view, and
 * Escape closing the detail view and not Settings.
 */

requiresPreview('plugins')
test.setTimeout(120_000)

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
    __pwned?: number
  }
}

const BIDI = /[‪-‮⁦-⁩‎‏]/

async function say(page: Page, id: string, text: string, level = 'log'): Promise<void> {
  const reply = await call(page, `${id}.say`, { text, level })
  expect(reply.error).toBeUndefined()
}

test('a plugin that prints and then stops: its row, console and activity say so in plain words', async ({ page }, info) => {
  await bootWithPlugins(page)
  const pkg = examplePackage('console-lines')
  await installPackage(page, pkg)
  const row = page.locator(`[data-plugin-row="${pkg.id}"]`)
  const settings = page.getByRole('dialog', { name: 'Settings' })

  await say(page, pkg.id, 'first line')
  await say(page, pkg.id, 'second line', 'warn')
  await say(page, pkg.id, '<img src=x onerror="window.__pwned=1">hello')
  await say(page, pkg.id, 'before ‮gnirts‬ ⁦after⁩ end', 'error')
  await say(page, pkg.id, 'last line', 'info')
  await waitForState(page, pkg.id, 'activated')
  await expect(row.locator('[data-plugin-run]')).toHaveText('Running')

  await row.getByRole('button', { name: `Details of ${pkg.name}` }).click()
  const detail = page.getByRole('region', { name: `Details of ${pkg.name}` })
  await detail.getByRole('tab', { name: 'Console' }).click()
  const list = detail.locator('[data-plugin-console]')
  await expect(list.locator('li')).toHaveCount(5)

  // Newest first, as text.
  const lines = await list.locator('li code').allInnerTexts()
  observe(info, 'console-lines', lines)
  expect(lines[0]).toBe('last line')
  expect(lines[lines.length - 1]).toBe('first line')
  expect(lines.indexOf('second line')).toBeGreaterThan(lines.indexOf('last line'))
  // The markup is text: no element came of it, and nothing ran.
  const hostile = lines.find((l) => l.includes('onerror'))
  expect(hostile).toBe('<img src=x onerror="window.__pwned=1">hello')
  expect(await list.locator('img').count()).toBe(0)
  await expect(list.locator('code', { hasText: 'onerror' })).toHaveCount(1)
  // Bidi controls are gone from what is shown, the words are not.
  const bidi = lines.find((l) => l.includes('gnirts'))
  expect(bidi).toBeDefined()
  expect(BIDI.test(bidi!), 'direction controls are cleaned').toBe(false)
  expect(bidi).toContain('after')
  expect(await page.evaluate(() => window.__pwned)).toBeUndefined()

  // Clear view: empties the view, deletes nothing; later lines show.
  await detail.getByRole('button', { name: 'Clear view' }).click()
  await expect(detail.getByText('Nothing printed yet.')).toBeVisible()
  await say(page, pkg.id, 'after the clear')
  await expect(list.locator('li code')).toHaveText(['after the clear'])
  // Clear view is kept while this page is open: leaving the Console tab and coming back does not bring the cleared lines back.
  await detail.getByRole('tab', { name: 'Activity' }).click()
  await detail.getByRole('tab', { name: 'Console' }).click()
  // The view is still cleared: only the line after the clear is shown.
  await expect(list.locator('li code')).toHaveText(['after the clear'])
  observe(info, 'console-after-tab-visit', { lines: await list.locator('li code').count() })
  await detail.getByRole('button', { name: 'Clear view' }).click()

  // Stop it.
  void call(page, methodOf(pkg, 'crash')).catch(() => undefined)
  await waitForState(page, pkg.id, 'crashed', 30_000)
  await detail.getByRole('tab', { name: 'Overview' }).click()
  const run = detail.locator('[data-plugin-run]')
  await expect(run).toContainText('Stopped')
  await expect(run).toContainText('Turn it off and on to start it again.')
  await expect(run).not.toContainText('Running')
  // The plain reason, in Hew's words. (Soft: the rest of the pane is checked either way.)
  await expect.soft(run).toContainText('Stopped: it ran into an error. Turn it off and on')
  const stopped = await run.innerText()
  observe(info, 'pane-stopped', { overview: stopped })
  await detail.getByRole('tab', { name: 'Activity' }).click()
  const activity = detail.locator('[data-plugin-activity] li')
  const entries = await activity.allInnerTexts()
  observe(info, 'pane-activity', entries)
  expect(entries.length).toBeGreaterThanOrEqual(3)
  expect(entries[0]).toMatch(/Stopped/)
  expect.soft(entries[0], 'the activity gives the plain reason').toMatch(/Stopped: it ran into an error/)
  expect(entries.join('\n')).toContain('Starting')
  expect(entries.join('\n')).toContain('Running')

  // Escape closes the detail view and leaves Settings open; the row reads the stop too.
  await page.keyboard.press('Escape')
  await expect(detail).toHaveCount(0)
  await expect(settings).toBeVisible()
  await expect(row.locator('[data-plugin-run]')).toContainText('Stopped')
  await expect(row.locator('[data-plugin-run]')).toContainText('Turn it off and on to start it again.')
  await expect.soft(row.locator('[data-plugin-run]')).toContainText('Stopped: it ran into an error.')
  await page.keyboard.press('Escape')
  await expect(settings).toHaveCount(0)
})
