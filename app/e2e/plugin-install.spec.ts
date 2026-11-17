import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { storedZipOf as storedZip } from '../src/plugins/storedZip'
import { requiresPreview } from './helpers/preview'
import { dumpPluginStore, seedPluginStore } from './helpers/pluginStore'

// Installing is a Preview feature: it needs a preview build and its switch.
requiresPreview('plugins')

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Settings > Plugins on the web, with real events: choose a `.hewplugin`,
 * read the consent sheet, see that Enter installs nothing, install, turn the
 * plugin off and on, reload and find it still installed, uninstall. The
 * package is built here from the example plugin's own files as a stored zip.
 */

const HERE = dirname(fileURLToPath(import.meta.url))
const EXAMPLE = resolve(HERE, '../../plugins/examples/manifest-only')
const ID = 'com.example.manifest-only'

function* filesUnder(dir: string): Generator<string> {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) yield* filesUnder(path)
    else yield path
  }
}

/** `plugins/examples/manifest-only/` as a `.hewplugin`. */
function examplePackage(): Buffer {
  const entries: Record<string, Uint8Array> = {}
  for (const file of filesUnder(EXAMPLE)) {
    entries[relative(EXAMPLE, file).split('\\').join('/')] = readFileSync(file)
  }
  return Buffer.from(storedZip(entries))
}

const switchOn = (page: Page) =>
  page.addInitScript(() => {
    localStorage.setItem('hew.settings.preview', JSON.stringify({ off: [], on: ['plugins'] }))
  })

async function boot(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
}

async function openPlugins(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Settings' }).click()
  await page.getByRole('tab', { name: 'Plugins' }).click()
}

const folderExists = (page: Page): Promise<boolean> =>
  page.evaluate(async (id) => {
    try {
      const root = await navigator.storage.getDirectory()
      const plugins = await root.getDirectoryHandle('plugins')
      await plugins.getDirectoryHandle(id)
      return true
    } catch {
      return false
    }
  }, ID)

test('installs from a file with consent, survives a reload, and uninstalls', async ({ page }) => {
  await switchOn(page)
  await boot(page)
  await openPlugins(page)
  await expect(page.getByText(/runs in a sandbox/)).toBeVisible()
  await expect(page.getByText('No plugins are installed.')).toBeVisible()

  await page.locator('input[type=file][aria-label="Plugin package"]').setInputFiles({
    name: 'manifest-only.hewplugin',
    mimeType: 'application/octet-stream',
    buffer: examplePackage(),
  })

  // The consent sheet: what it can do, Hew's warning, and that nobody signed it.
  const sheet = page.getByRole('dialog', { name: 'Install Manifest Only?' })
  await expect(sheet).toBeVisible()
  await expect(sheet.getByText('Read the model: every object, component, material, tag, and attribute')).toBeVisible()
  await expect(sheet.getByText('Connect to api.example.com (checked by Hew)')).toBeVisible()
  await expect(sheet.getByText('Can send what it reads from your model to api.example.com.')).toBeVisible()
  await expect(sheet.getByText('Unsigned. Hew cannot tell who made this.')).toBeVisible()
  await expect(sheet.getByText('From file')).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Cancel' })).toBeFocused()

  // Enter installs nothing, and the sheet stays.
  await page.keyboard.press('Enter')
  await expect(sheet).toBeVisible()
  expect(await folderExists(page)).toBe(false)

  await sheet.getByRole('button', { name: 'Install' }).click()
  await expect(sheet).toBeHidden()
  const row = page.locator(`[data-plugin-row="${ID}"]`)
  await expect(row).toBeVisible()
  await expect(row).toContainText('Manifest Only')
  await expect(row).toContainText('0.1.0')
  await expect(row).toContainText('From file')
  expect(await folderExists(page)).toBe(true)

  // Off and on again.
  const toggle = row.getByRole('switch', { name: 'Turn Manifest Only on or off' })
  await expect(toggle).toBeChecked()
  await toggle.click()
  await expect(toggle).not.toBeChecked()
  // Turning it on asks first, with Cancel as the default.
  await toggle.click()
  const turnOn = page.getByRole('dialog', { name: 'Turn on this plugin?' })
  await expect(turnOn).toBeVisible()
  await expect(toggle).not.toBeChecked()
  await page.keyboard.press('Enter')
  await expect(toggle).not.toBeChecked()
  await turnOn.getByRole('button', { name: 'Turn On' }).click()
  await expect(toggle).toBeChecked()

  // It is still there after a reload.
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
  await openPlugins(page)
  await expect(page.locator(`[data-plugin-row="${ID}"]`)).toContainText('Manifest Only')
  await expect(page.locator(`[data-plugin-row="${ID}"]`).getByRole('switch')).toBeChecked()

  // Uninstall asks first.
  await page.locator(`[data-plugin-row="${ID}"]`).getByRole('button', { name: 'Uninstall' }).click()
  await expect(page.getByText('Remove Manifest Only and its settings?')).toBeVisible()
  expect(await folderExists(page)).toBe(true)
  await page.getByRole('group', { name: 'Uninstall Manifest Only?' }).getByRole('button', { name: 'Uninstall' }).click()
  await expect(page.locator(`[data-plugin-row="${ID}"]`)).toHaveCount(0)
  await expect(page.getByText('No plugins are installed.')).toBeVisible()
  expect(await folderExists(page)).toBe(false)
})

