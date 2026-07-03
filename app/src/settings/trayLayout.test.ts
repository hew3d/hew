import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { getTrayLayout, setTrayLayout, subscribe, DEFAULT_TRAY_LAYOUT, type TrayLayout } from './trayLayout'

const STORAGE_KEY = 'hew.settings.trayLayout'

// This project's vitest environment is 'node' for .test.ts (no jsdom), so
// there is no real `localStorage` global — trayLayout.ts already guards every
// access in try/catch (privacy-mode / unavailable storage), which is exactly
// what lets it run at all under plain Node. Mirrors theme.test.ts's stub.
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
  setTrayLayout(DEFAULT_TRAY_LAYOUT)
})

afterEach(() => {
  ;(globalThis as { localStorage?: unknown }).localStorage = originalLocalStorage
})

const ALL_OPEN: TrayLayout = { modelInfo: true, objectInfo: true, materials: true, tags: true }

describe('tray layout setting', () => {
  it('defaults to the layout: Entity Info + Outliner open, Materials + Tags collapsed', () => {
    expect(DEFAULT_TRAY_LAYOUT).toEqual({ modelInfo: true, objectInfo: true, materials: false, tags: false })
    expect(getTrayLayout()).toEqual(DEFAULT_TRAY_LAYOUT)
  })

  it('set/get round-trips a full layout', () => {
    setTrayLayout(ALL_OPEN)
    expect(getTrayLayout()).toEqual(ALL_OPEN)
    const closedTags: TrayLayout = { ...ALL_OPEN, tags: false }
    setTrayLayout(closedTags)
    expect(getTrayLayout()).toEqual(closedTags)
  })

  it('set copies its argument (later caller-side mutation does not leak in)', () => {
    const layout: TrayLayout = { ...DEFAULT_TRAY_LAYOUT }
    setTrayLayout(layout)
    layout.materials = true
    expect(getTrayLayout().materials).toBe(false)
  })

  it('subscribe fires on change with the new value', () => {
    const seen: TrayLayout[] = []
    const unsub = subscribe((l) => seen.push(l))
    setTrayLayout(ALL_OPEN)
    expect(seen).toEqual([ALL_OPEN])
    unsub()
  })

  it('unsubscribe stops further notifications', () => {
    const seen: TrayLayout[] = []
    const unsub = subscribe((l) => seen.push(l))
    setTrayLayout(ALL_OPEN)
    unsub()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
    expect(seen).toEqual([ALL_OPEN])
  })

  it('persists to localStorage as JSON under the settings naming scheme', () => {
    setTrayLayout(ALL_OPEN)
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual(ALL_OPEN)
  })
})

// loadInitial runs at module-evaluation time, so restoring-from-storage needs
// a FRESH module instance per case (vi.resetModules + dynamic import); the
// statically-imported instance above is unaffected.
describe('tray layout restore on load', () => {
  it('restores a persisted layout, merging missing/mistyped flags over defaults', async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ materials: true, tags: 'yes' }))
    vi.resetModules()
    const fresh = await import('./trayLayout')
    expect(fresh.getTrayLayout()).toEqual({ ...DEFAULT_TRAY_LAYOUT, materials: true })
  })

  it('falls back to the defaults on malformed JSON', async () => {
    localStorage.setItem(STORAGE_KEY, '{not json')
    vi.resetModules()
    const fresh = await import('./trayLayout')
    expect(fresh.getTrayLayout()).toEqual(DEFAULT_TRAY_LAYOUT)
  })

  it('falls back to the defaults on a non-object value', async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(42))
    vi.resetModules()
    const fresh = await import('./trayLayout')
    expect(fresh.getTrayLayout()).toEqual(DEFAULT_TRAY_LAYOUT)
  })
})
