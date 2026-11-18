/**
 * The web plugin store on the in-memory file system: the layout it writes,
 * the record rules it mirrors from the desktop store, and what it refuses.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { FakeDir, linkedChannels } from '../test/opfsFake'
import { compareCodePoints } from './pluginText'
import { storedZipOf as storedZip } from './storedZip'
import { fakeWasm, indexStoredZip, manifestJson, pluginFile, textAt, treeOf, type ListedFile } from './testKit'
import {
  docStorageDelete,
  docStorageGet,
  docStorageList,
  docStorageSet,
  folderNameOk,
  makeWebPluginStore,
  packFolder,
  recordFingerprint,
  rollbackFingerprint,
  storageDelete,
  storageGet,
  storageList,
  storageSet,
  type LockProvider,
  type PluginWasm,
  type WebPluginStore,
  validDocumentId,
} from './webPluginStore'
import { PluginError } from './pluginError'
import type { Installed } from './types'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

/** An in-memory Web Lock: one holder at a time, in request order. */
function memoryLocks(): LockProvider & { held: boolean; log: string[] } {
  let tail: Promise<unknown> = Promise.resolve()
  const locks = {
    held: false,
    log: [] as string[],
    request<T>(name: string, callback: () => Promise<T>): Promise<T> {
      const run = tail.then(async () => {
        locks.held = true
        locks.log.push(`take ${name}`)
        try {
          return await callback()
        } finally {
          locks.log.push(`release ${name}`)
          locks.held = false
        }
      })
      tail = run.catch(() => undefined)
      return run
    },
  }
  return locks
}

function setup(
  over: {
    now?: () => number
    channel?: BroadcastChannel | null
    wasm?: PluginWasm
    root?: FakeDir
    locks?: LockProvider | null
  } = {},
) {
  const locks = over.locks === undefined ? memoryLocks() : over.locks
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
    locks,
  })
  return { root, keys, store, locks }
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

describe('command names', () => {
  const commands = (...ids: string[]) => ({ contributes: { commands: ids.map((id) => ({ id, class: 'read_only', summary: 's' })) } })

  it('refuses at stage a command whose namespaced name another installed plugin has, dots in ids counted', async () => {
    const { store } = setup()
    await install(store, { id: 'com.acme', ...commands('tool.run') })
    expect(await refusal(store.stageFile(pluginFile({ id: 'com.acme.tool', ...commands('run') })))).toBe('command_name_taken')
    await expect(store.stageFile(pluginFile({ id: 'com.acme.tool', ...commands('go') }))).resolves.toBeTruthy()
    // The plugin's own earlier version is not another plugin.
    await expect(store.stageFile(pluginFile({ id: 'com.acme', version: '2.0.0', ...commands('tool.run') }))).resolves.toBeTruthy()
  })

  it('says which name, cleaned, and refuses again at commit when another plugin took it while the sheet was open', async () => {
    const { store } = setup()
    const staged = await store.stageFile(pluginFile({ id: 'com.acme.tool', ...commands('run') }))
    await install(store, { id: 'com.acme', ...commands('tool.run') })
    const failure = await store.commit(staged.token, true).then(
      () => null,
      (e: PluginError) => e,
    )
    expect(failure?.refusal).toBe('command_name_taken')
    expect(failure?.message).toBe(
      'Another installed plugin already has a command named "com.acme.tool.run". Rename this plugin\'s command.',
    )
    expect(await refusal(store.uninstall('com.acme.tool'))).toBe('not_installed')
  })
})

