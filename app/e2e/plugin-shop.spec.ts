import { expect, test, type BrowserContext, type Page } from '@playwright/test'
import { requiresPreview } from './helpers/preview'
import { bootWithPlugins, logLines, drawBox, examplePackage, hostilePackage, installPackage, methodOf, observe, type Pkg } from './helpers/pluginRealm'
import {
  callIn,
  hasShopHook,
  installFileStore,
  openShop,
  saveAsViaMenu,
  newViaMenu,
  statesIn,
  waitStateIn,
  type FileStore,
} from './helpers/pluginStage'

/**
 * Plugins in Shop Mode (`#shop`), a read-only viewer in the same bundle: the
 * host starts, plugins get at most what a viewer may do, a plugin cannot ask
 * for more, there is nothing to manage plugins with, and the plugin's storage
 * for the model follows the model opened there. With the Plugins switch off
 * Shop has no plugin frame and no hook.
 */

requiresPreview('plugins')
test.setTimeout(180_000)

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
    __hew_shop_test?: import('../src/shop/testHarness').ShopTestHarness
  }
}

interface World {
  editor: Page
  store: FileStore
  hello: Pkg
  writer: Pkg
  careless: Pkg
  docs: Pkg
  optional: Pkg
}

/** The editor with four plugins installed, and two saved models, each with its own id. */
async function prepare(page: Page): Promise<World> {
  const store = await installFileStore(page)
  await bootWithPlugins(page)
  const hello = examplePackage('hello')
  const docs = examplePackage('doc-store')
  const optional = examplePackage('optional-view')
  const writer = examplePackage('box-writer')
  // The same writer with no handler of its own for a refusal: what Shop does to a plugin that does not expect one.
  const careless = hostilePackage('mutator', { permissions: ['model.read', 'model.write'], commands: [{ id: 'box', class: 'mutating' }] })
  for (const p of [hello, docs, optional, writer, careless]) await installPackage(page, p)
  await drawBox(page)
  await saveAsViaMenu(page, 'First.hew')
  await newViaMenu(page)
  await drawBox(page)
  await drawBox(page)
  await saveAsViaMenu(page, 'Second.hew')
  return { editor: page, store, hello, writer, careless, docs, optional }
}

const loadInShop = (page: Page, bytes: number[]): Promise<boolean> => page.evaluate((b) => window.__hew_shop_test!.load(b), bytes)

async function shopWith(context: BrowserContext, w: World, ids: string[]): Promise<Page> {
  const { page } = await openShop(context)
  for (const id of ids) await waitStateIn(page, 'shop', id, 'enabled', 60_000)
  return page
}

/** The model as a query reads it, without the history clock (a plugin's abandoned transaction moves the epoch, not the model). */
const sceneOf = async (page: Page): Promise<string> => {
  const { epoch: _epoch, ...model } = (await callIn(page, 'shop', 'hew.query.scene', {})).result as Record<string, unknown>
  return JSON.stringify(model)
}

test('the host starts in Shop, answers a read-only command, and refuses one that writes', async ({ page, context }, info) => {
  const w = await prepare(page)
  const shop = await shopWith(context, w, [w.hello.id, w.writer.id, w.careless.id])
  expect(await loadInShop(shop, w.store.files.get('Second.hew')!)).toBe(true)
  expect((await statesIn(shop, 'shop'))[w.hello.id]).toBe('enabled')

  const read = await callIn(shop, 'shop', methodOf(w.hello, 'greet'), { name: 'Shop' })
  expect(read.error).toBeUndefined()
  expect(read.result).toMatchObject({ greeting: 'Hello, Shop!', objects: 2 })
  await waitStateIn(shop, 'shop', w.hello.id, 'activated')

  // The same plugin does write where the model is open for editing.
  const control = await callIn(w.editor, 'editor', methodOf(w.writer, 'box'))
  expect(control.result).toMatchObject({ ok: true })

  const before = await sceneOf(shop)
  const write = await callIn(shop, 'shop', methodOf(w.writer, 'box'))
  expect(write.error).toBeUndefined()
  const outcome = write.result as { ok: boolean; code: number | null; message: string; refusal: string | null }
  observe(info, 'shop-write', outcome)
  expect(outcome.ok, 'the write is refused').toBe(false)
  // The kernel's own refusal: the method is outside the viewer's grant.
  expect(outcome.code).toBe(-32001)
  expect(outcome.message).toBe('method not in the granted profile')
  expect(await sceneOf(shop), 'the model is as it was').toBe(before)
  // The refusal does not stop the plugin that expected it.
  expect((await statesIn(shop, 'shop'))[w.writer.id]).toBe('activated')

  // A plugin that did not expect it is stopped, as anywhere a handler fails with an error of its own.
  const careless = await callIn(shop, 'shop', methodOf(w.careless, 'box'), { count: 1 })
  observe(info, 'shop-write-uncaught', { error: careless.error ?? null, log: (await logLines(shop)).filter((l) => l.source === 'plugins').map((l) => l.message) })
  expect(careless.error).toBeDefined()
  expect(await sceneOf(shop), 'and the model is still as it was').toBe(before)
  // Hew and the other plugins carry on.
  const again = await callIn(shop, 'shop', methodOf(w.hello, 'greet'), { name: 'again' })
  expect(again.result).toMatchObject({ greeting: 'Hello, again!', objects: 2 })
})