/**
 * Stands in for the browser's folder picker, which a test cannot click
 * through: `showDirectoryPicker` is replaced, before the app loads, by a stub
 * that answers a fake directory handle over `files` (path to text), whose
 * files are real `File` objects. Everything after the pick is the app's own.
 */
const stubFolderPicker = (page: Page, name: string, files: Record<string, string>) =>
  page.addInitScript(
    ([folder, entries]) => {
      type Node = { dirs: Map<string, Node>; files: Map<string, string> }
      const tree: Node = { dirs: new Map(), files: new Map() }
      for (const [path, text] of Object.entries(entries)) {
        const parts = path.split('/')
        let at = tree
        for (const part of parts.slice(0, -1)) {
          if (!at.dirs.has(part)) at.dirs.set(part, { dirs: new Map(), files: new Map() })
          at = at.dirs.get(part)!
        }
        at.files.set(parts[parts.length - 1], text)
      }
      const handle = (dirName: string, node: Node): unknown => ({
        kind: 'directory',
        name: dirName,
        async *entries() {
          for (const [n, text] of node.files) {
            yield [n, { kind: 'file', name: n, getFile: async () => new File([text], n) }]
          }
          for (const [n, child] of node.dirs) yield [n, handle(n, child)]
        },
      })
      ;(window as unknown as { showDirectoryPicker: () => Promise<unknown> }).showDirectoryPicker = async () => handle(folder, tree)
    },
    [name, files] as const,
  )

/** `plugins/examples/manifest-only/` as path to text. */
function exampleFiles(): Record<string, string> {
  const out: Record<string, string> = {}
  for (const file of filesUnder(EXAMPLE)) out[relative(EXAMPLE, file).split('\\').join('/')] = readFileSync(file, 'utf8')
  return out
}

test('installs a copy of a folder from the folder picker (stubbed), as a file install named after the folder', async ({ page }) => {
  await switchOn(page)
  await stubFolderPicker(page, 'manifest-only', exampleFiles())
  await boot(page)
  await openPlugins(page)
  await page.getByRole('button', { name: 'From folder…' }).click()
  const sheet = page.getByRole('dialog', { name: 'Install Manifest Only?' })
  await expect(sheet).toBeVisible()
  await expect(sheet.getByText('Copied from a folder')).toBeVisible()
  await expect(sheet.locator('[data-path-text] code')).toHaveText('manifest-only')
  await expect(sheet.getByText('Unsigned. Hew cannot tell who made this.')).toBeVisible()
  await sheet.getByRole('button', { name: 'Install' }).click()
  await expect(sheet).toBeHidden()
  const row = page.locator(`[data-plugin-row="${ID}"]`)
  await expect(row).toContainText('Manifest Only')
  await expect(row).toContainText('Copied from a folder')
  await expect(row.locator('[data-path-text] code')).toHaveText('manifest-only')
  // The copy holds the folder's files, byte for byte.
  const stored = await page.evaluate(async (id) => {
    const root = await navigator.storage.getDirectory()
    const files = await (await (await root.getDirectoryHandle('plugins')).getDirectoryHandle(id)).getDirectoryHandle('files')
    const dist = await files.getDirectoryHandle('dist')
    return (await (await dist.getFileHandle('background.js')).getFile()).text()
  }, ID)
  expect(stored).toBe(exampleFiles()['dist/background.js'])
})

test('refuses a folder holding a name a package may not have, and installs nothing', async ({ page }) => {
  await switchOn(page)
  await stubFolderPicker(page, 'manifest-only', { ...exampleFiles(), 'notes:draft.txt': 'x' })
  await boot(page)
  await openPlugins(page)
  await page.getByRole('button', { name: 'From folder…' }).click()
  await expect(page.getByRole('alert')).toContainText('notes:draft.txt')
  // Settings is a dialog itself; no consent sheet opened over it.
  await expect(page.getByRole('dialog', { name: /^Install / })).toHaveCount(0)
  expect(await folderExists(page)).toBe(false)
})

test('Cancel installs nothing', async ({ page }) => {
  await switchOn(page)
  await boot(page)
  await openPlugins(page)
  await page.locator('input[type=file][aria-label="Plugin package"]').setInputFiles({
    name: 'manifest-only.hewplugin',
    mimeType: 'application/octet-stream',
    buffer: examplePackage(),
  })
  const sheet = page.getByRole('dialog', { name: 'Install Manifest Only?' })
  await sheet.getByRole('button', { name: 'Cancel' }).click()
  await expect(sheet).toBeHidden()
  await expect(page.getByText('No plugins are installed.')).toBeVisible()
  expect(await folderExists(page)).toBe(false)
})

test('with the switch off there is no Plugins tab', async ({ page }) => {
  await boot(page)
  await page.getByRole('button', { name: 'Settings' }).click()
  const names = (await page.getByRole('tab').allTextContents()).map((n) => n.toLowerCase())
  expect(names.length).toBeGreaterThan(5)
  expect(names).toContain('preview')
  expect(names).not.toContain('plugins')
})

test('with the switch off, a stored plugin is left alone and Settings has no Plugins tab', async ({ page }) => {
  await boot(page)
  await seedPluginStore(page)
  const before = await dumpPluginStore(page)
  expect(Object.keys(before)).toHaveLength(5)
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
  await page.getByRole('button', { name: 'Settings' }).click()
  const names = (await page.getByRole('tab').allTextContents()).map((n) => n.toLowerCase())
  expect(names.length).toBeGreaterThan(5)
  expect(names).not.toContain('plugins')
  expect(await dumpPluginStore(page)).toEqual(before)
})
