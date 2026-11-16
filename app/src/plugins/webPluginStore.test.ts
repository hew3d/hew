/**
 * The web plugin store on the in-memory file system: the layout it writes,
 * the record rules it mirrors from the desktop store, and what it refuses.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { FakeDir, linkedChannels } from '../test/opfsFake'
import { storedZip } from '../test/storedZip'
import { fakeWasm, indexStoredZip, manifestJson, pluginFile, textAt, treeOf, type ListedFile } from './testKit'
import { makeWebPluginStore, type PluginWasm, type WebPluginStore } from './webPluginStore'
import { PluginError } from './pluginError'
import type { Installed } from './types'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

function setup(over: { now?: () => number; channel?: BroadcastChannel | null; wasm?: PluginWasm; root?: FakeDir } = {}) {
  const root = over.root ?? new FakeDir('opfs-root')
  const keys = new Map<string, string>()
  const storage = {
    get length() {
      return keys.size
    },
    key: (i: number) => [...keys.keys()][i] ?? null,
    removeItem: (k: string) => void keys.delete(k),
  }
  let tokens = 0
  const store = makeWebPluginStore({
    wasm: async () => over.wasm ?? fakeWasm(),
    opfsRoot: async () => root as unknown as FileSystemDirectoryHandle,
    channel: over.channel ?? null,
    storage,
    now: over.now ?? (() => 1_000),
    token: () => String(++tokens).padStart(32, '0'),
    hewVersion: () => '1.5.0',
  })
  return { root, keys, store }
}

async function install(store: WebPluginStore, over = {}, extra = {}): Promise<Installed> {
  const staged = await store.stageFile(pluginFile(over, extra))
  const record = await store.commit(staged.token, true)
  if (record === null) throw new Error('expected an install')
  return record
}

const refusal = async (p: Promise<unknown>): Promise<string> => {
  try {
    await p
  } catch (e) {
    return (e as PluginError).refusal
  }
  return 'none'
}

describe('stage and commit', () => {
  it('shows the consent and writes nothing until the answer is yes', async () => {
    const { root, store } = setup()
    const staged = await store.stageFile(pluginFile({ permissions: ['model.read', 'network:api.example.com'] }))
    expect(staged.consent.name).toBe('One')
    expect(staged.consent.unsigned).toBe(true)
    expect(staged.consent.source).toEqual({ kind: 'file', name: 'one.hewplugin' })
    expect(treeOf(root)).toEqual([])
    expect(await store.commit(staged.token, false)).toBeNull()
    expect(treeOf(root)).toEqual([])
    expect(await store.list()).toEqual([])
    // The token is spent.
    expect(await refusal(store.commit(staged.token, true))).toBe('pending_unknown')
  })

  it('writes the layout and a fresh record', async () => {
    const { root, store } = setup()
    const record = await install(store, { permissions: ['model.read'], optionalPermissions: ['files'] })
    expect(treeOf(root)).toEqual([
      'plugins/com.example.one/files/dist/background.js',
      'plugins/com.example.one/files/hew-plugin.json',
      'plugins/com.example.one/hew-plugin.json',
      'plugins/com.example.one/installed.json',
    ])
    expect(record).toMatchObject({
      schema: 1,
      id: 'com.example.one',
      version: '1.0.0',
      source: { kind: 'file', name: 'one.hewplugin' },
      grant: ['model.read'],
      optional_granted: [],
      enabled: true,
      startup_allowed: false,
      data_version: 1,
      signer: null,
    })
    const onDisk = JSON.parse((await textAt(root, 'plugins/com.example.one/installed.json')) as string)
    expect(Object.keys(onDisk)[0]).toBe('schema')
    expect(onDisk).toEqual(record)
    const listed = await store.list()
    expect(listed).toHaveLength(1)
    expect(listed[0]).toMatchObject({ kind: 'installed', previous_version: null })
  })

  it('refuses a file that is not a .hewplugin, an expired install and a bad id, and writes nothing', async () => {
    let clock = 1_000
    const { root, store } = setup({ now: () => clock })
    expect(await refusal(store.stageFile(new File([new Uint8Array([1])], 'one.zip')))).toBe('package_unreadable')
    const staged = await store.stageFile(pluginFile())
    clock += 10 * 60 * 1000 + 1
    expect(await refusal(store.commit(staged.token, true))).toBe('pending_expired')
    expect(treeOf(root)).toEqual([])
    for (const id of ['a/../b.c', '..', 'x', 'org.Hew.x', 'hew.core']) {
      expect(await refusal(store.stageFile(pluginFile({ id }))), id).toBe('invalid_id')
    }
    expect(treeOf(root)).toEqual([])
  })

  it('carries the kernel refusals through with their names and text', async () => {
    const { store } = setup()
    await expect(store.stageFile(pluginFile({ id: 'org.hew3d.x' }))).rejects.toMatchObject({
      refusal: 'reserved_id',
      message: '"org.hew3d.x" is kept for plugins Hew publishes.',
    })
    expect(await refusal(store.stageFile(pluginFile({ requires: [{ id: 'com.example.base', range: '*' }] })))).toBe(
      'dependency_missing',
    )
  })

  it('serializes overlapping calls', async () => {
    const { store } = setup()
    const [a, b] = await Promise.all([store.stageFile(pluginFile()), store.stageFile(pluginFile({ id: 'com.example.two' }))])
    await Promise.all([store.commit(a.token, true), store.commit(b.token, true)])
    expect((await store.list()).map((l) => (l.kind === 'installed' ? l.installed.id : l.id))).toEqual([
      'com.example.one',
      'com.example.two',
    ])
  })
})

describe('updates, rollback', () => {
  it('keeps the previous version, the enabled state and unknown fields', async () => {
    const { root, store } = setup()
    await install(store, { version: '1.0.0', optionalPermissions: ['files'] })
    await store.setEnabled('com.example.one', false)
    // A record field this build does not know survives an update.
    const path = 'plugins/com.example.one/installed.json'
    const folder = root.dirs.get('plugins')!.dirs.get('com.example.one')!
    const rec = JSON.parse((await textAt(root, path)) as string)
    rec.optional_granted = ['files']
    rec.future_field = { keep: 'me' }
    folder.files.set('installed.json', { bytes: new TextEncoder().encode(JSON.stringify(rec)), mtime: 1 })

    const staged = await store.stageFile(pluginFile({ version: '2.0.0', optionalPermissions: ['files'] }))
    expect(staged.consent.update?.from_version).toBe('1.0.0')
    expect(staged.consent.replaces).toEqual({ name: 'One', version: '1.0.0' })
    const record = await store.commit(staged.token, true)
    expect(record).toMatchObject({ version: '2.0.0', enabled: false, optional_granted: ['files'], future_field: { keep: 'me' } })
    expect(treeOf(root)).toContain('plugins/com.example.one/previous/installed.json')
    const listed = await store.list()
    expect(listed[0]).toMatchObject({ previous_version: '1.0.0' })
    expect(await textAt(root, 'plugins/com.example.one/previous/hew-plugin.json')).toContain('"1.0.0"')
    expect(treeOf(root).some((p) => p.includes('previous/previous'))).toBe(false)

    // A third version drops the oldest.
    await install(store, { version: '3.0.0' })
    expect(await textAt(root, 'plugins/com.example.one/previous/hew-plugin.json')).toContain('"2.0.0"')
    expect(treeOf(root).some((p) => p.includes('previous/previous'))).toBe(false)
  })

  it('drops an optional grant the new version no longer declares', async () => {
    const { root, store } = setup()
    await install(store, { optionalPermissions: ['files'] })
    const folder = root.dirs.get('plugins')!.dirs.get('com.example.one')!
    const rec = JSON.parse((await textAt(root, 'plugins/com.example.one/installed.json')) as string)
    rec.optional_granted = ['files']
    folder.files.set('installed.json', { bytes: new TextEncoder().encode(JSON.stringify(rec)), mtime: 1 })
    const record = await install(store, { version: '2.0.0', optionalPermissions: [] })
    expect(record.optional_granted).toEqual([])
  })

  it('rolls back and forward again', async () => {
    const { root, store } = setup()
    expect(await refusal(store.rollback('com.example.one'))).toBe('not_installed')
    await install(store, { version: '1.0.0' })
    expect(await refusal(store.rollback('com.example.one'))).toBe('no_previous_version')
    await install(store, { version: '2.0.0' })
    expect((await store.rollback('com.example.one')).version).toBe('1.0.0')
    expect(await textAt(root, 'plugins/com.example.one/previous/hew-plugin.json')).toContain('"2.0.0"')
    expect((await store.rollback('com.example.one')).version).toBe('2.0.0')
    expect(treeOf(root).some((p) => p.includes('.staging/'))).toBe(false)
  })
})

describe('previous and the commit recheck', () => {
  it('reads the version a rollback would restore, without changing anything', async () => {
    const { root, store } = setup()
    expect(await refusal(store.previous('com.example.one'))).toBe('not_installed')
    await install(store, { version: '1.0.0', permissions: ['model.read'] })
    expect(await refusal(store.previous('com.example.one'))).toBe('no_previous_version')
    await install(store, { version: '2.0.0', permissions: ['model.read', 'files'] })
    const before = treeOf(root)
    const prev = await store.previous('com.example.one')
    expect(prev.installed.version).toBe('1.0.0')
    expect(prev.manifest.permissions).toEqual(['model.read'])
    expect(treeOf(root)).toEqual(before)
  })

  it('refuses a commit whose dependency was removed after the sheet, and leaves nothing behind', async () => {
    const { root, store } = setup()
    await install(store, { id: 'com.example.base' })
    const staged = await store.stageFile(pluginFile({ requires: [{ id: 'com.example.base', range: '*' }] }))
    await store.uninstall('com.example.base')
    const before = treeOf(root)
    expect(await refusal(store.commit(staged.token, true))).toBe('dependency_missing')
    expect(treeOf(root)).toEqual(before)
    expect(treeOf(root).some((p) => p.includes('com.example.one'))).toBe(false)
  })
})

describe('on, off, uninstall', () => {
  it('turning a plugin on by hand allows startup; a fresh install does not', async () => {
    const { store } = setup()
    expect((await install(store)).startup_allowed).toBe(false)
    expect(await store.setEnabled('com.example.one', false)).toEqual([])
    let l = (await store.list())[0]
    expect(l.kind === 'installed' && l.installed).toMatchObject({ enabled: false, startup_allowed: false })
    await store.setEnabled('com.example.one', true)
    l = (await store.list())[0]
    expect(l.kind === 'installed' && l.installed).toMatchObject({ enabled: true, startup_allowed: true })
  })

  it('turns off the plugins that need the one turned off, and refuses to remove it while they run', async () => {
    const { store } = setup()
    await install(store, { id: 'com.example.base', name: 'Base' })
    await install(store, { id: 'com.example.mid', requires: [{ id: 'com.example.base', range: '*' }] })
    await install(store, { id: 'com.example.top', requires: [{ id: 'com.example.mid', range: '*' }] })
    expect(await refusal(store.uninstall('com.example.base'))).toBe('dependency_missing')
    const also = await store.setEnabled('com.example.base', false)
    expect(also.sort()).toEqual(['com.example.mid', 'com.example.top'])
    const listed = await store.list()
    expect(listed.map((l) => l.kind === 'installed' && l.installed.enabled)).toEqual([false, false, false])
    await store.uninstall('com.example.base')
    expect((await store.list()).map((l) => (l.kind === 'installed' ? l.installed.id : l.id))).toEqual([
      'com.example.mid',
      'com.example.top',
    ])
  })

  it('purges the plugin folder, its staging entries and only its own keys', async () => {
    const { root, keys, store } = setup()
    await install(store)
    await install(store, { id: 'com.example.one-more' })
    keys.set('hew.plugin.com.example.one.prefs', 'x')
    keys.set('hew.plugin.com.example.one-more.prefs', 'y')
    keys.set('hew.settings.units', 'z')
    const plugins = root.dirs.get('plugins')!
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    const token = '0123456789abcdef0123456789abcdef'
    await staging.getDirectoryHandle(`com.example.one-${token}`, { create: true })
    await staging.getDirectoryHandle(`com.example.one-more-${token}`, { create: true })
    await store.uninstall('com.example.one')
    expect([...keys.keys()].sort()).toEqual(['hew.plugin.com.example.one-more.prefs', 'hew.settings.units'])
    expect(plugins.dirs.has('com.example.one')).toBe(false)
    expect([...staging.dirs.keys()]).toEqual([`com.example.one-more-${token}`])
    expect(await refusal(store.uninstall('com.example.one'))).toBe('not_installed')
  })
})

describe('unreadable records', () => {
  it('lists a record from a newer Hew as unreadable and never rewrites it', async () => {
    const { root, store } = setup()
    await install(store)
    const folder = root.dirs.get('plugins')!.dirs.get('com.example.one')!
    const newer = new TextEncoder().encode(JSON.stringify({ schema: 2, id: 'com.example.one', future: true }))
    folder.files.set('installed.json', { bytes: newer, mtime: 1 })
    const [l] = await store.list()
    expect(l).toMatchObject({ kind: 'unreadable', id: 'com.example.one' })
    expect(await refusal(store.setEnabled('com.example.one', false))).toBe('installed_unreadable')
    expect(await refusal(store.stageFile(pluginFile({ version: '2.0.0' })))).toBe('installed_unreadable')
    expect(await refusal(store.rollback('com.example.one'))).toBe('installed_unreadable')
    expect(folder.files.get('installed.json')!.bytes).toEqual(newer)
    // It can still be removed.
    await store.uninstall('com.example.one')
    expect(await store.list()).toEqual([])
  })

  it('lists a folder with no record, or a record naming another plugin, as unreadable', async () => {
    const { root, store } = setup()
    await install(store)
    await install(store, { id: 'com.example.two' })
    const plugins = root.dirs.get('plugins')!
    plugins.dirs.get('com.example.one')!.files.delete('installed.json')
    const two = plugins.dirs.get('com.example.two')!
    const rec = JSON.parse((await textAt(root, 'plugins/com.example.two/installed.json')) as string)
    rec.id = 'com.example.other'
    two.files.set('installed.json', { bytes: new TextEncoder().encode(JSON.stringify(rec)), mtime: 1 })
    const listed = await store.list()
    expect(listed.map((l) => l.kind)).toEqual(['unreadable', 'unreadable'])
  })

  it('ignores staging entries and folders that are not plugin ids', async () => {
    const { root, store } = setup()
    await install(store)
    const plugins = root.dirs.get('plugins')!
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    await staging.getDirectoryHandle('com.example.one-0123456789abcdef0123456789abcdef', { create: true })
    await plugins.getDirectoryHandle('Not An Id', { create: true })
    expect(await store.list()).toHaveLength(1)
  })

  it('clears a leftover staging entry of the same id, and only of that id', async () => {
    const { root, store } = setup()
    const plugins = await root.getDirectoryHandle('plugins', { create: true })
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    const token = '0123456789abcdef0123456789abcdef'
    await staging.getDirectoryHandle(`com.example.one-${token}`, { create: true })
    await staging.getDirectoryHandle(`com.example.one-more-${token}`, { create: true })
    await store.stageFile(pluginFile())
    expect([...(staging as unknown as FakeDir).dirs.keys()]).toEqual([`com.example.one-more-${token}`])
  })
})

describe('reading a file back', () => {
  it('reads package files and refuses any path out of the folder', async () => {
    const { store } = setup()
    await install(store)
    expect(new TextDecoder().decode(await store.readFile('com.example.one', 'dist/background.js'))).toBe('// background')
    for (const rel of ['../installed.json', 'dist/../../installed.json', '/etc/passwd', 'dist\\background.js', '', 'a:b', 'dist//x', './x', 'installed.json']) {
      expect(await refusal(store.readFile('com.example.one', rel)), rel).toBe('package_path_invalid')
    }
    expect(await refusal(store.readFile('..', 'x'))).toBe('invalid_id')
    expect(await refusal(store.readFile('com.example.none', 'x'))).toBe('not_installed')
  })
})

describe('change notices', () => {
  it('tells local listeners and other tabs', async () => {
    const [tabA, tabB] = linkedChannels()
    const a = setup({ channel: tabA })
    const b = makeWebPluginStore({
      wasm: async () => fakeWasm(),
      opfsRoot: async () => a.root as unknown as FileSystemDirectoryHandle,
      channel: tabB,
      storage: null,
    })
    let local = 0
    let other = 0
    a.store.onChanged(() => local++)
    const off = b.onChanged(() => other++)
    await install(a.store)
    expect([local, other]).toEqual([1, 1])
    off()
    await a.store.setEnabled('com.example.one', false)
    expect([local, other]).toEqual([2, 1])
  })
})

it('writes the manifest bytes exactly as the package held them', async () => {
  const { root, store } = setup()
  await install(store, { name: 'Spacing   kept' })
  const written = await textAt(root, 'plugins/com.example.one/hew-plugin.json')
  expect(written).toBe(manifestJson({ name: 'Spacing   kept' }))
})

describe('what the kernel lists is all the store reads', () => {
  const MAIN = '// the real background script'

  it('ignores a fake end record appended to the archive', async () => {
    const real = storedZip({ 'hew-plugin.json': manifestJson(), 'dist/background.js': MAIN })
    // A second end-of-archive record at the end, pointing at a directory that
    // names other bytes. A reader that takes the last one reads the wrong file.
    const evil = storedZip({ 'hew-plugin.json': manifestJson({ permissions: ['model.write'] }), 'dist/background.js': 'EVIL' })
    const bytes = new Uint8Array(real.length + evil.length)
    bytes.set(real, 0)
    bytes.set(evil, real.length)
    // The kernel's list: the first archive only.
    const list: ListedFile[] = indexStoredZip(real)
    const { root, store } = setup({ wasm: fakeWasm({ files: () => list }) })
    const staged = await store.stageFile(new File([bytes], 'one.hewplugin'))
    expect(staged.consent.permissions.map((p) => p.name)).toEqual(['model.read'])
    await store.commit(staged.token, true)
    expect(await textAt(root, 'plugins/com.example.one/files/dist/background.js')).toBe(MAIN)
    expect(await textAt(root, 'plugins/com.example.one/hew-plugin.json')).toBe(manifestJson())
    expect(treeOf(root).filter((p) => p.includes('EVIL'))).toEqual([])
  })

  it('refuses a listed range outside the archive, and a manifest that is not the listed file', async () => {
    const file = pluginFile()
    const bytes = new Uint8Array(await file.arrayBuffer())
    const good = indexStoredZip(bytes)
    for (const bad of [
      good.map((f) => (f.path === 'hew-plugin.json' ? f : { ...f, size: f.size + 100000 })),
      good.map((f) => (f.path === 'hew-plugin.json' ? f : { ...f, offset: -1 })),
      good.map((f) => (f.path === 'hew-plugin.json' ? f : { ...f, offset: 1.5 })),
    ]) {
      const { root, store } = setup({ wasm: fakeWasm({ files: () => bad }) })
      expect(await refusal(store.stageFile(file))).toBe('package_unreadable')
      expect(treeOf(root)).toEqual([])
    }
    const lying = fakeWasm()
    const real = lying.plugin_package_from_zip
    lying.plugin_package_from_zip = (b) => {
      const v = JSON.parse(real(b))
      v.manifest.permissions = ['model.write']
      return JSON.stringify(v)
    }
    const { store } = setup({ wasm: lying })
    expect(await refusal(store.stageFile(file))).toBe('package_unreadable')
  })
})

describe('an interrupted install', () => {
  afterEach(() => vi.restoreAllMocks())

  /** Makes writing `installed.json` into a folder named `dir` fail `times` times. */
  function failRecordWrites(dir: string, times: number): void {
    const real = FakeDir.prototype.getFileHandle
    let left = times
    vi.spyOn(FakeDir.prototype, 'getFileHandle').mockImplementation(async function (this: FakeDir, name: string, o?: { create?: boolean }) {
      if (name === 'installed.json' && o?.create && this.name === dir && left > 0) {
        left--
        throw new Error('QuotaExceededError: disk full')
      }
      return real.call(this, name, o)
    })
  }

  it('a failed first install leaves a folder with no record, listed as unfinished, and installing again replaces it', async () => {
    const { root, store } = setup()
    const staged = await store.stageFile(pluginFile())
    failRecordWrites('com.example.one', 1)
    await expect(store.commit(staged.token, true)).rejects.toThrow()
    const [l] = await store.list()
    expect(l).toMatchObject({ kind: 'unreadable', id: 'com.example.one' })
    expect(l.kind === 'unreadable' && l.reason).toMatch(/remove it, or install the plugin again/)
    const again = await install(store)
    expect(again.version).toBe('1.0.0')
    expect((await store.list())[0]).toMatchObject({ kind: 'installed' })
    expect(treeOf(root).some((p) => p.includes('previous/'))).toBe(false)
  })

  it('a failed update puts the old version back', async () => {
    const { store } = setup()
    await install(store, { version: '1.0.0' })
    const staged = await store.stageFile(pluginFile({ version: '2.0.0' }))
    failRecordWrites('com.example.one', 1)
    await expect(store.commit(staged.token, true)).rejects.toThrow()
    const [l] = await store.list()
    expect(l).toMatchObject({ kind: 'installed', installed: { version: '1.0.0' } })
  })

  it('a failed rollback puts the current version back', async () => {
    const { store } = setup()
    await install(store, { version: '1.0.0' })
    await install(store, { version: '2.0.0' })
    failRecordWrites('com.example.one', 1)
    await expect(store.rollback('com.example.one')).rejects.toThrow()
    const [l] = await store.list()
    expect(l).toMatchObject({ kind: 'installed', installed: { version: '2.0.0' } })
  })

  it('a record from a newer Hew is still refused, and a folder with a record but no manifest is not "unfinished"', async () => {
    const { root, store } = setup()
    await install(store)
    const folder = root.dirs.get('plugins')!.dirs.get('com.example.one')!
    folder.files.delete('hew-plugin.json')
    expect(await refusal(store.stageFile(pluginFile({ version: '2.0.0' })))).toBe('installed_unreadable')
  })
})