test('a plugin cannot ask for more in Shop, and Shop has no way to manage plugins', async ({ page, context }, info) => {
  const w = await prepare(page)
  const shop = await shopWith(context, w, [w.optional.id])
  await loadInShop(shop, w.store.files.get('First.hew')!)

  const reply = await callIn(shop, 'shop', methodOf(w.optional, 'ask'))
  expect(reply.error).toBeUndefined()
  const asked = reply.result as { asked: unknown; refusal: string | null; ok: boolean }
  observe(info, 'shop-request', asked)
  expect(asked.refusal).toContain('not_available_in_shop')
  expect(asked.asked).toBeNull()

  // Nothing in the page manages plugins: no pane, no package input, no switch, no sheet.
  expect(await shop.locator('[data-plugin-row], [data-plugin-detail], input[type=file][aria-label="Plugin package"]').count()).toBe(0)
  expect(await shop.getByRole('dialog').count()).toBe(0)
  // The only plugin element is the host's hidden mount holding realm frames.
  const visibleFrames = await shop.evaluate(() => [...document.querySelectorAll('iframe')].filter((f) => f.getBoundingClientRect().width > 1).length)
  expect(visibleFrames).toBe(0)
  const menus = await shop.evaluate(() => document.body.innerText.toLowerCase())
  expect(menus).not.toContain('plugin')
})

test('with the switch off Shop has no plugin frame and no plugin hook', async ({ page, context }) => {
  const w = await prepare(page)
  const { page: shop } = await openShop(context, { switchOn: false })
  expect(await hasShopHook(shop)).toBe(false)
  expect(await shop.evaluate(() => document.querySelectorAll('iframe[sandbox]').length)).toBe(0)
  // A host that wrongly started would show up late: `openShop` waits for the
  // viewer, not for `loadPluginHost()` (a dynamic import of the host chunk,
  // then a store read), and no signal says "the start was not attempted". So
  // wait for the network to go quiet (the chunk fetch would be done) and then
  // a fixed 3 s for the store read and frame to appear, and only then look.
  // `expect.poll(...).toBe(false)` would pass on its first poll, so it waits
  // for nothing.
  await shop.waitForLoadState('networkidle')
  await shop.waitForTimeout(3_000)
  expect(await hasShopHook(shop)).toBe(false)
  expect(await shop.evaluate(() => document.querySelectorAll('iframe[sandbox]').length)).toBe(0)
  void w
})

test("opening another model in Shop switches a plugin's storage for the model, with no reload", async ({ page, context }, info) => {
  const w = await prepare(page)
  const shop = await shopWith(context, w, [w.docs.id])
  await shop.evaluate(() => void ((window as unknown as { __marker: number }).__marker = 7))
  const run = (method: string, params: unknown = {}) => callIn(shop, 'shop', methodOf(w.docs, method), params)
  const info1 = async (): Promise<{ documentId: string | null; seen: (string | null)[]; boot: string }> =>
    (await run('info')).result as unknown as { documentId: string | null; seen: (string | null)[]; boot: string }

  expect(await loadInShop(shop, w.store.files.get('First.hew')!)).toBe(true)
  const first = await info1()
  expect(first.documentId).toMatch(/^[0-9a-f-]{36}$/)
  expect((await run('docset', { key: 'k', value: 'first' })).error).toBeUndefined()
  expect((await run('docget', { key: 'k' })).result?.value).toBe('first')

  expect(await loadInShop(shop, w.store.files.get('Second.hew')!)).toBe(true)
  await expect.poll(async () => (await info1()).documentId).not.toBe(first.documentId)
  const second = await info1()
  expect(second.documentId).toMatch(/^[0-9a-f-]{36}$/)
  expect((await run('docget', { key: 'k' })).result?.value, 'the other model has none').toBeNull()
  expect((await run('docset', { key: 'k', value: 'second' })).error).toBeUndefined()

  expect(await loadInShop(shop, w.store.files.get('First.hew')!)).toBe(true)
  await expect.poll(async () => (await info1()).documentId).toBe(first.documentId)
  expect((await run('docget', { key: 'k' })).result?.value).toBe('first')
  const last = await info1()
  observe(info, 'shop-doc-switch', { first: first.documentId, second: second.documentId, seen: last.seen, sameRealm: last.boot === first.boot })
  expect(last.boot, 'the plugin kept running across the switches').toBe(first.boot)
  expect(await shop.evaluate(() => (window as unknown as { __marker?: number }).__marker), 'no reload').toBe(7)
})
