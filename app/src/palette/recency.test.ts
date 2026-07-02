import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { getRecent, recordRun, subscribe, clearRecent } from './recency'

const STORAGE_KEY = 'hew.palette.recent'

// Mirrors settings/debugMode.test.ts's FakeStorage — this project's vitest
// environment is 'node', with no real `localStorage` global.
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
  // recency.ts's `recent` array is a module-level singleton that otherwise
  // persists across tests in this file — clearRecent() resets it to a known
  // empty state (mirrors debugMode.test.ts's setDebugMode(false) reset).
  clearRecent()
})

afterEach(() => {
  ;(globalThis as { localStorage?: unknown }).localStorage = originalLocalStorage
})

describe('recency', () => {
  it('recordRun moves an id to the front', () => {
    recordRun('a')
    recordRun('b')
    recordRun('c')
    expect(getRecent()).toEqual(['c', 'b', 'a'])
  })

  it('re-running an existing id moves it to the front without duplicating', () => {
    recordRun('a')
    recordRun('b')
    recordRun('a')
    expect(getRecent()).toEqual(['a', 'b'])
  })

  it('caps at MAX_RECENT (10)', () => {
    for (let i = 0; i < 15; i++) recordRun(`id-${i}`)
    expect(getRecent().length).toBe(10)
    expect(getRecent()[0]).toBe('id-14')
  })

  it('persists to localStorage', () => {
    recordRun('x')
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual(['x'])
  })

  it('subscribe fires on recordRun with the new list', () => {
    const seen: string[][] = []
    const unsub = subscribe((ids) => seen.push(ids))
    recordRun('a')
    recordRun('b')
    expect(seen).toEqual([['a'], ['b', 'a']])
    unsub()
  })

  it('unsubscribe stops further notifications', () => {
    const seen: string[][] = []
    const unsub = subscribe((ids) => seen.push(ids))
    recordRun('a')
    unsub()
    recordRun('b')
    expect(seen).toEqual([['a']])
  })

  it('clearRecent empties the list and clears storage', () => {
    recordRun('a')
    recordRun('b')
    clearRecent()
    expect(getRecent()).toEqual([])
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull()
  })

  it('clearRecent notifies subscribers with an empty list', () => {
    recordRun('a')
    const seen: string[][] = []
    const unsub = subscribe((ids) => seen.push(ids))
    clearRecent()
    expect(seen).toEqual([[]])
    unsub()
  })
})
