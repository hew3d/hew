import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { getDebugMode, setDebugMode, subscribe } from './debugMode'

const STORAGE_KEY = 'hew.settings.debugMode'

// This project's vitest environment is 'node' (no jsdom/happy-dom dep), so
// there is no real `localStorage` global — debugMode.ts already guards every
// access in try/catch (privacy-mode / unavailable storage), which is exactly
// what lets it run at all under plain Node. To exercise the persistence
// behavior itself, install a minimal in-memory Storage-like stub for the
// duration of this suite (mirrors reproducerDump.test.ts's withFakeWindow).
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
  setDebugMode(false)
})

afterEach(() => {
  ;(globalThis as { localStorage?: unknown }).localStorage = originalLocalStorage
})

describe('debugMode', () => {
  it('defaults to false', () => {
    expect(getDebugMode()).toBe(false)
  })

  it('set/get round-trips', () => {
    setDebugMode(true)
    expect(getDebugMode()).toBe(true)
    setDebugMode(false)
    expect(getDebugMode()).toBe(false)
  })

  it('subscribe fires on change with the new value', () => {
    const seen: boolean[] = []
    const unsub = subscribe((on) => seen.push(on))
    setDebugMode(true)
    setDebugMode(false)
    expect(seen).toEqual([true, false])
    unsub()
  })

  it('unsubscribe stops further notifications', () => {
    const seen: boolean[] = []
    const unsub = subscribe((on) => seen.push(on))
    setDebugMode(true)
    unsub()
    setDebugMode(false)
    expect(seen).toEqual([true])
  })

  it('persists to localStorage', () => {
    setDebugMode(true)
    expect(localStorage.getItem(STORAGE_KEY)).toBe('true')
    setDebugMode(false)
    expect(localStorage.getItem(STORAGE_KEY)).toBe('false')
  })
})
