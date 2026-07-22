import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * Unified Open dialog E2E — the real pipeline in a real browser with the
 * real WASM kernel: ONE dialog (`FileHost.openAny()`), reached from both the
 * Welcome screen's "Open a file…" and File ▸ Open…, accepting `.hew` plus
 * every import format and dispatching by the picked extension.
 *
 * `stl-import.spec.ts`'s "real File ▸ Import… UI" section covers the
 * import-only dialog (`openForImport()`, no `.hew`) end to end already; this
 * file covers the NEW unified dialog specifically — that a `.hew` pick loads
 * straight through with neither the STL units chooser nor the import report
 * appearing, and that an import-format pick through the SAME dialog still
 * reaches those same post-pick steps.
 */

const fixture = (relPath: string): number[] =>
  Array.from(readFileSync(resolve(process.cwd(), relPath)))

/** Install a fake `showOpenFilePicker` that hands the app the given bytes
 * under the given name, then reload so the init script is live before the
 * app boots (mirrors stl-import.spec.ts's armFilePicker). */
async function armFilePicker(page: Page, name: string, bytes: number[]): Promise<void> {
  await page.addInitScript(
    ([n, b]) => {
      const buf = new Uint8Array(b as number[])
      // @ts-expect-error test override of the File System Access API
      window.showOpenFilePicker = async () => [
        {
          getFile: async () => ({
            name: n as string,
            arrayBuffer: async () => buf.buffer,
          }),
        },
      ]
    },
    [name, bytes] as [string, number[]],
  )
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

test('unified Open, via Welcome "Open a file…": a .hew pick loads straight through, no import UI', async ({
  page,
}) => {
  await page.goto('/')
  await page.evaluate(() => localStorage.removeItem('hew.settings.showWelcome'))
  await armFilePicker(page, 'wall-clock.hew', fixture('public/samples/wall-clock.hew'))

  const welcome = page.getByRole('dialog', { name: /welcome to hew/i })
  await expect(welcome).toBeVisible()

  await welcome.getByText('Open a file…').click()
  await expect(welcome).not.toBeVisible()

  // Loaded straight through: no STL units chooser, no import report — the
  // .hew branch applied the bytes directly rather than routing through
  // runImportPick's post-pick import steps.
  await expect(page.getByRole('dialog', { name: /stl import units/i })).toHaveCount(0)
  await expect(page.getByRole('dialog', { name: /import report/i })).toHaveCount(0)
  await page.waitForFunction(() => (window.__hew_test?.getObjectCount() ?? 0) > 0, null, {
    timeout: 15_000,
  })
})

test('unified Open, via File ▸ Open…: an .stl pick routes through the units chooser and report', async ({
  page,
}) => {
  await page.goto('/')
  await armFilePicker(page, 'bracket.stl', fixture('e2e/fixtures/stl/cube_binary.stl'))

  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Open…', { exact: true }).click()

  const chooser = page.getByRole('dialog', { name: /stl import units/i })
  await expect(chooser).toBeVisible()
  await expect(chooser.getByRole('radio', { name: /millimeters/i })).toBeChecked()
  await chooser.getByRole('button', { name: /^import$/i }).click()

  const report = page.getByRole('dialog', { name: /import report/i })
  await expect(report).toBeVisible()
  await expect(report).toContainText('Import Complete')

  await expect.poll(() => page.evaluate(() => window.__hew_test?.getObjectCount() ?? 0)).toBe(1)
})
