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
    const handlers: (() => void)[] = []
    const unlisten = vi.fn()
    const listen = vi.fn(async (_e: string, h: () => void) => {
      handlers.push(h)
      return unlisten
    })
    return { calls, answers, handlers, unlisten, installer: makeDesktopInstaller({ invoke: invoke as never, listen }) }
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
    handlers[0]()
    expect(fn).toHaveBeenCalledOnce()
    off()
    expect(unlisten).toHaveBeenCalledOnce()
    const early = installer.onChanged(fn)
    early()
    await Promise.resolve()
    await Promise.resolve()
    expect(unlisten).toHaveBeenCalledTimes(2)
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
