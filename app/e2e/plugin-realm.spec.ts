import { expect, test, type Page } from '@playwright/test'
import { isProductionShape, requiresPreview } from './helpers/preview'
import {
  boot,
  bootWithPlugins,
  call,
  crashInfo,
  drawBox,
  examplePackage,
  hostilePackage,
  installPackage,
  logLines,
  methodOf,
  objectCount,
  observe,
  openPlugins,
  responsive,
  stateHash,
  states,
  switchOn,
  waitForState,
} from './helpers/pluginRealm'

/**
 * The plugin realm in a real browser: a sandboxed frame, a Worker started
 * from a blob, the SDK runtime, and the host that serves a plugin's commands.
 * Packages are built here; the first test goes through Settings > Plugins
 * the way a person does, the rest install through the same file input.
 *
 * What a hostile plugin does to the realm is in plugin-hostile.spec.ts.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const hasFrame = (page: Page): Promise<boolean> => page.evaluate(() => document.querySelector('iframe[sandbox]') !== null)
const hasHook = (page: Page): Promise<boolean> =>
  page.evaluate(() => (window as unknown as { __hew_test?: { plugins?: unknown } }).__hew_test?.plugins !== undefined)

const idsOnDisk = (page: Page): Promise<string[]> =>
  page.evaluate(async () => {
    try {
      const root = await navigator.storage.getDirectory()
      const plugins = await root.getDirectoryHandle('plugins')
      const out: string[] = []
      for await (const [name, handle] of plugins.entries()) if (handle.kind === 'directory' && !name.startsWith('.')) out.push(name)
      return out.sort()
    } catch {
      return []
    }
  })

const storageFileText = (page: Page, id: string): Promise<string | null> =>
  page.evaluate(async (i) => {
    try {
      const root = await navigator.storage.getDirectory()
      const dir = await (await root.getDirectoryHandle('plugins')).getDirectoryHandle(i)
      return await (await (await dir.getFileHandle('storage.json')).getFile()).text()
    } catch {
      return null
    }
  }, id)

const reload = async (page: Page): Promise<void> => {
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
}

// The switch decides whether anything starts; the same assertions hold in
// both builds, so this block never skips.
test.describe('the wall', () => {
  test('with the switch off nothing boots: no frame and no hook', async ({ page }) => {
    await boot(page)
    expect(await hasFrame(page)).toBe(false)
    expect(await hasHook(page)).toBe(false)
  })

  test('the host is there only when both the build and the switch allow it', async ({ page }) => {
    await switchOn(page)
    await boot(page)
    await expect.poll(() => hasHook(page), { timeout: 20_000 }).toBe(!isProductionShape)
    // No plugin is installed, so no realm has been made either way.
    expect(await hasFrame(page)).toBe(false)
  })
})

test.describe('the realm', () => {
  requiresPreview('plugins')
  test.setTimeout(90_000)

  test('the switch off leaves an installed, enabled plugin inert', async ({ page }) => {
    await bootWithPlugins(page)
    const startup = examplePackage('startup')
    const hello = examplePackage('hello')
    await installPackage(page, hello)
    await installPackage(page, startup)
    expect(await idsOnDisk(page)).toEqual([hello.id, startup.id].sort())
    // Switch off, reload: the packages are still on disk, nothing runs.
    await page.addInitScript(() => localStorage.setItem('hew.settings.preview', JSON.stringify({ off: ['plugins'], on: [] })))
    await reload(page)
    expect(await hasHook(page)).toBe(false)
    expect(await hasFrame(page)).toBe(false)
    expect(await idsOnDisk(page)).toEqual([hello.id, startup.id].sort())
    await drawBox(page)
  })

  test('hello: install with consent, answer, turn off and on, survive a reload, uninstall', async ({ page }) => {
    await bootWithPlugins(page)
    const hello = examplePackage('hello')

    // The consent sheet, with real clicks.
    await page.locator('input[type=file][aria-label="Plugin package"]').setInputFiles({
      name: 'hello.hewplugin',
      mimeType: 'application/octet-stream',
      buffer: hello.buffer,
    })
    const sheet = page.getByRole('dialog', { name: 'Install Hello?' })
    await expect(sheet).toBeVisible()
    await expect(sheet.getByText('Read the model: every object, component, material, tag, and attribute')).toBeVisible()
    await expect(sheet.getByText('Unsigned. Hew cannot tell who made this.')).toBeVisible()
    await page.keyboard.press('Enter')
    await expect(sheet).toBeVisible()
    await sheet.getByRole('button', { name: 'Install' }).click()
    await expect(sheet).toBeHidden()
    await waitForState(page, hello.id, 'enabled')
    // Nothing started: a plugin starts on its first command.
    expect(await hasFrame(page)).toBe(false)

    await drawBox(page)
    const first = await call(page, methodOf(hello, 'greet'), { name: 'Kurt' })
    expect(first.result).toEqual({ greeting: 'Hello, Kurt!', objects: 1, times: 1 })
    await waitForState(page, hello.id, 'activated')
    expect(await hasFrame(page)).toBe(true)
    const iframe = await page.evaluate(() => {
      const f = document.querySelector('iframe[sandbox]') as HTMLIFrameElement
      return { sandbox: f.getAttribute('sandbox'), hidden: f.getAttribute('aria-hidden'), count: document.querySelectorAll('iframe').length }
    })
    expect(iframe).toEqual({ sandbox: 'allow-scripts', hidden: 'true', count: 1 })

    // Off: the realm and its frame go, its command is not found.
    const row = page.locator(`[data-plugin-row="${hello.id}"]`)
    const toggle = row.getByRole('switch', { name: 'Turn Hello on or off' })
    await toggle.click()
    await expect(toggle).not.toBeChecked()
    await expect.poll(async () => (await states(page))[hello.id]).toBeUndefined()
    await expect.poll(() => hasFrame(page)).toBe(false)
    const gone = await call(page, methodOf(hello, 'greet'), {})
    expect(gone.error?.code).toBe(-32601)

    // On again asks first; Cancel is the default.
    await toggle.click()
    const turnOn = page.getByRole('dialog', { name: 'Turn on this plugin?' })
    await expect(turnOn).toBeVisible()
    await expect(turnOn.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await turnOn.getByRole('button', { name: 'Turn On' }).click()
    await expect(toggle).toBeChecked()
    await waitForState(page, hello.id, 'enabled')
    // Storage outlives the realm: the count goes on.
    const second = await call(page, methodOf(hello, 'greet'), {})
    expect(second.result).toEqual({ greeting: 'Hello, Hew!', objects: 1, times: 2 })

    // A reload keeps the plugin and its storage.
    await reload(page)
    await expect.poll(async () => (await states(page))[hello.id], { timeout: 20_000 }).toBe('enabled')
    const third = await call(page, methodOf(hello, 'greet'), {})
    expect(third.result).toMatchObject({ times: 3 })
    expect(await storageFileText(page, hello.id)).toBe('{"times":"3"}')

    // Uninstall removes the plugin, its realm and its storage.
    await openPlugins(page)
    await page.locator(`[data-plugin-row="${hello.id}"]`).getByRole('button', { name: 'Uninstall' }).click()
    await page.getByRole('group', { name: 'Uninstall Hello?' }).getByRole('button', { name: 'Uninstall' }).click()
    await expect(page.locator(`[data-plugin-row="${hello.id}"]`)).toHaveCount(0)
    await expect.poll(() => hasFrame(page)).toBe(false)
    expect(await idsOnDisk(page)).toEqual([])
    expect(await storageFileText(page, hello.id)).toBeNull()
    expect((await states(page))[hello.id]).toBeUndefined()
    // A different package with the same id does not inherit anything.
    await installPackage(page, hello)
    const again = await call(page, methodOf(hello, 'greet'), {})
    expect(again.result).toMatchObject({ times: 1 })
  })

  test('startup: runs at startup only after the person turned it on, and says so', async ({ page }) => {
    await bootWithPlugins(page)
    const startup = examplePackage('startup')
    await installPackage(page, startup)
    // Installed but never turned on by hand: it waits for a command.
    expect((await states(page))[startup.id]).toBe('enabled')
    expect(await hasFrame(page)).toBe(false)

    // Turn it off and on by hand: now it may start with Hew, and does.
    const row = page.locator(`[data-plugin-row="${startup.id}"]`)
    const toggle = row.getByRole('switch', { name: 'Turn Startup on or off' })
    await drawBox(page)
    await toggle.click()
    await expect(toggle).not.toBeChecked()
    await toggle.click()
    await page.getByRole('dialog', { name: 'Turn on this plugin?' }).getByRole('button', { name: 'Turn On' }).click()
    await waitForState(page, startup.id, 'activated')
    const status = await call(page, methodOf(startup, 'status'), {})
    expect(status.result).toEqual({ activation: 'startup', ranAtStartup: true, objectsAtStart: 1 })
    expect((await logLines(page)).some((l) => l.message.includes(`Plugin ${startup.id} 0.1.0 started (startup).`))).toBe(true)

    // With Hew starting: no command needed.
    await reload(page)
    await waitForState(page, startup.id, 'activated', 20_000)
    const after = await call(page, methodOf(startup, 'status'), {})
    expect(after.result).toEqual({ activation: 'startup', ranAtStartup: true, objectsAtStart: 0 })
  })

  test('a plugin with the right grant changes the model as one undo step and undoes cleanly', async ({ page }) => {
    await bootWithPlugins(page)
    const pkg = hostilePackage('mutator', {
      permissions: ['model.read', 'model.write'],
      commands: [{ id: 'box', class: 'mutating' }],
    })
    await installPackage(page, pkg)
    const hash0 = await stateHash(page)
    const depth = async (): Promise<number> => ((await call(page, 'hew.history.status', {})).result as { undo_depth: number }).undo_depth
    const d0 = await depth()
    const made = await call(page, methodOf(pkg, 'box'), { count: 3 })
    expect(made.error).toBeUndefined()
    expect((made.result as { made: string[] }).made).toHaveLength(3)
    expect(await objectCount(page)).toBe(3)
    // Three transactions inside one command are one undo step.
    expect(await depth()).toBe(d0 + 1)
    const hash1 = await stateHash(page)
    expect(hash1).not.toBe(hash0)
    expect((await call(page, 'hew.history.undo', {})).error).toBeUndefined()
    expect(await objectCount(page)).toBe(0)
    expect(await stateHash(page)).toBe(hash0)
    expect((await call(page, 'hew.history.redo', {})).error).toBeUndefined()
    expect(await objectCount(page)).toBe(3)
    expect(await stateHash(page)).toBe(hash1)
    expect((await states(page))[pkg.id]).toBe('activated')
  })

  test('a plugin cannot borrow another plugin’s grant, and a refused write leaves no trace', async ({ page }) => {
    await bootWithPlugins(page)
    // Two copies of one plugin: one may write, one may only read.
    const writer = hostilePackage('mutator', { suffix: 'a', permissions: ['model.read', 'model.write'], commands: [{ id: 'box', class: 'mutating' }] })
    const reader = hostilePackage('mutator', { suffix: 'b', permissions: ['model.read'], commands: [{ id: 'box', class: 'mutating' }] })
    await installPackage(page, writer)
    await installPackage(page, reader)
    expect((await call(page, methodOf(writer, 'box'), { count: 1 })).error).toBeUndefined()
    expect(await objectCount(page)).toBe(1)
    const hash0 = await stateHash(page)
    // With the writer running beside it, the reader's first write is still refused.
    const refused = await call(page, methodOf(reader, 'box'), { count: 2 })
    expect(refused.error).toBeDefined()
    expect(await objectCount(page)).toBe(1)
    expect(await stateHash(page)).toBe(hash0)
    expect((await states(page))[writer.id]).toBe('activated')
    await drawBox(page)
  })

  test('nested Workers share one compiled WebAssembly module', async ({ page }) => {
    await bootWithPlugins(page)
    const pkg = hostilePackage('nested-workers')
    await installPackage(page, pkg)
    const reply = await call(page, methodOf(pkg), {})
    expect(reply.error).toBeUndefined()
    expect(reply.result).toEqual({ answers: [11, 12, 13] })
    expect((await states(page))[pkg.id]).toBe('activated')
  })

  test('two plugins keep separate storage', async ({ page }) => {
    await bootWithPlugins(page)
    const a = hostilePackage('storage', { suffix: 'a', permissions: ['model.read', 'model.write'] })
    const b = hostilePackage('storage', { suffix: 'b', permissions: ['model.read'] })
    await installPackage(page, a)
    await installPackage(page, b)
    expect((await call(page, methodOf(a), { op: 'set', key: 'secret', value: 'a-only' })).error).toBeUndefined()
    expect((await call(page, methodOf(b), { op: 'get', key: 'secret' })).result).toEqual({ value: null })
    expect((await call(page, methodOf(b), { op: 'list' })).result).toEqual({ keys: [] })
    expect((await call(page, methodOf(a), { op: 'get', key: 'secret' })).result).toEqual({ value: 'a-only' })
    // The same key in both is two values.
    await call(page, methodOf(b), { op: 'set', key: 'secret', value: 'b-only' })
    expect((await call(page, methodOf(a), { op: 'get', key: 'secret' })).result).toEqual({ value: 'a-only' })
    expect((await call(page, methodOf(b), { op: 'get', key: 'secret' })).result).toEqual({ value: 'b-only' })
    expect(await storageFileText(page, a.id)).toBe('{"secret":"\\"a-only\\""}')
    expect(await storageFileText(page, b.id)).toBe('{"secret":"\\"b-only\\""}')
    expect(await responsive(page)).toBeGreaterThan(0)
  })

  test('storage: reads see writes, a quota of 5 MiB, key bounds', async ({ page }) => {
    await bootWithPlugins(page)
    const pkg = hostilePackage('storage')
    await installPackage(page, pkg)
    const run = (params: unknown) => call(page, methodOf(pkg), params)
    const alive = async (): Promise<void> => {
      expect((await states(page))[pkg.id]).toBe('activated')
    }

    // Read your writes, before the write has reached the file.
    expect((await run({ op: 'readYourWrites' })).result).toEqual({ first: 1, second: 2, gone: null })

    // 5 MiB in all: five values of 900,000 characters fit, the sixth is refused.
    const fill = await run({ op: 'fill', limit: 8 })
    expect(fill.result).toEqual({ stored: 5, refusal: 'storage_quota' })
    await alive()
    expect(((await run({ op: 'list' })).result as { keys: string[] }).keys).toEqual(['fill0', 'fill1', 'fill2', 'fill3', 'fill4'])
    // Deleting makes room.
    await run({ op: 'delete', key: 'fill0' })
    await run({ op: 'delete', key: 'fill1' })
    expect((await run({ op: 'oneValue', chars: 900000 })).result).toEqual({ done: true })
    await alive()

    // Key bounds: empty is a TypeError in the SDK; 256 bytes pass; over 256 bytes is refused by name.
    const empty = await run({ op: 'keyOfLength', chars: 0 })
    expect((empty.result as { threw: string }).threw).toMatch(/^TypeError/)
    expect((await run({ op: 'keyOfLength', chars: 256 })).result).toEqual({ done: true })
    const notJson = await run({ op: 'notJson' })
    expect((notJson.result as { threw: string }).threw).toMatch(/^TypeError/)
    const wide = await run({ op: 'keyOfLength', chars: 129, char: 'é' })
    observe(test.info(), 'storage-key-258-bytes', { reply: wide, state: (await states(page))[pkg.id] })
    expect((wide.result as { threw: string }).threw, 'a 258-byte key of 129 characters is refused by name').toMatch(/^storage_key_invalid:/)
    await alive()
  })

  test('storage: a key over 256 bytes is refused by name, not by stopping the plugin', async ({ page }) => {
    await bootWithPlugins(page)
    const long = hostilePackage('storage', { suffix: 'key' })
    await installPackage(page, long)
    const key = await call(page, methodOf(long), { op: 'keyOfLength', chars: 257 })
    observe(test.info(), 'storage-key-257-ascii', { reply: key, state: (await states(page))[long.id] })
    expect(key.error, 'a 257-byte key must be refused by name, not by stopping the plugin').toBeUndefined()
    expect((key.result as { threw: string }).threw, 'a 257-byte key is refused by name').toMatch(/^storage_key_invalid:/)
    expect((await states(page))[long.id]).toBe('activated')
  })

  test('storage: a value over the message limit is refused, not a reason to stop the plugin', async ({ page }) => {
    await bootWithPlugins(page)
    const huge = hostilePackage('storage', { suffix: 'value' })
    await installPackage(page, huge)
    const value = await call(page, methodOf(huge), { op: 'oneValue', chars: 1100000 })
    observe(test.info(), 'storage-value-1.1M-chars', { reply: value, state: (await states(page))[huge.id], stop: await crashInfo(page, huge) })
    expect(value.error, 'a value over 1 MiB must not stop the plugin').toBeUndefined()
    expect((await states(page))[huge.id]).toBe('activated')
  })

  test('storage survives a reload of the page and is removed with the plugin', async ({ page }) => {
    await bootWithPlugins(page)
    const pkg = hostilePackage('storage')
    await installPackage(page, pkg)
    await call(page, methodOf(pkg), { op: 'set', key: 'keep', value: { n: 7 } })
    await reload(page)
    await expect.poll(async () => (await states(page))[pkg.id], { timeout: 20_000 }).toBe('enabled')
    expect((await call(page, methodOf(pkg), { op: 'get', key: 'keep' })).result).toEqual({ value: { n: 7 } })
    await openPlugins(page)
    await page.locator(`[data-plugin-row="${pkg.id}"]`).getByRole('button', { name: 'Uninstall' }).click()
    await page.getByRole('group', { name: `Uninstall ${pkg.name}?` }).getByRole('button', { name: 'Uninstall' }).click()
    await expect(page.locator(`[data-plugin-row="${pkg.id}"]`)).toHaveCount(0)
    expect(await storageFileText(page, pkg.id)).toBeNull()
    await installPackage(page, pkg)
    expect((await call(page, methodOf(pkg), { op: 'get', key: 'keep' })).result).toEqual({ value: null })
  })
})
