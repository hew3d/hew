/**
 * What the plugin stage specs share beyond `pluginRealm.ts`: a package made
 * from an example with its manifest changed, Shop Mode's host hooks, file
 * pickers that keep what is saved in the test (so a model can be saved and
 * opened again, across a reload), and the plugin's data on disk.
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { BrowserContext, Page } from '@playwright/test'
import { storedZipOf as storedZip } from '../../src/plugins/storedZip'
import { applyPreviewPolicy, type Leaks } from './pluginRealm'
import { EXAMPLES_DIR, examplePackage, type Pkg, type RealmState } from './pluginRealm'

// ------------------------------------------------------------- packages

/** An example with its manifest changed by `change` (which edits the parsed manifest in place), as a package. */
export function variantPackage(dirName: string, change: (manifest: Record<string, any>) => void): Pkg {
  const base = examplePackage(dirName)
  const dir = join(EXAMPLES_DIR, dirName)
  const manifest = JSON.parse(readFileSync(join(dir, 'hew-plugin.json'), 'utf8')) as Record<string, any>
  change(manifest)
  const main = readFileSync(join(dir, 'dist/background.js'))
  const buffer = Buffer.from(storedZip({ 'hew-plugin.json': JSON.stringify(manifest), 'dist/background.js': main }))
  return { buffer, id: String(manifest.id ?? base.id), name: String(manifest.name ?? base.name) }
}

// ----------------------------------------------------------------- Shop

interface Hooks {
  call(frame: string): Promise<string>
  states(): Record<string, RealmState>
  reasons(): Record<string, string>
  activate(id: string): void
  listening(): string[]
}
type ShopWindow = { __hew_shop_test?: { plugins?: Hooks; isReady(): boolean; load(bytes: number[]): boolean } }
type EditorWindow = { __hew_test?: { plugins?: Hooks } }

/** The page's plugin hooks: the editor's, or Shop's. */
export type Where = 'editor' | 'shop'

/** A second page in the same browser context (same storage), on Shop Mode, with the switch on. */
export async function openShop(context: BrowserContext, opts: { switchOn?: boolean } = {}): Promise<{ page: Page; leaks: Leaks }> {
  const page = await context.newPage()
  const on = opts.switchOn ?? true
  await page.addInitScript(
    (value) => localStorage.setItem('hew.settings.preview', JSON.stringify(value)),
    on ? { off: [], on: ['plugins'] } : { off: ['plugins'], on: [] },
  )
  const leaks = await applyPreviewPolicy(page)
  await page.goto('/#shop')
  await page.waitForFunction(() => (window as unknown as ShopWindow).__hew_shop_test?.isReady() === true, null, { timeout: 90_000 })
  return { page, leaks }
}

export const hasShopHook = (page: Page): Promise<boolean> =>
  page.evaluate(() => (window as unknown as ShopWindow).__hew_shop_test?.plugins !== undefined)

/** Calls a method on the host's test connection in the editor or in Shop. */
export async function callIn(page: Page, where: Where, method: string, params: unknown = {}): Promise<import('./pluginRealm').Reply> {
  const frame = JSON.stringify({ jsonrpc: '2.0', id: nextId++, method, params })
  const text = await page.evaluate(
    ([w, f]) => {
      const hooks = w === 'shop' ? (window as unknown as ShopWindow).__hew_shop_test?.plugins : (window as unknown as EditorWindow).__hew_test?.plugins
      if (hooks === undefined) throw new Error(`no plugin hooks in ${w}`)
      return hooks.call(f)
    },
    [where, frame] as const,
  )
  const reply = JSON.parse(text) as import('./pluginRealm').Reply
  const r = reply.result
  if (r !== undefined && Object.keys(r).length === 1 && 'r' in r) reply.result = r.r as Record<string, unknown>
  return reply
}
let nextId = 50_000

export function statesIn(page: Page, where: Where): Promise<Record<string, RealmState>> {
  return page.evaluate((w) => {
    const hooks = w === 'shop' ? (window as unknown as ShopWindow).__hew_shop_test?.plugins : (window as unknown as EditorWindow).__hew_test?.plugins
    return hooks === undefined ? {} : hooks.states()
  }, where)
}

export function waitStateIn(page: Page, where: Where, id: string, state: RealmState, timeout = 20_000): Promise<unknown> {
  return page.waitForFunction(
    ([w, i, s]) => {
      const hooks = w === 'shop' ? (window as unknown as ShopWindow).__hew_shop_test?.plugins : (window as unknown as EditorWindow).__hew_test?.plugins
      return hooks?.states()[i] === s
    },
    [where, id, state] as const,
    { timeout, polling: 100 },
  )
}

export function hooksOf<T>(page: Page, where: Where, fn: 'listening' | 'reasons'): Promise<T> {
  return page.evaluate(
    ([w, f]) => {
      const hooks = w === 'shop' ? (window as unknown as ShopWindow).__hew_shop_test?.plugins : (window as unknown as EditorWindow).__hew_test?.plugins
      if (hooks === undefined) throw new Error(`no plugin hooks in ${w}`)
      return hooks[f]() as unknown
    },
    [where, fn] as const,
  ) as Promise<T>
}

