import { expect, test, type Page } from '@playwright/test'
import { requiresPreview } from './helpers/preview'
import { bootWithPlugins, call, examplePackage, installPackage, methodOf, observe, states, waitForState } from './helpers/pluginRealm'
import { activateIn, callIn } from './helpers/pluginStage'

/**
 * Optional permissions, the person's say. In Settings > Plugins > Details a
 * plugin's optional permission is a switch, off until the person turns it on
 * and agrees in a sheet; the running plugin is told and carries on without
 * restarting. Turning it off asks nothing. A plugin that asks for one itself
 * is refused unless the person has just acted in its own UI, and then gets
 * the same sheet, outside Settings, and its call goes on in place.
 */

requiresPreview('plugins')
test.setTimeout(120_000)

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

interface Hidden {
  ok: boolean
  refusal: string | null
  calls: number
  grants: number
  boot: string
}
interface Ask {
  asked: { granted?: unknown } | null
  refusal: string | null
  ok: boolean
  after: string | null
  calls: number
  grants: number
  boot: string
}

const SHEET = 'Allow more for this plugin?'

async function openDetails(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: `Details of ${name}` }).click()
  await expect(page.getByRole('region', { name: `Details of ${name}` })).toBeVisible()
}

test('the switch asks first, the running plugin is told in place, and turning it off asks nothing', async ({ page }, info) => {
  await bootWithPlugins(page)
  const pkg = examplePackage('optional-view')
  await installPackage(page, pkg)
  const hidden = async (): Promise<Hidden> => (await call(page, methodOf(pkg, 'hidden'))).result as unknown as Hidden
  const state = async (): Promise<{ calls: number; grants: number; boot: string }> =>
    (await call(page, methodOf(pkg, 'state'))).result as unknown as { calls: number; grants: number; boot: string }

  // Not granted: the command answers, and what it needs is refused.
  const refused = await hidden()
  observe(info, 'optional-refused', refused)
  expect(refused.ok).toBe(false)
  expect(refused.refusal).toBeTruthy()
  await waitForState(page, pkg.id, 'activated')

  await openDetails(page, pkg.name)
  const sw = page.locator('[data-optional-permission="view"]')
  await expect(sw).not.toBeChecked()
  const sheet = page.getByRole('dialog', { name: SHEET })

  // Turning it on asks. Cancel has focus, Enter does nothing, Cancel leaves it off.
  await sw.click()
  await expect(sheet).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Cancel' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(sheet).toBeVisible()
  await sheet.getByRole('button', { name: 'Cancel' }).click()
  await expect(sheet).toHaveCount(0)
  await expect(sw).not.toBeChecked()
  expect((await hidden()).ok, 'cancelled: still refused').toBe(false)

  // Allow turns it on; the same running realm answers next, having been told once.
  const before = await state()
  await sw.click()
  await expect(sheet).toBeVisible()
  await sheet.getByRole('button', { name: 'Allow' }).click()
  await expect(sheet).toHaveCount(0)
  await expect(sw).toBeChecked()
  await expect.poll(async () => (await state()).grants, { timeout: 10_000 }).toBe(1)
  const granted = await hidden()
  observe(info, 'optional-granted', { before, granted })
  expect(granted.ok, 'the call succeeds without a reload').toBe(true)
  expect(granted.boot, 'the same running realm answered').toBe(before.boot)
  expect(granted.grants, 'the grant callback ran once').toBe(1)
  expect(granted.calls).toBeGreaterThan(before.calls)
  expect((await states(page))[pkg.id]).toBe('activated')

  // Off asks nothing; the next call is refused (the plugin may have restarted).
  await sw.click()
  await expect(sw).not.toBeChecked()
  await expect(sheet).toHaveCount(0)
  await expect.poll(async () => (await hidden()).ok, { timeout: 15_000 }).toBe(false)
})

test('a request from the plugin is refused without an act by the person, and with one shows the sheet and goes on in place', async ({ page }, info) => {
  await bootWithPlugins(page)
  const pkg = examplePackage('optional-view')
  await installPackage(page, pkg)
  const ask = (): Promise<import('./helpers/pluginRealm').Reply> => callIn(page, 'editor', methodOf(pkg, 'ask'))

  const quiet = (await ask()).result as unknown as Ask
  observe(info, 'request-no-act', quiet)
  expect(quiet.refusal).toContain('no_user_activation')
  expect(quiet.ok, 'and nothing was granted').toBe(false)

  // Leave Settings: the sheet is the plugin's, shown over the document window.
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog', { name: 'Settings' })).toHaveCount(0)

  // The person acts in the plugin's own UI (the harness stands in), then the plugin asks.
  const running = (await call(page, methodOf(pkg, 'state'))).result as unknown as { boot: string }
  await activateIn(page, pkg.id)
  let answered = false
  const asking = ask().finally(() => {
    answered = true
  })
  const sheet = page.getByRole('dialog', { name: SHEET })
  await expect(sheet).toBeVisible()
  expect(answered, 'the command waits for the person').toBe(false)
  await expect(sheet.getByRole('button', { name: 'Cancel' })).toBeFocused()
  await sheet.getByRole('button', { name: 'Allow' }).click()
  const done = (await asking).result as unknown as Ask
  observe(info, 'request-with-act', done)
  expect(done.refusal).toBeNull()
  expect(done.asked).toBeTruthy()
  expect(done.ok, 'the read after the request, in the same command, succeeds').toBe(true)
  expect(done.boot, 'the plugin did not restart').toBe(running.boot)
  expect(done.grants).toBe(1)
})