describe('a changed plugin while an install waits', () => {
  it('refuses a commit when the installed record changed since the sheet', async () => {
    const { root, store } = setup()
    await install(store, { version: '1.0.0' })
    const staged = await store.stageFile(pluginFile({ version: '2.0.0' }))
    await store.setEnabled('com.example.one', false)
    const before = treeOf(root)
    await expect(store.commit(staged.token, true)).rejects.toMatchObject({
      refusal: 'pending_stale',
      message: 'This plugin changed while the install was waiting. Start the install again.',
    })
    expect(treeOf(root)).toEqual(before)
  })

  it('refuses when it was not installed at the sheet and is now, and the reverse', async () => {
    const a = setup()
    const first = await a.store.stageFile(pluginFile())
    // Another tab installs it meanwhile.
    await install(setup({ root: a.root }).store, { version: '0.5.0' })
    expect(await refusal(a.store.commit(first.token, true))).toBe('pending_stale')

    const b = setup()
    await install(b.store)
    const update = await b.store.stageFile(pluginFile({ version: '2.0.0' }))
    await b.store.uninstall('com.example.one')
    expect(await refusal(b.store.commit(update.token, true))).toBe('pending_stale')
    expect(await b.store.list()).toEqual([])
  })

  it('a newer stage of the same plugin replaces the older pending install', async () => {
    const { store } = setup()
    const older = await store.stageFile(pluginFile({ version: '1.0.0' }))
    const newer = await store.stageFile(pluginFile({ version: '1.1.0' }))
    expect(await refusal(store.commit(older.token, true))).toBe('pending_unknown')
    expect((await store.commit(newer.token, true))?.version).toBe('1.1.0')
    // Another plugin's pending is left alone.
    const x = await store.stageFile(pluginFile({ id: 'com.example.x' }))
    await store.stageFile(pluginFile({ id: 'com.example.y' }))
    expect((await store.commit(x.token, true))?.id).toBe('com.example.x')
  })
})
