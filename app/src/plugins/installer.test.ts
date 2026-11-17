import { beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { FakeDir } from '../test/opfsFake'
import { makeDesktopInstaller, makeWebInstaller } from './installer'
import { fakeWasm, pluginFile } from './testKit'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

describe('desktop installer', () => {
  const setup = () => {
    const calls: [string, unknown][] = []
    const answers = new Map<string, unknown>()
    const invoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
      calls.push([command, args])
      const a = answers.get(command)
      if (a instanceof Error) throw a
      if (typeof a === 'function') return (a as () => unknown)()
      return a
    })
    const handlers: ((e: { payload: unknown }) => void)[] = []
    const events: string[] = []
    const unlisten = vi.fn()
    const listen = vi.fn(async (event: string, h: (e: { payload: unknown }) => void) => {
      events.push(event)
      handlers.push(h)
      return unlisten
    })
    return { calls, answers, handlers, events, unlisten, installer: makeDesktopInstaller({ invoke: invoke as never, listen }) }
  }

  it('maps each call to its command with the page arguments only', async () => {
    const { calls, answers, installer } = setup()
    answers.set('plugin_list', [])
    answers.set('plugin_commit', null)
    answers.set('plugin_set_enabled', ['b'])
    answers.set('plugin_rollback', { id: 'a' })
    await installer.list()
    await installer.commit('tok', false)
    expect(await installer.setEnabled('a', false)).toEqual(['b'])
    await installer.uninstall('a')
    await installer.rollback('a')
    expect(calls).toEqual([
      ['plugin_list', undefined],
      ['plugin_commit', { token: 'tok', accept: false }],
      ['plugin_set_enabled', { id: 'a', enabled: false }],
      ['plugin_uninstall', { id: 'a' }],
      ['plugin_rollback', { id: 'a' }],
    ])
    expect(installer.tier).toBe('desktop')
  })

  it('picks, then stages only what was picked; a cancelled pick stages nothing', async () => {
    const { calls, answers, installer } = setup()
    answers.set('plugin_pick_package', null)
    expect(await installer.pickAndStage('file')).toBeNull()
    expect(calls.map((c) => c[0])).toEqual(['plugin_pick_package'])
    answers.set('plugin_pick_dir', '/home/me/plugin')
    answers.set('plugin_stage_dir', { token: 't', consent: {} })
    expect(await installer.pickAndStage('dir')).toEqual({ token: 't', consent: {} })
    expect(calls.slice(1)).toEqual([
      ['plugin_pick_dir', undefined],
      ['plugin_stage_dir', { path: '/home/me/plugin' }],
    ])
    await expect(installer.stageFile(new File([], 'x.hewplugin'))).rejects.toThrow()
  })

  it('turns a shell refusal string into a named error with the text alone as its message', async () => {
    const { answers, installer } = setup()
    answers.set('plugin_commit', Object.assign(new Error('x'), { message: 'pending_expired: This install waited too long.' }))
    await expect(installer.commit('t', true)).rejects.toMatchObject({ refusal: 'pending_expired', message: 'This install waited too long.' })
  })

  it('makes an icon blob URL only for an image type, and null when unreadable', async () => {
    const { answers, installer } = setup()
    const make = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:icon')
    answers.set('plugin_read_file', new Uint8Array([1, 2]).buffer)
    expect(await installer.iconUrl('a', 'icon.png')).toBe('blob:icon')
    expect(await installer.iconUrl('a', 'icon.html')).toBeNull()
    answers.set('plugin_read_file', new Error('package_path_invalid: no'))
    expect(await installer.iconUrl('a', 'icon.png')).toBeNull()
    make.mockRestore()
  })

  it('listens for changes and stops, even when asked to stop before the listener is ready', async () => {
    const { handlers, unlisten, installer } = setup()
    const fn = vi.fn()
    const off = installer.onChanged(fn)
    await Promise.resolve()
    await Promise.resolve()
    handlers[0]({ payload: null })
    expect(fn).toHaveBeenCalledOnce()
    off()
    expect(unlisten).toHaveBeenCalledOnce()
    const early = installer.onChanged(fn)
    early()
    await Promise.resolve()
    await Promise.resolve()
    expect(unlisten).toHaveBeenCalledTimes(2)
  })

  it('reads and sets developer mode, reviews, and looks at a waiting request through the shell', async () => {
    const { calls, answers, installer } = setup()
    answers.set('plugin_dev_mode', true)
    answers.set('plugin_set_dev_mode', false)
    answers.set('plugin_dev_review', { token: 'r', consent: { review: true } })
    const id = 'c'.repeat(32)
    answers.set('plugin_dev_request', { id, path: '/home/me/\u202Eplugin\n' })
    expect(await installer.devMode!()).toBe(true)
    expect(await installer.setDevMode!(false)).toBe(false)
    expect(await installer.review!('com.example.dev')).toEqual({ token: 'r', consent: { review: true } })
    // Display text only, cleaned again here.
    expect(await installer.devRequest!()).toEqual({ id, path: '/home/me/plugin' })
    answers.set('plugin_dev_request', null)
    expect(await installer.devRequest!()).toBeNull()
    answers.set('plugin_dev_request', { id, path: 7 })
    expect(await installer.devRequest!()).toBeNull()
    // An id that is not one the shell makes is dropped with the request.
    answers.set('plugin_dev_request', { id: '../x', path: '/home/me/plugin' })
    expect(await installer.devRequest!()).toBeNull()
    // A value that is not exactly true is off.
    answers.set('plugin_dev_mode', 'yes')
    expect(await installer.devMode!()).toBe(false)
    expect(calls.map((c) => c)).toEqual([
      ['plugin_dev_mode', undefined],
      ['plugin_set_dev_mode', { on: false }],
      ['plugin_dev_review', { id: 'com.example.dev' }],
      ['plugin_dev_request', undefined],
      ['plugin_dev_request', undefined],
      ['plugin_dev_request', undefined],
      ['plugin_dev_request', undefined],
      ['plugin_dev_mode', undefined],
    ])
    answers.set('plugin_set_dev_mode', Object.assign(new Error('x'), { message: 'developer_mode_declined: Developer mode was left off.' }))
    await expect(installer.setDevMode!(true)).rejects.toMatchObject({ refusal: 'developer_mode_declined' })
  })

  it("opens the folder picker for a request by its id alone, never a path, and dismisses by id", async () => {
    const { calls, answers, installer } = setup()
    const id = 'd'.repeat(32)
    answers.set('plugin_pick_dir', '/home/me/picked')
    answers.set('plugin_stage_dir', { token: 't', consent: {} })
    answers.set('plugin_dev_request_dismiss', null)
    await installer.pickAndStage('dir', { useRequest: id })
    await installer.dismissDevRequest!(id)
    expect(calls).toEqual([
      ['plugin_pick_dir', { useRequest: id }],
      ['plugin_stage_dir', { path: '/home/me/picked' }],
      ['plugin_dev_request_dismiss', { id }],
    ])
  })

  it('passes on well-formed requests, and null when one is gone', async () => {
    const { events, handlers, installer } = setup()
    const fn = vi.fn()
    installer.onDevRequest!(fn)
    await Promise.resolve()
    await Promise.resolve()
    const at = events.indexOf('plugins-dev-request')
    expect(at).toBeGreaterThanOrEqual(0)
    for (const payload of ['x', { path: '/x' }, { id: 'zz', path: '/x' }, []]) handlers[at]({ payload })
    expect(fn).not.toHaveBeenCalled()
    handlers[at]({ payload: { id: 'e'.repeat(32), path: '/x' } })
    handlers[at]({ payload: null })
    expect(fn.mock.calls).toEqual([[{ id: 'e'.repeat(32), path: '/x' }], [null]])
    // At the event boundary the path is cleaned and cut to the shell's 512
    // characters, whatever arrives (the notice cuts it again to show it).
    handlers[at]({ payload: { id: 'e'.repeat(32), path: `/\u202E${'a'.repeat(5_000)}` } })
    const cut = (fn.mock.calls[2][0] as { path: string }).path
    expect(Array.from(cut)).toHaveLength(512)
    expect(cut.startsWith('/aaa')).toBe(true)
  })

  it('passes on only well-formed folder change notices', async () => {
    const { events, handlers, installer } = setup()
    const fn = vi.fn()
    installer.onDevChanged!(fn)
    await Promise.resolve()
    await Promise.resolve()
    const at = events.indexOf('plugins-dev-changed')
    expect(at).toBeGreaterThanOrEqual(0)
    for (const payload of [null, 'x', { id: 'a' }, { id: 'a', manifest_changed: 'no' }, { id: 'a', manifest_changed: false, problem: 'Not A Name' }, { id: 'x'.repeat(129), manifest_changed: false }]) {
      handlers[at]({ payload })
    }
    expect(fn).not.toHaveBeenCalled()
    handlers[at]({ payload: { id: 'com.example.dev', manifest_changed: false, reason: null, problem: 'folder_missing' } })
    expect(fn).toHaveBeenCalledWith({ id: 'com.example.dev', manifest_changed: false, reason: null, problem: 'folder_missing' })
  })
})

describe('web installer', () => {
  it('stages a file, installs it and serves its icon from the store', async () => {
    const root = new FakeDir('opfs-root')
    const installer = makeWebInstaller({
      wasm: async () => fakeWasm(),
      opfsRoot: async () => root as unknown as FileSystemDirectoryHandle,
      channel: null,
      storage: null,
    })
    expect(installer.tier).toBe('web')
    await expect(installer.pickAndStage('file')).rejects.toThrow()
    const staged = await installer.stageFile(pluginFile({ icon: 'icon.png' }, { 'icon.png': 'png-bytes' }))
    await installer.commit(staged.token, true)
    const make = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:icon')
    expect(await installer.iconUrl('com.example.one', 'icon.png')).toBe('blob:icon')
    expect(await installer.iconUrl('com.example.one', 'dist/background.js')).toBeNull()
    expect(await installer.iconUrl('com.example.one', '../installed.json')).toBeNull()
    make.mockRestore()
  })
})