describe('a rollback follows the install rules', () => {
  const commands = (...ids: string[]) => ({ contributes: { commands: ids.map((id) => ({ id, class: 'read_only', summary: 's' })) } })

  it('refuses a version whose command name another plugin took since, moving nothing', async () => {
    const { root, store } = setup()
    await install(store, { id: 'com.acme', ...commands('tool.run') })
    await install(store, { id: 'com.acme', version: '2.0.0' })
    await install(store, { id: 'com.acme.tool', ...commands('run') })
    const before = treeOf(root)
    expect(await refusal(store.rollback('com.acme'))).toBe('command_name_taken')
    expect(treeOf(root)).toEqual(before)
  })

  it('refuses a version whose requirement is gone, moving nothing', async () => {
    const { root, store } = setup()
    await install(store, { id: 'com.example.dep' })
    await install(store, { id: 'com.acme', requires: [{ id: 'com.example.dep', range: '^1' }] })
    await install(store, { id: 'com.acme', version: '2.0.0' })
    await store.uninstall('com.example.dep')
    const before = treeOf(root)
    expect(await refusal(store.rollback('com.acme'))).toBe('dependency_missing')
    expect(treeOf(root)).toEqual(before)
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

describe('the plugin lock', () => {
  it('refuses every change, with a typed error, when the browser has no Web Locks, and still lists', async () => {
    const { root, store } = setup({ locks: null })
    expect(await store.list()).toEqual([])
    const id = 'com.example.one'
    expect(await refusal(store.stageFile(pluginFile()))).toBe('locks_unavailable')
    expect(await refusal(store.commit('x', true))).toBe('locks_unavailable')
    expect(await refusal(store.setEnabled(id, true))).toBe('locks_unavailable')
    expect(await refusal(store.uninstall(id))).toBe('locks_unavailable')
    expect(await refusal(store.rollback(id))).toBe('locks_unavailable')
    expect(await refusal(store.storage.storageSet(id, 'k', 'v'))).toBe('locks_unavailable')
    expect(await refusal(store.storage.storageDelete(id, 'k'))).toBe('locks_unavailable')
    expect(treeOf(root)).toEqual([])
  })

  it('writes only while holding the lock, and two overlapping changes never interleave', async () => {
    const locks = memoryLocks()
    const root = new FakeDir('opfs-root')
    const make = () =>
      makeWebPluginStore({
        wasm: async () => fakeWasm(),
        opfsRoot: async () => root as unknown as FileSystemDirectoryHandle,
        channel: null,
        storage: null,
        locks,
        hewVersion: () => '1.5.0',
      })
    // Two tabs: two stores over one file system and one lock.
    const tabA = make()
    const tabB = make()
    const writes: boolean[] = []
    const realRemove = FakeDir.prototype.removeEntry
    const realFile = FakeDir.prototype.getFileHandle
    const spies = [
      vi.spyOn(FakeDir.prototype, 'removeEntry').mockImplementation(async function (this: FakeDir, ...a) {
        writes.push(locks.held)
        await new Promise((r) => setTimeout(r, 0))
        return realRemove.apply(this, a)
      }),
      vi.spyOn(FakeDir.prototype, 'getFileHandle').mockImplementation(async function (this: FakeDir, name, opts) {
        if (opts?.create) {
          writes.push(locks.held)
          await new Promise((r) => setTimeout(r, 0))
        }
        return realFile.call(this, name, opts)
      }),
    ]
    try {
      const sa = await tabA.stageFile(pluginFile())
      const sb = await tabB.stageFile(pluginFile({ id: 'com.example.two' }))
      locks.log.length = 0
      writes.length = 0
      await Promise.all([tabA.commit(sa.token, true), tabB.commit(sb.token, true), tabA.setEnabled('com.example.one', false)])
    } finally {
      for (const s of spies) s.mockRestore()
    }
    expect(writes.length).toBeGreaterThan(10)
    expect(writes.every(Boolean)).toBe(true)
    // take, release, take, release...: never two takes in a row.
    expect(locks.log).toEqual(['take hew-plugins', 'release hew-plugins', 'take hew-plugins', 'release hew-plugins', 'take hew-plugins', 'release hew-plugins'])
  })
})

/** Fails one mutating file system step, or every step from there on. */
function injector() {
  let count = 0
  let failAt = Infinity
  let sticky = false
  const tick = (): void => {
    count++
    if (count === failAt || (sticky && count > failAt)) throw new Error('InjectedFailure: the disk said no')
  }
  const realRemove = FakeDir.prototype.removeEntry
  const realDir = FakeDir.prototype.getDirectoryHandle
  const realFile = FakeDir.prototype.getFileHandle
  const spies = [
    vi.spyOn(FakeDir.prototype, 'removeEntry').mockImplementation(async function (this: FakeDir, ...a) {
      tick()
      return realRemove.apply(this, a)
    }),
    vi.spyOn(FakeDir.prototype, 'getDirectoryHandle').mockImplementation(async function (this: FakeDir, name, opts) {
      if (opts?.create && !this.dirs.has(name)) tick()
      return realDir.call(this, name, opts)
    }),
    vi.spyOn(FakeDir.prototype, 'getFileHandle').mockImplementation(async function (this: FakeDir, name, opts) {
      if (opts?.create) tick()
      const handle = await realFile.call(this, name, opts)
      if (!opts?.create) return handle
      const real = handle.createWritable.bind(handle)
      handle.createWritable = async () => {
        const w = await real()
        const close = w.close.bind(w)
        w.close = async () => {
          tick()
          return close()
        }
        return w
      }
      return handle
    }),
  ]
  return {
    arm(at: number, stick = false) {
      count = 0
      failAt = at
      sticky = stick
    },
    disarm() {
      failAt = Infinity
      sticky = false
    },
    get count() {
      return count
    },
    restore() {
      for (const s of spies) s.mockRestore()
    },
  }
}

describe('swapping versions survives a failure at every step', () => {
  const ID = 'com.example.one'

  /** 1.0.0 installed, then 2.0.0 over it; saved data written between. */
  async function twoVersions() {
    const env = setup()
    await install(env.store, { version: '1.0.0' })
    await env.store.storage.storageSet(ID, 'k', 'one')
    await install(env.store, { version: '2.0.0' })
    await env.store.storage.storageSet(ID, 'k2', 'two')
    return env
  }

  async function versionsOf(env: ReturnType<typeof setup>): Promise<{ current: string; previous: string | null }> {
    const listed = await env.store.list()
    expect(listed).toHaveLength(1)
    const [l] = listed
    if (l.kind !== 'installed') throw new Error(`unreadable: ${JSON.stringify(l)}`)
    return { current: l.installed.version, previous: l.previous_version }
  }

  async function bothIntact(env: ReturnType<typeof setup>, arrangement: { current: string; previous: string }): Promise<void> {
    expect(await versionsOf(env)).toEqual(arrangement)
    const prev = await env.store.previous(ID)
    expect(prev.installed.version).toBe(arrangement.previous)
    expect(await textAt(env.root, `plugins/${ID}/hew-plugin.json`)).toContain(`"${arrangement.current}"`)
    expect(await textAt(env.root, `plugins/${ID}/previous/hew-plugin.json`)).toContain(`"${arrangement.previous}"`)
    expect(await textAt(env.root, `plugins/${ID}/files/dist/background.js`)).not.toBeNull()
    expect(await textAt(env.root, `plugins/${ID}/previous/files/dist/background.js`)).not.toBeNull()
    // The saved data is the plugin's, never lost.
    expect(await env.store.storage.storageList(ID)).toEqual(['k', 'k2'])
  }

  async function sweep(
    op: (env: ReturnType<typeof setup>, prepared: unknown) => Promise<unknown>,
    prepare: (env: ReturnType<typeof setup>) => Promise<unknown>,
    before: { current: string; previous: string },
    after: { current: string; previous: string },
  ): Promise<void> {
    // How many steps does a clean run take?
    const probe = injector()
    let total: number
    try {
      const env = await twoVersions()
      const prepared = await prepare(env)
      probe.arm(Infinity)
      await op(env, prepared)
      total = probe.count
    } finally {
      probe.restore()
    }
    expect(total).toBeGreaterThan(8)
    let rejected = 0
    for (const sticky of [false, true]) {
      for (let at = 1; at <= total; at++) {
        const inj = injector()
        try {
          const env = await twoVersions()
          const prepared = await prepare(env)
          inj.arm(at, sticky)
          let failed = false
          try {
            await op(env, prepared)
          } catch {
            failed = true
          }
          // The disk is healthy again (a reload, after a sticky failure).
          inj.disarm()
          const label = `${sticky ? 'every step from' : 'only'} step ${at}`
          if (failed) {
            rejected++
            await bothIntact(env, before).catch((e) => {
              throw new Error(`${label}: ${e instanceof Error ? e.message : e}`)
            })
          } else {
            await bothIntact(env, after).catch((e) => {
              throw new Error(`${label} (succeeded): ${e instanceof Error ? e.message : e}`)
            })
          }
        } finally {
          inj.restore()
        }
      }
    }
    // Most steps are before the swap completes: they must reject.
    expect(rejected).toBeGreaterThan(total)
  }

  it('rollback leaves the installed and the previous version whole at any single failure and at a crash', async () => {
    await sweep(
      (env) => env.store.rollback(ID),
      async () => undefined,
      { current: '2.0.0', previous: '1.0.0' },
      { current: '1.0.0', previous: '2.0.0' },
    )
  })

  it('an update leaves the installed and the previous version whole at any single failure and at a crash', async () => {
    // Before: 2.0.0 over 1.0.0. After a good update: 3.0.0 over 2.0.0.
    await sweep(
      async (env, staged) => env.store.commit((staged as { token: string }).token, true),
      (env) => env.store.stageFile(pluginFile({ version: '3.0.0' })),
      { current: '2.0.0', previous: '1.0.0' },
      { current: '3.0.0', previous: '2.0.0' },
    )
  })

  /** A `.old` copy a finished swap could not remove: whole, marked, and now a
   *  leftover beside a healthy folder. */
  async function leftover(env: ReturnType<typeof setup>): Promise<void> {
    const plugins = env.root.dirs.get('plugins') as FakeDir
    const folder = plugins.dirs.get(ID) as FakeDir
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    const old = await staging.getDirectoryHandle(`${ID}-${'d'.repeat(32)}.old`, { create: true })
    for (const [n, f] of folder.files) old.files.set(n, f)
    for (const [n, d] of folder.dirs) old.dirs.set(n, d)
    old.files.set('.swap-intent', { bytes: new Uint8Array([49, 10]), mtime: 0 })
  }

  it('an uninstall cut short at any step leaves the plugin installed or gone, and never brings it back from a `.old` copy', async () => {
    const probe = injector()
    let total: number
    try {
      const env = await twoVersions()
      await leftover(env)
      probe.arm(Infinity)
      await env.store.uninstall(ID)
      total = probe.count
    } finally {
      probe.restore()
    }
    expect(total).toBeGreaterThan(1)
    let gone = 0
    let kept = 0
    for (const sticky of [false, true]) {
      // One past the last step: nothing fails, the uninstall completes.
      for (let at = 1; at <= total + 1; at++) {
        const inj = injector()
        const label = `${sticky ? 'every step from' : 'only'} step ${at}`
        try {
          const env = await twoVersions()
          await leftover(env)
          inj.arm(at, sticky)
          let failed = false
          try {
            await env.store.uninstall(ID)
          } catch {
            failed = true
          }
          inj.disarm()
          // Whether the plugin's folder was already removed when it stopped:
          // from then on it is uninstalled, whatever else is left behind.
          const removed = (await textAt(env.root, `plugins/${ID}/installed.json`)) === null
          // Two later calls: the first may recover, the second must agree.
          for (let again = 0; again < 2; again++) {
            const listed = await env.store.list()
            expect(listed.length === 0 || !removed, `${label}: the removed plugin came back`).toBe(true)
            if (listed.length === 0) {
              expect(await textAt(env.root, `plugins/${ID}/installed.json`), label).toBeNull()
              if (again === 0) gone++
            } else {
              // Never resurrected: still installed means the uninstall did not get past its first step.
              expect(failed, `${label}: the plugin is back after a completed uninstall`).toBe(true)
              await bothIntact(env, { current: '2.0.0', previous: '1.0.0' }).catch((e) => {
                throw new Error(`${label}: ${e instanceof Error ? e.message : e}`)
              })
              if (again === 0) kept++
            }
          }
          if (!failed) expect(await env.store.list(), label).toEqual([])
        } finally {
          inj.restore()
        }
      }
    }
    expect(gone).toBeGreaterThan(0)
    expect(kept).toBeGreaterThan(0)
  })

  it('a rollback carries the current saved data to the version coming back', async () => {
    const env = await twoVersions()
    await env.store.rollback(ID)
    expect(await env.store.storage.storageGet(ID, 'k2')).toBe('two')
    expect(await textAt(env.root, `plugins/${ID}/previous/storage.json`)).toBe('{"k":"one","k2":"two"}')
    expect(treeOf(env.root).some((p) => p.includes('.staging/'))).toBe(false)
  })

  it('restores a swap a crash cut short, on the next call', async () => {
    const env = await twoVersions()
    // The crash state: the folder emptied half way, the `.old` copy whole.
    const plugins = env.root.dirs.get('plugins') as FakeDir
    const folder = plugins.dirs.get(ID) as FakeDir
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    const old = await staging.getDirectoryHandle(`${ID}-${'a'.repeat(32)}.old`, { create: true })
    for (const [n, f] of folder.files) old.files.set(n, f)
    for (const [n, d] of folder.dirs) old.dirs.set(n, d)
    // The swap wrote its intent once the copy was whole.
    old.files.set('.swap-intent', { bytes: new Uint8Array([49, 10]), mtime: 0 })
    plugins.dirs.set(ID, new FakeDir(ID))
    plugins.dirs.get(ID)!.files.set('hew-plugin.json', folder.files.get('hew-plugin.json')!)
    await bothIntact(env, { current: '2.0.0', previous: '1.0.0' })
    expect(treeOf(env.root).some((p) => p.includes('.old'))).toBe(false)
    // The marker is the swap's, not the plugin's: it does not come back.
    expect(treeOf(env.root).some((p) => p.includes('.swap-intent'))).toBe(false)
  })

  /** The crash state: the installed folder emptied of its record, its copy
   *  whole in `.staging`, with or without the swap's intent marker. */
  async function crashed(env: ReturnType<typeof setup>, marked: boolean): Promise<string> {
    const plugins = env.root.dirs.get('plugins') as FakeDir
    const folder = plugins.dirs.get(ID) as FakeDir
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    const name = `${ID}-${'a'.repeat(32)}.old`
    const old = await staging.getDirectoryHandle(name, { create: true })
    for (const [n, f] of folder.files) old.files.set(n, f)
    for (const [n, d] of folder.dirs) old.dirs.set(n, d)
    if (marked) old.files.set('.swap-intent', { bytes: new Uint8Array([49, 10]), mtime: 0 })
    plugins.dirs.set(ID, new FakeDir(ID))
    plugins.dirs.get(ID)!.files.set('hew-plugin.json', folder.files.get('hew-plugin.json')!)
    return name
  }

  it('does not restore from a `.old` copy that carries no intent marker', async () => {
    const env = await twoVersions()
    const name = await crashed(env, false)
    const listed = await env.store.list()
    expect(listed).toMatchObject([{ kind: 'unreadable', id: ID }])
    // Nothing deletes it either.
    expect(treeOf(env.root).some((p) => p.includes(name))).toBe(true)
  })

  it('staging a package after a failed recovery keeps the `.old` copy, so the next call still restores it', async () => {
    // Total steps a clean stage takes after a crash, then every failure point.
    const probe = injector()
    let total: number
    try {
      const env = await twoVersions()
      await crashed(env, true)
      probe.arm(Infinity)
      await env.store.stageFile(pluginFile({ version: '3.0.0' }))
      total = probe.count
    } finally {
      probe.restore()
    }
    expect(total).toBeGreaterThan(0)
    for (const sticky of [false, true]) {
      for (let at = 1; at <= total; at++) {
        const inj = injector()
        try {
          const env = await twoVersions()
          await crashed(env, true)
          inj.arm(at, sticky)
          // A failing disk fails the recovery inside the call, or the call.
          let staged: { token: string } | null = null
          try {
            staged = await env.store.stageFile(pluginFile({ version: '3.0.0' }))
          } catch {
            staged = null
          }
          // The person cancels, or the call failed; either way nothing is accepted.
          if (staged !== null) await env.store.commit(staged.token, false)
          inj.disarm()
          await bothIntact(env, { current: '2.0.0', previous: '1.0.0' }).catch((e) => {
            throw new Error(`${sticky ? 'every step from' : 'only'} step ${at}: ${e instanceof Error ? e.message : e}`)
          })
          // A copy left because its removal failed after the restore is a
          // leftover, harmless beside a folder with a record: the next swap
          // or uninstall of the id removes it.
        } finally {
          inj.restore()
        }
      }
    }
  })

  it('an update removes the `.old` leftovers of its id once the folder is healthy, and no other plugin\'s', async () => {
    const env = await twoVersions()
    const plugins = env.root.dirs.get('plugins') as FakeDir
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    const mine = await staging.getDirectoryHandle(`${ID}-${'b'.repeat(32)}.old`, { create: true })
    mine.files.set('installed.json', { bytes: new Uint8Array([123, 125]), mtime: 0 })
    const other = await staging.getDirectoryHandle(`com.example.two-${'c'.repeat(32)}.old`, { create: true })
    other.files.set('installed.json', { bytes: new Uint8Array([123, 125]), mtime: 0 })
    await install(env.store, { version: '3.0.0' })
    const left = treeOf(env.root).filter((p) => p.includes('.old'))
    expect(left.some((p) => p.includes(`${ID}-`))).toBe(false)
    expect(left.some((p) => p.includes('com.example.two-'))).toBe(true)
  })
})

describe('plugin storage', () => {
  const ID = 'com.example.one'

  it('keeps strings by key in plugins/<id>/storage.json, lists them sorted, and says what changed', async () => {
    const posts: unknown[] = []
    const channel = { addEventListener: () => {}, postMessage: (m: unknown) => void posts.push(m) } as unknown as BroadcastChannel
    const { root, store } = setup({ channel })
    await install(store)
    const s = store.storage
    expect(await s.storageGet(ID, 'a')).toBeNull()
    expect(await s.storageList(ID)).toEqual([])
    await s.storageSet(ID, 'b', 'two')
    await s.storageSet(ID, 'a', '1')
    await s.storageSet(ID, '__proto__', 'x')
    expect(await s.storageGet(ID, 'b')).toBe('two')
    expect(await s.storageGet(ID, '__proto__')).toBe('x')
    expect(await s.storageGet(ID, 'toString')).toBeNull()
    expect(await s.storageList(ID)).toEqual(['__proto__', 'a', 'b'])
    expect(JSON.parse((await textAt(root, `plugins/${ID}/storage.json`)) as string)).toEqual({ a: '1', b: 'two', ['__proto__']: 'x' })
    expect(posts.filter((m) => (m as { kind: string }).kind === 'storage')).toEqual([
      { kind: 'storage', id: ID, keys: ['b'] },
      { kind: 'storage', id: ID, keys: ['a'] },
      { kind: 'storage', id: ID, keys: ['__proto__'] },
    ])
    posts.length = 0
    await s.storageDelete(ID, 'nothing')
    expect(posts).toEqual([])
    await s.storageDelete(ID, 'a')
    expect(posts).toEqual([{ kind: 'storage', id: ID, keys: ['a'] }])
    await s.storageDelete(ID, 'b')
    await s.storageDelete(ID, '__proto__')
    // The file goes with the last key.
    expect(treeOf(root).some((p) => p.endsWith('storage.json'))).toBe(false)
  })

  it('does not wake another tab\'s pane for a storage notice', async () => {
    const [tabA, tabB] = linkedChannels()
    const a = setup({ channel: tabA })
    let other = 0
    const b = makeWebPluginStore({
      wasm: async () => fakeWasm(),
      opfsRoot: async () => a.root as unknown as FileSystemDirectoryHandle,
      channel: tabB,
      storage: null,
    })
    b.onChanged(() => other++)
    await install(a.store)
    expect(other).toBe(1)
    // The linked fake delivers without data; a real storage message carries it.
    const seen: Array<(ev: unknown) => void> = []
    const fake = { addEventListener: (_t: string, fn: (ev: unknown) => void) => seen.push(fn), postMessage: () => {} } as unknown as BroadcastChannel
    const c = makeWebPluginStore({ wasm: async () => fakeWasm(), opfsRoot: async () => a.root as unknown as FileSystemDirectoryHandle, channel: fake, storage: null })
    let hits = 0
    c.onChanged(() => hits++)
    seen.forEach((f) => f({ data: { kind: 'storage', id: ID, keys: ['k'] } }))
    expect(hits).toBe(0)
    seen.forEach((f) => f({ data: { kind: 'changed' } }))
    expect(hits).toBe(1)
  })

  it('refuses a plugin that is not installed, an id that is not an id, and never makes a folder', async () => {
    const { root, store } = setup()
    const s = store.storage
    for (const call of [
      () => s.storageGet(ID, 'k'),
      () => s.storageSet(ID, 'k', 'v'),
      () => s.storageDelete(ID, 'k'),
      () => s.storageList(ID),
    ]) {
      expect(await refusal(call())).toBe('not_installed')
    }
    expect(treeOf(root)).toEqual([])
    expect(root.dirs.size).toBe(0)
    for (const bad of ['..', 'a/../b.c', 'x', 'hew.x']) {
      expect(await refusal(s.storageSet(bad, 'k', 'v')), bad).toBe('invalid_id')
      expect(await refusal(s.storageGet(bad, 'k')), bad).toBe('invalid_id')
    }
    // A folder with no record is not an installed plugin.
    const plugins = await root.getDirectoryHandle('plugins', { create: true })
    await plugins.getDirectoryHandle(ID, { create: true })
    expect(await refusal(s.storageSet(ID, 'k', 'v'))).toBe('not_installed')
    expect(treeOf(root)).toEqual([])
  })

  it('refuses an empty key and one over 256 bytes (counted in UTF-8), and reports with the store convention', async () => {
    const { store } = setup()
    await install(store)
    const s = store.storage
    expect(await refusal(s.storageSet(ID, '', 'v'))).toBe('storage_key_invalid')
    expect(await refusal(s.storageGet(ID, ''))).toBe('storage_key_invalid')
    await s.storageSet(ID, 'x'.repeat(256), 'v')
    expect(await refusal(s.storageSet(ID, 'x'.repeat(257), 'v'))).toBe('storage_key_invalid')
    // 129 two-byte characters are 258 bytes.
    await expect(s.storageSet(ID, 'é'.repeat(129), 'v')).rejects.toMatchObject({
      refusal: 'storage_key_invalid',
      message: 'A storage key must be 1 to 256 bytes long.',
    })
    await s.storageSet(ID, 'é'.repeat(128), 'v')
  })

  it('refuses past 5 MiB as compact JSON and writes nothing', async () => {
    const { root, store } = setup()
    await install(store)
    const s = store.storage
    const MiB = 1024 * 1024
    await s.storageSet(ID, 'big', 'x'.repeat(4 * MiB))
    const before = await textAt(root, `plugins/${ID}/storage.json`)
    await expect(s.storageSet(ID, 'more', 'y'.repeat(2 * MiB))).rejects.toMatchObject({ refusal: 'storage_quota' })
    expect(await textAt(root, `plugins/${ID}/storage.json`)).toBe(before)
    // A single value too large on its own.
    expect(await refusal(s.storageSet(ID, 'huge', 'z'.repeat(5 * MiB + 1)))).toBe('storage_quota')
    // Replacing a value with a smaller one is fine; the file only has to fit.
    await s.storageSet(ID, 'big', 'x')
    expect(await s.storageGet(ID, 'big')).toBe('x')
    // The overhead of the JSON counts: exactly 5 MiB of value plus the braces and key does not fit.
    expect(await refusal(s.storageSet(ID, 'k', 'v'.repeat(5 * MiB - 8)))).toBe('storage_quota')
    await s.storageSet(ID, 'k', 'v'.repeat(5 * MiB - 100))
  })

  it('refuses a file that is not an object of strings and never rewrites it', async () => {
    const { root, store } = setup()
    await install(store)
    const folder = root.dirs.get('plugins')!.dirs.get(ID)!
    for (const bad of ['not json', '[1]', '"s"', 'null', '{"a":1}', '{"a":{"b":"c"}}', '{"a":"b"']) {
      const bytes = new TextEncoder().encode(bad)
      folder.files.set('storage.json', { bytes, mtime: 1 })
      for (const call of [
        () => store.storage.storageGet(ID, 'a'),
        () => store.storage.storageList(ID),
        () => store.storage.storageSet(ID, 'a', 'v'),
        () => store.storage.storageDelete(ID, 'a'),
      ]) {
        expect(await refusal(call()), bad).toBe('storage_unreadable')
      }
      expect(folder.files.get('storage.json')!.bytes, bad).toEqual(bytes)
    }
    // A file over the cap is unreadable too.
    folder.files.set('storage.json', { bytes: new Uint8Array(5 * 1024 * 1024 + 1).fill(32), mtime: 1 })
    expect(await refusal(store.storage.storageList(ID))).toBe('storage_unreadable')
  })

  it('is carried across an update and a rollback, and removed on uninstall', async () => {
    const { root, store } = setup()
    await install(store, { version: '1.0.0' })
    await store.storage.storageSet(ID, 'k', 'v')
    await install(store, { version: '2.0.0' })
    expect(await store.storage.storageGet(ID, 'k')).toBe('v')
    expect(await textAt(root, `plugins/${ID}/previous/storage.json`)).toBe('{"k":"v"}')
    await store.storage.storageSet(ID, 'k', 'v2')
    await store.rollback(ID)
    expect(await store.storage.storageGet(ID, 'k')).toBe('v2')
    await store.rollback(ID)
    expect(await store.storage.storageGet(ID, 'k')).toBe('v2')
    await store.uninstall(ID)
    expect(treeOf(root).some((p) => p.includes('storage.json'))).toBe(false)
    // Installed again, it starts empty.
    await install(store)
    expect(await store.storage.storageList(ID)).toEqual([])
  })

  it('writes under the lock', async () => {
    const locks = memoryLocks()
    const { store } = setup({ locks })
    await install(store)
    locks.log.length = 0
    await store.storage.storageSet(ID, 'k', 'v')
    expect(locks.log).toEqual(['take hew-plugins', 'release hew-plugins'])
  })

  it('is exported as module functions over navigator.storage and navigator.locks', async () => {
    const root = new FakeDir('opfs-root')
    const locks = memoryLocks()
    const storageDesc = Object.getOwnPropertyDescriptor(navigator, 'storage')
    const locksDesc = Object.getOwnPropertyDescriptor(navigator, 'locks')
    Object.defineProperty(navigator, 'storage', { configurable: true, value: { getDirectory: async () => root } })
    Object.defineProperty(navigator, 'locks', { configurable: true, value: locks })
    try {
      // Install through a store over the same file system.
      const { store } = setup({ root })
      await install(store)
      await storageSet(ID, 'b', '2')
      await storageSet(ID, 'a', '1')
      expect(await storageGet(ID, 'a')).toBe('1')
      expect(await storageList(ID)).toEqual(['a', 'b'])
      await storageDelete(ID, 'a')
      expect(await storageList(ID)).toEqual(['b'])
      expect(await refusal(storageGet('com.example.none', 'k'))).toBe('not_installed')
    } finally {
      for (const [k, d] of [['storage', storageDesc], ['locks', locksDesc]] as const) {
        if (d) Object.defineProperty(navigator, k, d)
        else delete (navigator as unknown as Record<string, unknown>)[k]
      }
    }
  })
})

describe('what the person was shown is what is applied', () => {
  const ID = 'com.example.one'

  it('turning on refuses plugin_changed when the record is not the one shown, and applies the one that is', async () => {
    const { store } = setup()
    await install(store, { permissions: ['model.read'] })
    await store.setEnabled(ID, false)
    const shown = recordFingerprint((await listedOne(store)).installed)
    // Another tab updates the plugin to a version that asks for more.
    await install(store, { version: '2.0.0', permissions: ['model.read', 'network:api.example.com'] })
    expect(await refusal(store.setEnabled(ID, true, shown))).toBe('plugin_changed')
    expect((await listedOne(store)).installed.enabled).toBe(false)
    // Shown the current record, it turns on.
    await store.setEnabled(ID, true, recordFingerprint((await listedOne(store)).installed))
    expect((await listedOne(store)).installed.enabled).toBe(true)
    // Turning off asks nothing of what was shown.
    await store.setEnabled(ID, false, 'anything')
    expect((await listedOne(store)).installed.enabled).toBe(false)
  })

  it('a reinstall of the same version is a change to what was shown', async () => {
    const { store } = setup({ now: (() => { let t = 1000; return () => (t += 10) })() })
    await install(store)
    await store.setEnabled(ID, false)
    const shown = recordFingerprint((await listedOne(store)).installed)
    await install(store)
    expect(await refusal(store.setEnabled(ID, true, shown))).toBe('plugin_changed')
  })

  it('rolling back refuses plugin_changed when previous/ no longer holds the version shown', async () => {
    const { store } = setup()
    await install(store, { version: '1.0.0' })
    await install(store, { version: '2.0.0' })
    const before = await store.previous(ID)
    const shown = rollbackFingerprint((await listedOne(store)).installed, before.installed)
    // Another tab installs 3.0.0: 2.0.0 becomes the previous version.
    await install(store, { version: '3.0.0', permissions: ['network:api.example.com'] })
    expect(await refusal(store.rollback(ID, shown))).toBe('plugin_changed')
    const now = await listedOne(store)
    expect(now.installed.version).toBe('3.0.0')
    expect(now.previous_version).toBe('2.0.0')
    // Shown the pair that is there, it rolls back.
    const prev = await store.previous(ID)
    const back = await store.rollback(ID, rollbackFingerprint(now.installed, prev.installed))
    expect(back.version).toBe('2.0.0')
  })

  async function listedOne(store: WebPluginStore) {
    const [l] = await store.list()
    if (l.kind !== 'installed') throw new Error('expected an installed plugin')
    return l
  }
})

describe('saved keys sort by code point, as the desktop store sorts them', () => {
  it('puts U+FF21 before a character outside the Basic Multilingual Plane, and lists the same order', async () => {
    const { store } = setup()
    await install(store)
    const wide = '\uFF21'
    const emoji = '\u{1F600}'
    expect([emoji, wide].sort()).toEqual([emoji, wide])
    expect(compareCodePoints(wide, emoji)).toBeLessThan(0)
    expect(compareCodePoints(emoji, wide)).toBeGreaterThan(0)
    expect(compareCodePoints('a', 'a')).toBe(0)
    expect(compareCodePoints('a', 'ab')).toBeLessThan(0)
    await store.storage.storageSet('com.example.one', emoji, '1')
    await store.storage.storageSet('com.example.one', wide, '2')
    await store.storage.storageSet('com.example.one', 'a', '3')
    expect(await store.storage.storageList('com.example.one')).toEqual(['a', wide, emoji])
  })
})


// ----------------------------------------------------- a folder, copied in

/** A picked folder: paths to contents, as the browser's handles. */
function pickedFolder(name: string, entries: Record<string, string | Uint8Array>): FakeDir {
  const root = new FakeDir(name)
  for (const [path, content] of Object.entries(entries)) {
    const parts = path.split('/')
    let dir = root
    for (const p of parts.slice(0, -1)) {
      let next = dir.dirs.get(p)
      if (next === undefined) {
        next = new FakeDir(p)
        dir.dirs.set(p, next)
      }
      dir = next
    }
    dir.files.set(parts[parts.length - 1], { bytes: typeof content === 'string' ? new TextEncoder().encode(content) : content, mtime: 0 })
  }
  return root
}

/** A folder whose files report `sizes` and count every byte read. */
function sizedFolder(sizes: Record<string, number>): { dir: FileSystemDirectoryHandle; reads: string[] } {
  const reads: string[] = []
  const handles = Object.entries(sizes).map(([name, size]) => [
    name,
    {
      kind: 'file',
      name,
      getFile: async () => ({
        size,
        arrayBuffer: async () => {
          reads.push(name)
          return new ArrayBuffer(size)
        },
      }),
    },
  ])
  const dir = {
    kind: 'directory',
    name: 'big',
    async *entries() {
      for (const h of handles) yield h
    },
  }
  return { dir: dir as unknown as FileSystemDirectoryHandle, reads }
}

const asHandle = (d: FakeDir): FileSystemDirectoryHandle => d as unknown as FileSystemDirectoryHandle

describe('installing a copy of a folder (the web)', () => {
  const plugin = (over = {}, extra: Record<string, string> = {}) =>
    pickedFolder('my-plugin', { 'hew-plugin.json': manifestJson(over), 'dist/background.js': '// background', ...extra })

  it('packs the folder into a stored package the reader reads, and installs it as a file named after the folder', async () => {
    const { root, store } = setup()
    const staged = await store.stageFolder(asHandle(plugin({}, { 'icons/a.png': 'png' })))
    expect(staged.consent.source).toEqual({ kind: 'file', name: 'my-plugin', origin: 'folder' })
    expect(treeOf(root)).toEqual([])
    const record = await store.commit(staged.token, true)
    expect(record).toMatchObject({ id: 'com.example.one', source: { kind: 'file', name: 'my-plugin' }, enabled: true })
    expect(treeOf(root)).toEqual([
      'plugins/com.example.one/files/dist/background.js',
      'plugins/com.example.one/files/hew-plugin.json',
      'plugins/com.example.one/files/icons/a.png',
      'plugins/com.example.one/hew-plugin.json',
      'plugins/com.example.one/installed.json',
    ])
    expect(await textAt(root, 'plugins/com.example.one/files/dist/background.js')).toBe('// background')
  })

  it('builds what the kernel would read: every file, byte for byte, in a sorted stored zip', async () => {
    const bytes = await packFolder(asHandle(plugin({}, { 'b.txt': 'bee', 'a/z.txt': 'zed' })))
    const files = indexStoredZip(bytes)
    expect(files.map((f) => f.path)).toEqual(['a/z.txt', 'b.txt', 'dist/background.js', 'hew-plugin.json'])
    const at = (path: string) => {
      const f = files.find((x) => x.path === path)!
      return new TextDecoder().decode(bytes.subarray(f.offset, f.offset + f.size))
    }
    expect(at('b.txt')).toBe('bee')
    expect(at('a/z.txt')).toBe('zed')
  })

  it('refuses a name the package reader refuses, and names that collide ignoring case', async () => {
    const { store } = setup()
    for (const bad of ['a:b.js', 'CON.js', 'lpt1', 'trailing.', 'trailing ', 'caf\u00e9.js', 'tab\there', 'q?.js', 'back\\slash']) {
      expect(await refusal(store.stageFolder(asHandle(plugin({}, { [bad]: 'x' })))), bad).toBe('package_path_invalid')
    }
    expect(await refusal(store.stageFolder(asHandle(plugin({}, { 'A.js': 'x', 'a.js': 'y' }))))).toBe('package_path_invalid')
    expect(await refusal(store.stageFolder(asHandle(plugin({}, { 'Dist/x.js': 'x' }))))).toBe('package_path_invalid')
    expect(await refusal(store.stageFolder(asHandle(plugin({}, { [`${'a'.repeat(1020)}/b.js`]: 'x' }))))).toBe('package_path_invalid')
    for (const ok of ['.hidden', 'a-b_c.d.js', 'CONSOLE.js', 'x y.js']) expect(folderNameOk(ok), ok).toBe(true)
  })

  it('refuses a folder over the caps before reading any of it', async () => {
    const { store } = setup()
    for (const sizes of [{ 'one.bin': (32 << 20) + 1 }, { 'a.bin': 30 << 20, 'b.bin': 30 << 20, 'c.bin': 5 << 20 }] as Record<string, number>[]) {
      const { dir, reads } = sizedFolder(sizes)
      expect(await refusal(store.stageFolder(dir))).toBe('package_too_large')
      expect(reads).toEqual([])
    }
    const many: Record<string, number> = {}
    for (let i = 0; i < 4097; i++) many[`f${i}.txt`] = 0
    const { dir, reads } = sizedFolder(many)
    expect(await refusal(store.stageFolder(dir))).toBe('package_too_large')
    expect(reads).toEqual([])
    // Just under on the data, over once the zip's own headers are counted.
    const tight = sizedFolder({ 'a.bin': 32 << 20, 'b.bin': (32 << 20) - 10 })
    expect(await refusal(store.stageFolder(tight.dir))).toBe('package_too_large')
    expect(tight.reads).toEqual([])
  })

  it('refuses a file that changed size while it was read', async () => {
    const { store } = setup()
    const dir = plugin()
    const file = dir.files.get('hew-plugin.json')!
    let calls = 0
    const real = dir.getFileHandle.bind(dir)
    dir.getFileHandle = async (name: string, options?: { create?: boolean }) => {
      const h = await real(name, options)
      if (name !== 'hew-plugin.json') return h
      return {
        ...h,
        getFile: async () => {
          calls++
          const f = await h.getFile()
          return calls === 1 ? f : { ...f, arrayBuffer: async () => new Uint8Array([...file.bytes, 32]).buffer }
        },
      }
    }
    expect(await refusal(store.stageFolder(asHandle(dir)))).toBe('package_unreadable')
  })

  it('cuts off a listing that never ends, and a tree that nests without end, as too large', async () => {
    const { store } = setup()
    let yielded = 0
    const endless = {
      kind: 'directory',
      name: 'endless',
      async *entries() {
        for (;;) {
          yielded++
          yield [`d${yielded}`, { kind: 'directory', name: `d${yielded}`, async *entries() {} }]
        }
      },
    }
    expect(await refusal(store.stageFolder(endless as unknown as FileSystemDirectoryHandle))).toBe('package_too_large')
    expect(yielded).toBeLessThanOrEqual(4097)
    let depth = 0
    const nest = (): unknown => ({
      kind: 'directory',
      name: 'n',
      async *entries() {
        depth++
        yield ['n', nest()]
      },
    })
    expect(await refusal(store.stageFolder(nest() as FileSystemDirectoryHandle))).toBe('package_too_large')
    expect(depth).toBeLessThanOrEqual(513)
    let dirsYielded = 0
    const dirsOnly = {
      kind: 'directory',
      name: 'dirs',
      async *entries() {
        for (let i = 0; i < 5000; i++) {
          dirsYielded++
          yield [`e${i}`, { kind: 'directory', name: `e${i}`, async *entries() {} }]
        }
      },
    }
    expect(await refusal(store.stageFolder(dirsOnly as unknown as FileSystemDirectoryHandle))).toBe('package_too_large')
    // Folders count toward the entry cap, and the walk stops there.
    expect(dirsYielded).toBeLessThan(5000)
  })

  it('refuses a file that grew between being measured and being read, and stops a stream that outruns its size', async () => {
    const mk = (measured: number, later: number, chunks: number[]) => {
      let reads = 0
      let pulled = 0
      let calls = 0
      const dir = {
        kind: 'directory',
        name: 'p',
        async *entries() {
          yield [
            'hew-plugin.json',
            {
              kind: 'file',
              name: 'hew-plugin.json',
              getFile: async () => ({
                size: ++calls === 1 ? measured : later,
                arrayBuffer: async () => {
                  reads++
                  return new ArrayBuffer(later)
                },
                stream: () =>
                  new ReadableStream<Uint8Array>({
                    pull(c) {
                      const n = chunks[pulled++]
                      if (n === undefined) c.close()
                      else c.enqueue(new Uint8Array(n))
                    },
                  }),
              }),
            },
          ]
        },
      }
      return { dir: dir as unknown as FileSystemDirectoryHandle, reads: () => reads, pulled: () => pulled }
    }
    const grew = mk(10, 1 << 20, [])
    await expect(packFolder(grew.dir)).rejects.toMatchObject({ refusal: 'package_unreadable' })
    expect(grew.reads()).toBe(0)
    expect(grew.pulled()).toBe(0)
    const lies = mk(10, 10, [6, 6, 6, 6, 6])
    await expect(packFolder(lies.dir)).rejects.toMatchObject({ refusal: 'package_unreadable' })
    expect(lies.pulled()).toBeLessThan(5)
    const honest = mk(10, 10, [4, 6])
    expect((await packFolder(honest.dir)).length).toBeGreaterThan(10)
  })

  it('records a folder copy as one, and shows the stored record the field the kernel ignores', async () => {
    const { store } = setup()
    const staged = await store.stageFolder(asHandle(plugin()))
    expect(staged.consent.source).toEqual({ kind: 'file', name: 'my-plugin', origin: 'folder' })
    const record = await store.commit(staged.token, true)
    expect(record?.source).toEqual({ kind: 'file', name: 'my-plugin', origin: 'folder' })
    const [l] = await store.list()
    expect(l.kind === 'installed' && l.installed.source).toEqual({ kind: 'file', name: 'my-plugin', origin: 'folder' })
  })

  it('says a folder with no manifest has none', async () => {
    const { store } = setup()
    const failure = async (p: Promise<unknown>) => (await p.then(() => null, (e: PluginError) => e)) as PluginError
    const dir = pickedFolder('no-manifest', { 'dist/background.js': '// background' })
    const folder = await failure(store.stageFolder(asHandle(dir)))
    expect(folder.refusal).toBe('manifest_missing')
    expect(folder.message).toBe('This folder has no hew-plugin.json at its top level.')
  })

  it('cleans the folder name it records', async () => {
    const { store } = setup()
    const dir = plugin()
    Object.defineProperty(dir, 'name', { value: '\u202Emy\u0000 plugin\n' })
    const staged = await store.stageFolder(asHandle(dir))
    expect(staged.consent.source).toEqual({ kind: 'file', name: 'my plugin', origin: 'folder' })
  })
})

describe('per-document storage', () => {
  const ID = 'com.example.one'
  const DOC = '3f2c9107-5ed4-40a8-806b-1ec297405d83'
  const DOC2 = '00000000-0000-4000-8000-000000000002'
  const MiB = 1024 * 1024

  it('keeps one object of strings per document in plugin-data/<id>/documents/, and says which document changed', async () => {
    const posts: unknown[] = []
    const channel = { addEventListener: () => {}, postMessage: (m: unknown) => void posts.push(m) } as unknown as BroadcastChannel
    const { root, store } = setup({ channel })
    await install(store)
    const s = store.storage
    expect(await s.docStorageGet(ID, DOC, 'a')).toBeNull()
    expect(await s.docStorageList(ID, DOC)).toEqual([])
    // Reading makes nothing.
    expect(root.dirs.has('plugin-data')).toBe(false)
    posts.length = 0
    await s.docStorageSet(ID, DOC, 'b', '2')
    await s.docStorageSet(ID, DOC, 'a', '1')
    await s.docStorageSet(ID, DOC2, 'a', 'other')
    expect(await s.docStorageGet(ID, DOC, 'a')).toBe('1')
    expect(await s.docStorageList(ID, DOC)).toEqual(['a', 'b'])
    expect(await s.docStorageList(ID, DOC2)).toEqual(['a'])
    expect(JSON.parse((await textAt(root, `plugin-data/${ID}/documents/${DOC}.json`)) as string)).toEqual({ a: '1', b: '2' })
    // Plugin storage is a separate thing.
    expect(await s.storageList(ID)).toEqual([])
    expect(posts[0]).toEqual({ kind: 'storage', id: ID, scope: 'document', document_id: DOC, keys: ['b'] })
    posts.length = 0
    await s.docStorageDelete(ID, DOC, 'nothing')
    expect(posts).toEqual([])
    await s.docStorageDelete(ID, DOC, 'a')
    await s.docStorageDelete(ID, DOC, 'b')
    expect(posts).toHaveLength(2)
    // The file goes with its last key.
    expect(treeOf(root).filter((p) => p.startsWith('plugin-data/'))).toEqual([
      `plugin-data/${ID}/documents/${DOC2}.json`,
      `plugin-data/${ID}/usage.json`,
    ])
    // A pane in another tab is not woken by it.
    let hits = 0
    store.onChanged(() => hits++)
    await s.docStorageSet(ID, DOC, 'c', '3')
    expect(hits).toBe(0)
  })

  it('checks the plugin id, the document id and the key before any path, and needs an installed plugin', async () => {
    const { root, store } = setup()
    const s = store.storage
    expect(await refusal(s.docStorageSet(ID, DOC, 'k', 'v'))).toBe('not_installed')
    expect(await refusal(s.docStorageGet(ID, DOC, 'k'))).toBe('not_installed')
    expect(await refusal(s.docStorageList(ID, DOC))).toBe('not_installed')
    expect(await refusal(s.docStorageDelete(ID, DOC, 'k'))).toBe('not_installed')
    expect(root.dirs.size).toBe(0)
    await install(store)
    for (const bad of ['', '..', DOC.toUpperCase(), `${DOC}/../x`, '../../plugins/com.example.one/installed', DOC.slice(1)]) {
      expect(validDocumentId(bad), bad).toBe(false)
      expect(await refusal(s.docStorageSet(ID, bad, 'k', 'v')), bad).toBe('document_id_invalid')
      expect(await refusal(s.docStorageGet(ID, bad, 'k')), bad).toBe('document_id_invalid')
      expect(await refusal(s.docStorageList(ID, bad)), bad).toBe('document_id_invalid')
      expect(await refusal(s.docStorageDelete(ID, bad, 'k')), bad).toBe('document_id_invalid')
    }
    // The plugin id first, then the document id.
    expect(await refusal(s.docStorageSet('..', '..', 'k', 'v'))).toBe('invalid_id')
    expect(await refusal(s.docStorageSet(ID, DOC, '', 'v'))).toBe('storage_key_invalid')
    expect(await refusal(s.docStorageSet(ID, DOC, 'x'.repeat(257), 'v'))).toBe('storage_key_invalid')
    // A key with a lone surrogate has no UTF-8 form; refused in both scopes.
    expect(await refusal(s.docStorageSet(ID, DOC, 'a\uD800', 'v'))).toBe('storage_key_invalid')
    expect(await refusal(s.storageSet(ID, '\uDC00b', 'v'))).toBe('storage_key_invalid')
    // So is a value: the desktop cannot take one, and could not read it back.
    expect(await refusal(s.docStorageSet(ID, DOC, 'k', 'a\uD800'))).toBe('storage_value_invalid')
    expect(await refusal(s.storageSet(ID, 'k', '\uDC00b'))).toBe('storage_value_invalid')
    await s.storageSet(ID, 'k', 'ok \u{1F600}')
    await s.docStorageSet(ID, DOC, 'emoji \u{1F600}', 'v')
    expect(root.dirs.get('plugin-data')?.dirs.get(ID)?.dirs.get('documents')?.files.size).toBe(1)
  })

  it('reports a failure to make the data folder as io, not storage_unreadable', async () => {
    const { store } = setup()
    await install(store)
    const real = FakeDir.prototype.getDirectoryHandle
    const spy = vi.spyOn(FakeDir.prototype, 'getDirectoryHandle').mockImplementation(async function (this: FakeDir, name, opts) {
      if (name === 'plugin-data' && opts?.create) throw new DOMException('full', 'QuotaExceededError')
      return real.call(this, name, opts)
    })
    try {
      expect(await refusal(store.storage.docStorageSet(ID, DOC, 'k', 'v'))).toBe('io')
    } finally {
      spy.mockRestore()
    }
  })

  it('refuses one document past 5 MiB and the whole tree past 50 MiB, and writes nothing', async () => {
    const { root, store } = setup()
    await install(store)
    const s = store.storage
    expect(await refusal(s.docStorageSet(ID, DOC, 'a', 'x'.repeat(5 * MiB)))).toBe('storage_quota')
    const big = 'x'.repeat(5 * MiB - 100)
    const doc = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`
    for (let n = 0; n < 10; n++) await s.docStorageSet(ID, doc(n), 'a', big)
    await expect(s.docStorageSet(ID, doc(10), 'a', big)).rejects.toMatchObject({
      refusal: 'storage_quota',
      message: "This plugin's saved data would pass 50 MB in all, the most a plugin may keep.",
    })
    expect(await textAt(root, `plugin-data/${ID}/documents/${doc(10)}.json`)).toBeNull()
    // A missing or damaged total is measured again, and still refuses.
    const tree = root.dirs.get('plugin-data')!.dirs.get(ID)!
    tree.files.delete('usage.json')
    expect(await refusal(s.docStorageSet(ID, doc(10), 'a', big))).toBe('storage_quota')
    tree.files.set('usage.json', { bytes: new TextEncoder().encode('{"bytes":-5}'), mtime: 1 })
    expect(await refusal(s.docStorageSet(ID, doc(10), 'a', big))).toBe('storage_quota')
    // Shrinking makes room; a total left too high refuses nothing that fits.
    await s.docStorageDelete(ID, doc(0), 'a')
    tree.files.set('usage.json', { bytes: new TextEncoder().encode('{"bytes":999999999999}'), mtime: 1 })
    await s.docStorageSet(ID, doc(10), 'a', big)
    const total = JSON.parse((await textAt(root, `plugin-data/${ID}/usage.json`)) as string).bytes as number
    expect(total).toBeGreaterThanOrEqual(10 * (5 * MiB - 100))
    expect(total).toBeLessThanOrEqual(50 * MiB)
  })

  it('is untouched by an update and a rollback, removed on uninstall, and a new install starts empty', async () => {
    const { root, store } = setup()
    await install(store, { version: '1.0.0' })
    await install(store, { id: 'com.example.two' })
    await store.storage.docStorageSet(ID, DOC, 'k', 'v')
    await store.storage.docStorageSet('com.example.two', DOC, 'k', 'v')
    const before = treeOf(root).filter((p) => p.startsWith('plugin-data/'))
    const bytes = await textAt(root, `plugin-data/${ID}/documents/${DOC}.json`)
    await install(store, { version: '2.0.0' })
    await store.rollback(ID)
    expect(treeOf(root).filter((p) => p.startsWith('plugin-data/'))).toEqual(before)
    expect(await textAt(root, `plugin-data/${ID}/documents/${DOC}.json`)).toBe(bytes)
    await store.uninstall(ID)
    expect(root.dirs.get('plugin-data')?.dirs.has(ID)).toBe(false)
    expect(await store.storage.docStorageList('com.example.two', DOC)).toEqual(['k'])
    await install(store, { version: '1.0.0' })
    expect(await store.storage.docStorageList(ID, DOC)).toEqual([])
  })

  it('an uninstall cut short at any step never leaves data for a plugin that is gone, and nothing brings it back', async () => {
    const prepare = async () => {
      const env = setup()
      await install(env.store, { version: '1.0.0' })
      await install(env.store, { version: '2.0.0' })
      await env.store.storage.docStorageSet(ID, DOC, 'k', 'v')
      await env.store.storage.storageSet(ID, 'k', 'v')
      return env
    }
    const probe = injector()
    let total: number
    try {
      const env = await prepare()
      probe.arm(Infinity)
      await env.store.uninstall(ID)
      total = probe.count
    } finally {
      probe.restore()
    }
    expect(total).toBeGreaterThan(1)
    let gone = 0
    let kept = 0
    for (const sticky of [false, true]) {
      for (let at = 1; at <= total + 1; at++) {
        const inj = injector()
        const label = `${sticky ? 'every step from' : 'only'} step ${at}`
        try {
          const env = await prepare()
          inj.arm(at, sticky)
          let failed = false
          try {
            await env.store.uninstall(ID)
          } catch {
            failed = true
          }
          inj.disarm()
          const dataNow = () => env.root.dirs.get('plugin-data')?.dirs.has(ID) ?? false
          let dataGone = !dataNow()
          for (let again = 0; again < 3; again++) {
            const listed = await env.store.list()
            if (listed.length === 0) {
              expect(dataNow(), `${label}: data kept for a plugin that is gone`).toBe(false)
              if (again === 0) gone++
            } else if (again === 0) {
              kept++
              expect(failed, label).toBe(true)
            }
            // Once gone, never back.
            if (dataGone) expect(dataNow(), `${label}: the data came back`).toBe(false)
            dataGone = !dataNow()
            // A plugin still installed can still be uninstalled, data and all.
            if (again === 1 && listed.length > 0) {
              await env.store.uninstall(ID)
              expect(dataNow(), label).toBe(false)
            }
          }
          if (!failed) expect(dataNow(), label).toBe(false)
        } finally {
          inj.restore()
        }
      }
    }
    expect(gone).toBeGreaterThan(0)
    expect(kept).toBeGreaterThan(0)
  })

  it('writes under the lock and is exported as module functions', async () => {
    const root = new FakeDir('opfs-root')
    const locks = memoryLocks()
    const storageDesc = Object.getOwnPropertyDescriptor(navigator, 'storage')
    const locksDesc = Object.getOwnPropertyDescriptor(navigator, 'locks')
    Object.defineProperty(navigator, 'storage', { configurable: true, value: { getDirectory: async () => root } })
    Object.defineProperty(navigator, 'locks', { configurable: true, value: locks })
    try {
      const { store } = setup({ root })
      await install(store)
      locks.log.length = 0
      await docStorageSet(ID, DOC, 'b', '2')
      expect(locks.log).toEqual(['take hew-plugins', 'release hew-plugins'])
      await docStorageSet(ID, DOC, 'a', '1')
      expect(await docStorageGet(ID, DOC, 'a')).toBe('1')
      expect(await docStorageList(ID, DOC)).toEqual(['a', 'b'])
      await docStorageDelete(ID, DOC, 'a')
      expect(await docStorageList(ID, DOC)).toEqual(['b'])
      expect(await refusal(docStorageGet('com.example.none', DOC, 'k'))).toBe('not_installed')
    } finally {
      for (const [k, d] of [['storage', storageDesc], ['locks', locksDesc]] as const) {
        if (d) Object.defineProperty(navigator, k, d)
        else delete (navigator as unknown as Record<string, unknown>)[k]
      }
    }
  })
})

describe('setGrant', () => {
  it('writes the wider grant under the lock, answers with the record as stored and tells listeners', async () => {
    const { root, store, locks: lockProvider } = setup()
    const locks = lockProvider as ReturnType<typeof memoryLocks>
    await install(store, { permissions: ['model.read'], optionalPermissions: ['files', 'view'] })
    const changed = vi.fn()
    store.onChanged(changed)
    locks.log.length = 0
    const after = await store.setGrant('com.example.one', ['model.read'], ['files'])
    expect(after.grant).toEqual(['model.read'])
    expect(after.optional_granted).toEqual(['files'])
    expect(locks.log).toEqual(['take hew-plugins', 'release hew-plugins'])
    expect(changed).toHaveBeenCalledTimes(1)
    const stored = JSON.parse((await textAt(root, 'plugins/com.example.one/installed.json')) as string)
    expect(stored.optional_granted).toEqual(['files'])
    const listed = (await store.list())[0]
    expect(listed.kind === 'installed' && listed.installed.optional_granted).toEqual(['files'])
  })

  it('takes a permission back without asking anything, and keeps what else the record holds', async () => {
    const { root, store } = setup()
    await install(store, { optionalPermissions: ['files', 'view'] })
    await store.setGrant('com.example.one', ['model.read'], ['files', 'view'])
    const after = await store.setGrant('com.example.one', ['model.read'], ['view'])
    expect(after.optional_granted).toEqual(['view'])
    const stored = JSON.parse((await textAt(root, 'plugins/com.example.one/installed.json')) as string)
    expect(stored).toMatchObject({ enabled: true, version: '1.0.0', optional_granted: ['view'] })
  })

  it('refuses a name the manifest does not declare, and leaves the record as it was', async () => {
    const { root, store } = setup()
    await install(store, { permissions: ['model.read'], optionalPermissions: ['files'] })
    const before = await textAt(root, 'plugins/com.example.one/installed.json')
    expect(await refusal(store.setGrant('com.example.one', ['model.read'], ['view']))).toBe('unknown_permission')
    // A required name is not an optional one, and the other way round.
    expect(await refusal(store.setGrant('com.example.one', ['model.read'], ['model.read']))).toBe('unknown_permission')
    expect(await refusal(store.setGrant('com.example.one', ['files'], []))).toBe('unknown_permission')
    expect(await refusal(store.setGrant('com.example.one', ['model.read'], [{ x: 1 }] as unknown as string[]))).toBe('unknown_permission')
    expect(await textAt(root, 'plugins/com.example.one/installed.json')).toBe(before)
  })

  it('refuses a plugin that is not installed, and an id that is not one', async () => {
    const { store } = setup()
    expect(await refusal(store.setGrant('com.example.none', [], []))).toBe('not_installed')
    expect(await refusal(store.setGrant('../x', [], []))).toBe('invalid_id')
  })

  it('refuses where two tabs cannot be kept apart, as every change does', async () => {
    const { store } = setup({ locks: null })
    expect(await refusal(store.setGrant('com.example.one', [], []))).toBe('locks_unavailable')
  })

  it('does not run two changes at once: a second grant waits for the first', async () => {
    const { store, locks: lockProvider } = setup()
    const locks = lockProvider as ReturnType<typeof memoryLocks>
    await install(store, { optionalPermissions: ['files', 'view'] })
    locks.log.length = 0
    await Promise.all([store.setGrant('com.example.one', ['model.read'], ['files']), store.setGrant('com.example.one', ['model.read'], ['files', 'view'])])
    expect(locks.log).toEqual(['take hew-plugins', 'release hew-plugins', 'take hew-plugins', 'release hew-plugins'])
    const listed = (await store.list())[0]
    expect(listed.kind === 'installed' && listed.installed.optional_granted).toEqual(['files', 'view'])
  })
})

describe('grantLines', () => {
  const lines = JSON.stringify({ added: [{ name: 'files', text: 'Ask you to pick files,   and save files you choose' }], warnings: ['Careful.'] })

  it('asks the kernel with the stored manifest, the stored record and the names, and cleans what comes back', async () => {
    const wasm: PluginWasm = { ...fakeWasm(), plugin_grant_lines: vi.fn(() => lines) }
    const { store } = setup({ wasm })
    await install(store, { optionalPermissions: ['files'] })
    const got = await store.grantLines('com.example.one', ['model.read'], ['files'])
    expect(got).toEqual({ added: [{ name: 'files', text: 'Ask you to pick files, and save files you choose' }], warnings: ['Careful.'] })
    const call = vi.mocked(wasm.plugin_grant_lines as NonNullable<PluginWasm['plugin_grant_lines']>).mock.calls[0]
    expect(JSON.parse(call[0]).id).toBe('com.example.one')
    expect(JSON.parse(call[1]).optional_granted).toEqual([])
    expect(JSON.parse(call[2])).toEqual(['model.read'])
    expect(JSON.parse(call[3])).toEqual(['files'])
  })

  it('drops what is not a line, and says so plainly when the kernel cannot say', async () => {
    const wasm: PluginWasm = { ...fakeWasm(), plugin_grant_lines: () => JSON.stringify({ added: [{ name: 1 }, 'x', { name: 'files', text: 'ok' }], warnings: [3, 'w'] }) }
    const { store } = setup({ wasm })
    await install(store, { optionalPermissions: ['files'] })
    expect(await store.grantLines('com.example.one', [], ['files'])).toEqual({ added: [{ name: 'files', text: 'ok' }], warnings: ['w'] })
    const plain = setup()
    await install(plain.store, { optionalPermissions: ['files'] })
    expect(await refusal(plain.store.grantLines('com.example.one', [], ['files']))).toBe('unsupported')
  })

  it('passes the kernel\'s refusal of a name on', async () => {
    const wasm: PluginWasm = {
      ...fakeWasm(),
      plugin_grant_lines: () => {
        throw 'MANIFEST_UNKNOWN_PERMISSION: not declared'
      },
    }
    const { store } = setup({ wasm })
    await install(store)
    expect(await refusal(store.grantLines('com.example.one', [], ['view']))).toBe('unknown_permission')
  })
})
