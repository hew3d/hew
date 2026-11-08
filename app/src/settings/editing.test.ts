// @vitest-environment jsdom
import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest'
import { getCopiesLinked, setCopiesLinked, subscribeCopiesLinked } from './editing'

beforeEach(() => {
  setCopiesLinked(true)
  localStorage.clear()
})

describe('copies linked setting', () => {
  it('copies_are_linked_defaults_to_true_and_persists', async () => {
    localStorage.clear()
    vi.resetModules()
    const fresh = await import('./editing')
    expect(fresh.getCopiesLinked()).toBe(true)
    fresh.setCopiesLinked(false)
    expect(localStorage.getItem('hew.settings.copiesLinked')).toBe('false')
    vi.resetModules()
    const again = await import('./editing')
    expect(again.getCopiesLinked()).toBe(false)
    localStorage.setItem('hew.settings.copiesLinked', 'garbage')
    vi.resetModules()
    expect((await import('./editing')).getCopiesLinked()).toBe(true)
  })

  it('notifies subscribers and syncs from another tab through the storage event', () => {
    const cb = vi.fn()
    const off = subscribeCopiesLinked(cb)
    setCopiesLinked(false)
    expect(cb).toHaveBeenLastCalledWith(false)
    window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.copiesLinked', newValue: 'true' }))
    expect(getCopiesLinked()).toBe(true)
    off()
  })
})

describe('settings-changed from another window (desktop shell)', () => {
  afterEach(() => {
    // @ts-expect-error — jsdom has no such property unless a test set it.
    delete window.__TAURI_INTERNALS__
  })
  it('copies_linked_setting_syncs_across_windows_through_the_settings_event', async () => {
    const callbacks: ((e: { payload: unknown }) => void)[] = []
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      value: {
        transformCallback: (cb: (e: { payload: unknown }) => void) => callbacks.push(cb),
        invoke: async () => 0,
        unregisterCallback: () => {},
      },
      configurable: true,
    })
    vi.resetModules()
    const fresh = await import('./editing')
    await vi.waitFor(() => expect(callbacks.length).toBeGreaterThanOrEqual(1))
    const send = (payload: unknown): void => callbacks.forEach((h) => h({ payload }))
    send({ copiesLinked: false })
    expect(fresh.getCopiesLinked()).toBe(false)
    send({ theme: 'dark' })
    send({ copiesLinked: 'garbage' })
    expect(fresh.getCopiesLinked()).toBe(false)
    send({ copiesLinked: true })
    expect(fresh.getCopiesLinked()).toBe(true)
  })
})
