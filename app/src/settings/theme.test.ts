import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { getThemeSetting, setThemeSetting, subscribe, resolveTheme, getResolvedTheme } from './theme'

const STORAGE_KEY = 'hew.settings.theme'

// This project's vitest environment is 'node' (no jsdom/happy-dom dep), so
// there is no real `localStorage` global — theme.ts already guards every
// access in try/catch (privacy-mode / unavailable storage), which is exactly
// what lets it run at all under plain Node. Mirrors debugMode.test.ts's stub.
class FakeStorage {
  private store = new Map<string, string>()
  getItem(key: string): string | null {
    return this.store.has(key) ? this.store.get(key)! : null
  }
  setItem(key: string, value: string): void {
    this.store.set(key, value)
  }
  removeItem(key: string): void {
    this.store.delete(key)
  }
  clear(): void {
    this.store.clear()
  }
}

let originalLocalStorage: unknown

beforeEach(() => {
  originalLocalStorage = (globalThis as { localStorage?: unknown }).localStorage
  ;(globalThis as { localStorage?: unknown }).localStorage = new FakeStorage()
  setThemeSetting('auto')
})

afterEach(() => {
  ;(globalThis as { localStorage?: unknown }).localStorage = originalLocalStorage
})

describe('theme setting', () => {
  it('defaults to auto', () => {
    expect(getThemeSetting()).toBe('auto')
  })

  it('set/get round-trips through all three values', () => {
    setThemeSetting('dark')
    expect(getThemeSetting()).toBe('dark')
    setThemeSetting('light')
    expect(getThemeSetting()).toBe('light')
    setThemeSetting('auto')
    expect(getThemeSetting()).toBe('auto')
  })

  it('subscribe fires on change with the new value', () => {
    const seen: string[] = []
    const unsub = subscribe((s) => seen.push(s))
    setThemeSetting('dark')
    setThemeSetting('light')
    expect(seen).toEqual(['dark', 'light'])
    unsub()
  })

  it('unsubscribe stops further notifications', () => {
    const seen: string[] = []
    const unsub = subscribe((s) => seen.push(s))
    setThemeSetting('dark')
    unsub()
    setThemeSetting('light')
    expect(seen).toEqual(['dark'])
  })

  it('persists to localStorage', () => {
    setThemeSetting('dark')
    expect(localStorage.getItem(STORAGE_KEY)).toBe('dark')
    setThemeSetting('light')
    expect(localStorage.getItem(STORAGE_KEY)).toBe('light')
  })
})

describe('resolveTheme', () => {
  it('passes explicit light/dark through unchanged', () => {
    expect(resolveTheme('light')).toBe('light')
    expect(resolveTheme('dark')).toBe('dark')
  })

  it('falls back to dark for auto when there is no window (this suite\'s default node environment)', () => {
    expect(typeof window).toBe('undefined')
    expect(resolveTheme('auto')).toBe('dark')
  })

  it('resolves auto via a mocked matchMedia when window is present', () => {
    const original = (globalThis as { window?: unknown }).window
    const fakeWindow = {
      matchMedia: (query: string) => ({
        matches: query === '(prefers-color-scheme: dark)',
      }),
    }
    ;(globalThis as { window?: unknown }).window = fakeWindow
    try {
      expect(resolveTheme('auto')).toBe('dark')
    } finally {
      ;(globalThis as { window?: unknown }).window = original
    }
  })

  it('getResolvedTheme wraps resolveTheme(getThemeSetting())', () => {
    setThemeSetting('light')
    expect(getResolvedTheme()).toBe('light')
    setThemeSetting('dark')
    expect(getResolvedTheme()).toBe('dark')
  })
})
