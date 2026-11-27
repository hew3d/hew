import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * Model Health, "Rejoin as one solid" — the real UI, real wasm kernel, and
 * the real `hew.solid.rejoin` bus command. Two copies of the open cube sit
 * exactly on each other and share their open rim, which the diagnosis reads
 * as one model split at import; the third button joins them into one
 * watertight Object, and one Undo brings both pieces back.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

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

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('rejoin_as_one_solid_turns_a_split_board_into_one_watertight_object_in_one_undo', async ({ page }) => {
  await armFilePicker(page, 'cube_open.stl', fixture('cube_open.stl'))
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
  await page.getByRole('dialog', { name: /stl import units/i }).getByRole('button', { name: /^import$/i }).click()
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  await page.evaluate((bytes) => {
    window.__hew_test!.importStl(bytes, 0.001)
  }, fixture('cube_open.stl'))

  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object' }).click()
  await page.getByTestId('menu-bar').getByText('Model Health…', { exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'Model Health' })).toBeVisible()
  const family = page.locator('[data-health-group^="family-"]')
  await expect(family).toHaveCount(1)
  await expect(page.locator('[data-health-row]')).toHaveCount(2)
  await expect(family.getByRole('button', { name: 'Rejoin', exact: true })).toBeVisible()

  await family.getByRole('button', { name: 'Rejoin as one solid' }).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  // One Undo brings both pieces back; one Redo returns the one solid.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText(/^Undo\b/).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(2)
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText(/^Redo\b/).click()
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
})
