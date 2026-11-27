import { expect, test } from '@playwright/test'
import { requiresPreview } from './helpers/preview'
import { bootWithPlugins, drawBox, examplePackage, installPackage, methodOf, observe, openPlugins, waitForState } from './helpers/pluginRealm'
import { callIn, documentFiles, installFileStore, newViaMenu, openViaMenu, pluginDataExists, saveAsViaMenu } from './helpers/pluginStage'

/**
 * A plugin's storage for the model that is open: kept on this device under
 * the model's own id, which a model gets when it is first saved. It follows
 * the model through save, close and reopen, differs between models, leaves
 * the plugin's own storage alone, and goes with the plugin when it is
 * removed.
 */

requiresPreview('plugins')
test.setTimeout(180_000)

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

interface Info {
  documentId: string | null
  document: string
  seen: (string | null)[]
  keys: string[]
  boot: string
}

test('storage for the open model survives a save, a reload and a reopen, and is separate for each model', async ({ page }, testInfo) => {
  const pkg = examplePackage('doc-store')
  const store = await installFileStore(page)
  await bootWithPlugins(page)
  await installPackage(page, pkg)

  const run = (method: string, params: unknown = {}) => callIn(page, 'editor', methodOf(pkg, method), params)
  const info = async (): Promise<Info> => (await run('info')).result as unknown as Info
  const docget = async (key: string): Promise<unknown> => (await run('docget', { key })).result?.value
  const globalget = async (key: string): Promise<unknown> => (await run('globalget', { key })).result?.value
  const seen: Record<string, unknown> = {}

  // An unsaved model has no id; what is kept for it is kept in memory.
  await drawBox(page)
  expect((await run('docset', { key: 'note', value: 'for the first model' })).error).toBeUndefined()
  expect((await run('globalset', { key: 'mine', value: 'for the plugin' })).error).toBeUndefined()
  const unsaved = await info()
  seen.unsaved = { documentId: unsaved.documentId, keys: unsaved.keys, seen: unsaved.seen }
  expect(unsaved.documentId).toBeNull()
  expect(await docget('note')).toBe('for the first model')
  expect(await documentFiles(page, pkg.id), 'nothing is written for a model with no id').toBeNull()

  // The first save gives the model an id; the plugin is told, and what it kept is still there.
  await saveAsViaMenu(page, 'First.hew')
  await expect.poll(async () => (await info()).documentId, { timeout: 15_000 }).toMatch(UUID)
  const afterSave = await info()
  const idFirst = afterSave.documentId!
  seen.afterSave = { documentId: idFirst, seen: afterSave.seen }
  expect(await docget('note')).toBe('for the first model')
  expect(afterSave.seen, 'the plugin was told of the id once').toEqual([idFirst])
  expect(afterSave.boot, 'the plugin did not restart').toBe(unsaved.boot)
  // The write that was waiting for an id is on disk now, under it.
  await expect.poll(() => documentFiles(page, pkg.id)).toEqual([`${idFirst}.json`])

  // A second model: no id, nothing kept, then its own id and its own value.
  await newViaMenu(page)
  await drawBox(page)
  const blank = await info()
  expect(blank.documentId).toBeNull()
  expect(await docget('note'), 'a model other than the first has no value').toBeNull()
  expect(await globalget('mine'), "the plugin's own storage is the same for every model").toBe('for the plugin')
  expect((await run('docset', { key: 'note', value: 'for the second model' })).error).toBeUndefined()
  await saveAsViaMenu(page, 'Second.hew')
  await expect.poll(async () => (await info()).documentId, { timeout: 15_000 }).toMatch(UUID)
  const idSecond = (await info()).documentId!
  expect(idSecond).not.toBe(idFirst)
  expect(await docget('note')).toBe('for the second model')
  await expect.poll(() => documentFiles(page, pkg.id)).toEqual([`${idFirst}.json`, `${idSecond}.json`].sort())

  // Back to the first: its own value again, with the id it had.
  await openViaMenu(page, 'First.hew')
  await expect.poll(async () => (await info()).documentId).toBe(idFirst)
  expect(await docget('note')).toBe('for the first model')

  // A reload: the same file opens with the same id and the same value.
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
  await page.waitForFunction(() => (window as unknown as { __hew_test: { plugins?: unknown } }).__hew_test.plugins !== undefined, null, { timeout: 30_000 })
  await waitForState(page, pkg.id, 'enabled')
  const fresh = await info()
  expect(fresh.documentId, 'after a reload the page holds a new, unsaved model').toBeNull()
  expect(await docget('note')).toBeNull()
  await openViaMenu(page, 'First.hew')
  await expect.poll(async () => (await info()).documentId).toBe(idFirst)
  expect(await docget('note'), 'per-document storage survives save and reopen').toBe('for the first model')
  expect(await globalget('mine')).toBe('for the plugin')
  await openViaMenu(page, 'Second.hew')
  await expect.poll(async () => (await info()).documentId).toBe(idSecond)
  expect(await docget('note')).toBe('for the second model')

  observe(testInfo, 'doc-storage', { ...seen, idFirst, idSecond, files: store.saved, onDisk: await documentFiles(page, pkg.id) })

  // Removing the plugin removes what it kept for every model.
  expect(await pluginDataExists(page, pkg.id)).toBe(true)
  await openPlugins(page)
  await page.locator(`[data-plugin-row="${pkg.id}"]`).getByRole('button', { name: 'Uninstall' }).click()
  await page.getByRole('group', { name: `Uninstall ${pkg.name}?` }).getByRole('button', { name: 'Uninstall' }).click()
  await expect(page.locator(`[data-plugin-row="${pkg.id}"]`)).toHaveCount(0)
  await expect.poll(() => pluginDataExists(page, pkg.id)).toBe(false)
  await expect.poll(() => documentFiles(page, pkg.id)).toBeNull()
})