export function activateIn(page: Page, id: string): Promise<void> {
  return page.evaluate((i) => (window as unknown as EditorWindow & { __hew_test: { plugins: Hooks } }).__hew_test.plugins.activate(i), id)
}

// ---------------------------------------------------------- file pickers

export interface FileStore {
  files: Map<string, number[]>
  /** Names the save picker was asked to hand out, in order. */
  saved: string[]
}

/**
 * File System Access pickers that keep what is written in the test: the next
 * save picker answers with `saveAs`, the next open picker with `openName`
 * (set through `page.evaluate`, see `nextSave` and `nextOpen`). The store
 * outlives a reload of the page.
 */
export async function installFileStore(page: Page): Promise<FileStore> {
  const store: FileStore = { files: new Map(), saved: [] }
  await page.exposeFunction('__fs_put', (name: string, bytes: number[]) => {
    store.files.set(name, bytes)
    store.saved.push(name)
  })
  await page.exposeFunction('__fs_get', (name: string) => store.files.get(name) ?? null)
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown> & {
      __fs_put(n: string, b: number[]): Promise<void>
      __fs_get(n: string): Promise<number[] | null>
    }
    const handleFor = (name: string) => ({
      kind: 'file',
      name,
      getFile: async () => {
        const bytes = await w.__fs_get(name)
        if (bytes === null) throw new Error(`no such file ${name}`)
        const buf = new Uint8Array(bytes)
        return { name, arrayBuffer: async () => buf.buffer }
      },
      queryPermission: async () => 'granted',
      requestPermission: async () => 'granted',
      createWritable: async () => {
        const chunks: number[] = []
        return {
          write: async (data: ArrayBufferView | ArrayBuffer) => {
            const view = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
            for (const b of view) chunks.push(b)
          },
          close: async () => {
            await w.__fs_put(name, chunks)
          },
        }
      },
    })
    const names = { save: 'Model.hew', open: 'Model.hew' }
    w.__fs_names = names
    w.showSaveFilePicker = async () => handleFor(names.save)
    w.showOpenFilePicker = async () => [handleFor(names.open)]
  })
  return store
}

export const nextSave = (page: Page, name: string): Promise<void> =>
  page.evaluate((n) => void ((window as unknown as { __fs_names: { save: string } }).__fs_names.save = n), name)
export const nextOpen = (page: Page, name: string): Promise<void> =>
  page.evaluate((n) => void ((window as unknown as { __fs_names: { open: string } }).__fs_names.open = n), name)

// ------------------------------------------------------------- the disk

/** File names directly under the origin-private `plugin-data/<id>/documents/`, or null when there is no such folder. */
export function documentFiles(page: Page, id: string): Promise<string[] | null> {
  return page.evaluate(async (i) => {
    try {
      const root = await navigator.storage.getDirectory()
      const dir = await (await (await root.getDirectoryHandle('plugin-data')).getDirectoryHandle(i)).getDirectoryHandle('documents')
      const out: string[] = []
      for await (const [name] of dir.entries()) out.push(name)
      return out.sort()
    } catch {
      return null
    }
  }, id)
}

/** Whether the origin-private `plugin-data/<id>/` folder exists. */
export function pluginDataExists(page: Page, id: string): Promise<boolean> {
  return page.evaluate(async (i) => {
    try {
      const root = await navigator.storage.getDirectory()
      await (await root.getDirectoryHandle('plugin-data')).getDirectoryHandle(i)
      return true
    } catch {
      return false
    }
  }, id)
}

// ------------------------------------------------------------ the menus

/** Closes Settings if it is open, so the menu bar can be reached. */
async function closeSettings(page: Page): Promise<void> {
  const settings = page.getByRole('dialog', { name: 'Settings' })
  if (await settings.isVisible()) {
    await page.keyboard.press('Escape')
    await settings.waitFor({ state: 'hidden' })
  }
}

async function fileMenu(page: Page) {
  await closeSettings(page)
  return page.getByTestId('menu-bar').getByRole('button', { name: 'File' })
}

/** File > Save As… with real clicks; the picker answers with `name`. */
export async function saveAsViaMenu(page: Page, name: string): Promise<void> {
  await nextSave(page, name)
  await (await fileMenu(page)).click()
  await page.getByText('Save As…', { exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Save As' })
  await dialog.waitFor({ state: 'visible' })
  await page.keyboard.press('Enter')
  await dialog.waitFor({ state: 'hidden' })
  await page.waitForFunction((n) => document.title.endsWith(`${n} — Hew`) && !document.title.startsWith('•'), name, { timeout: 15_000 })
}

/** File > Open… of a file the store holds. */
export async function openViaMenu(page: Page, name: string): Promise<void> {
  await nextOpen(page, name)
  await (await fileMenu(page)).click()
  await page.getByText('Open…', { exact: true }).click()
  await page.waitForFunction((n) => document.title.includes(`${n} — Hew`), name, { timeout: 15_000 })
}

/** File > New: a blank, unsaved model. */
export async function newViaMenu(page: Page): Promise<void> {
  await (await fileMenu(page)).click()
  await page.getByTestId('menu-bar').getByText('New', { exact: true }).first().click()
  await page.waitForFunction(() => !document.title.includes('.hew'), null, { timeout: 15_000 })
}
