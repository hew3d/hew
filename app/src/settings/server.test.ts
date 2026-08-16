// @vitest-environment jsdom
/**
 * settings/server.ts — the Rust-backed "Open on Phone" server setting's JS
 * cache. `@tauri-apps/api/core` is mocked; `isTauri` is flipped on by
 * defining `window.__TAURI_INTERNALS__` BEFORE the module under test first
 * evaluates (the same isolation trick App.openInNewWindow.test.tsx uses),
 * which is why the module is imported dynamically inside the tests.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))
const mockListen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen, emit: vi.fn() }))

async function loadTauriModule() {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
  vi.resetModules()
  return import('./server')
}

async function loadWebModule() {
  // @ts-expect-error — jsdom has no such property unless a test set it.
  delete window.__TAURI_INTERNALS__
  vi.resetModules()
  return import('./server')
}

describe('settings/server', () => {
  beforeEach(() => {
    mockInvoke.mockReset()
    mockListen.mockReset()
    mockListen.mockResolvedValue(() => {})
  })

  afterEach(() => {
    // @ts-expect-error — see loadWebModule.
    delete window.__TAURI_INTERNALS__
  })

  it('effectiveOrigin: cloud mode always targets the Hew cloud, self-hosted its own origin', async () => {
    const m = await loadWebModule()
    expect(m.effectiveOrigin({ mode: 'cloud', origin: 'https://hew.example.org', uploadKey: 'k' })).toBe(m.CLOUD_ORIGIN)
    expect(m.effectiveOrigin({ mode: 'self-hosted', origin: 'https://hew.example.org', uploadKey: '' })).toBe(
      'https://hew.example.org',
    )
  })

  it('web build: not available, resolves the default without ever invoking, and refuses to set', async () => {
    const m = await loadWebModule()
    expect(m.serverSettingAvailable()).toBe(false)
    expect(await m.getServerSetting()).toEqual(m.DEFAULT_SERVER_SETTING)
    expect(mockInvoke).not.toHaveBeenCalled()
    await expect(m.setServerSetting(m.DEFAULT_SERVER_SETTING)).rejects.toThrow(/desktop-only/)
  })

  it('desktop: asks Rust once and caches; setServerSetting persists through Rust and notifies subscribers', async () => {
    const m = await loadTauriModule()
    const stored = { mode: 'self-hosted' as const, origin: 'https://hew.example.org', uploadKey: 'k' }
    mockInvoke.mockImplementation(async (cmd: string, args?: { setting?: unknown }) => {
      if (cmd === 'get_server_setting') return stored
      if (cmd === 'set_server_setting') return { ...(args?.setting as object), origin: 'https://normalized.example' }
      throw new Error(cmd)
    })
    expect(m.serverSettingAvailable()).toBe(true)
    expect(m.getCachedServerSetting()).toBeNull()
    expect(await m.getServerSetting()).toEqual(stored)
    expect(await m.getServerSetting()).toEqual(stored)
    expect(mockInvoke.mock.calls.filter((c) => c[0] === 'get_server_setting')).toHaveLength(1)
    expect(m.getCachedServerSetting()).toEqual(stored)

    const listener = vi.fn()
    const unsubscribe = m.subscribe(listener)
    const saved = await m.setServerSetting({ mode: 'self-hosted', origin: 'HTTPS://Normalized.Example/', uploadKey: '' })
    expect(mockInvoke).toHaveBeenCalledWith('set_server_setting', {
      setting: { mode: 'self-hosted', origin: 'HTTPS://Normalized.Example/', uploadKey: '' },
    })
    // The cache takes Rust's canonical answer, not what was sent.
    expect(saved.origin).toBe('https://normalized.example')
    expect(m.getCachedServerSetting()).toEqual(saved)
    expect(listener).toHaveBeenCalledTimes(1)
    unsubscribe()
  })

  it('desktop: a rejected set_server_setting propagates the Rust error and leaves the cache alone', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_server_setting') return m.DEFAULT_SERVER_SETTING
      if (cmd === 'set_server_setting') throw { kind: 'invalidOrigin', message: 'the address needs a host name' }
      throw new Error(cmd)
    })
    await m.getServerSetting()
    await expect(m.setServerSetting({ mode: 'self-hosted', origin: 'https://', uploadKey: '' })).rejects.toMatchObject({
      kind: 'invalidOrigin',
    })
    expect(m.getCachedServerSetting()).toEqual(m.DEFAULT_SERVER_SETTING)
  })

  it('desktop: follows another window through the settings-changed event with a server payload', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockResolvedValue(m.DEFAULT_SERVER_SETTING)
    await m.getServerSetting()
    // Let the module's own listen() registration settle.
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(mockListen).toHaveBeenCalledWith('settings-changed', expect.any(Function))
    const handler = mockListen.mock.calls.find((c) => c[0] === 'settings-changed')?.[1] as (e: {
      payload: unknown
    }) => void
    const listener = vi.fn()
    m.subscribe(listener)
    handler({ payload: { key: 'server', server: { mode: 'self-hosted', origin: 'https://x.example', uploadKey: '' } } })
    expect(listener).toHaveBeenCalledTimes(1)
    expect(m.getCachedServerSetting()).toEqual({ mode: 'self-hosted', origin: 'https://x.example', uploadKey: '' })
    // Other keys, or a malformed payload, are ignored.
    handler({ payload: { key: 'libraryFolder' } })
    handler({ payload: { key: 'server', server: { mode: 'nope' } } })
    expect(listener).toHaveBeenCalledTimes(1)
  })
})
