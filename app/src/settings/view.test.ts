// @vitest-environment jsdom
import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest'
import {
  ZOOM_WHEEL_FACTORS,
  getZoomWheelStop,
  setZoomWheelStop,
  getZoomWheelSpeed,
  zoomWheelFactor,
  getSmoothCamera,
  setSmoothCamera,
  shouldAnimateCamera,
  inertiaEnabled,
  getShowToolHints,
  setShowToolHints,
  getShowToolPointer,
  setShowToolPointer,
  resolveToolCursor,
  PLAIN_CURSOR,
  getMeasurementsPosition,
  setMeasurementsPosition,
  subscribeMeasurementsPosition,
  mayReapplyToolCursor,
} from './view'


beforeEach(() => {
  setZoomWheelStop(2)
  setSmoothCamera(true)
  setShowToolHints(true)
  setShowToolPointer(true)
  setMeasurementsPosition('top-right')
  localStorage.clear()
})

describe('view settings defaults', () => {
  it('match today\'s behavior with nothing stored (fresh module, factory fallbacks)', async () => {
    // The static imports above were already driven through their setters by
    // beforeEach; a fresh module over empty storage shows the real fallbacks.
    localStorage.clear()
    vi.resetModules()
    const fresh = await import('./view')
    expect(fresh.getZoomWheelStop()).toBe(2)
    expect(fresh.getZoomWheelSpeed()).toBe(1)
    expect(fresh.getSmoothCamera()).toBe(true)
    expect(fresh.getShowToolHints()).toBe(true)
    expect(fresh.getShowToolPointer()).toBe(true)
    expect(fresh.getMeasurementsPosition()).toBe('bottom-right')
  })
  it('ignores garbage in storage and falls back', async () => {
    localStorage.setItem('hew.settings.zoomWheelSpeed', '9')
    localStorage.setItem('hew.settings.smoothCamera', 'maybe')
    localStorage.setItem('hew.settings.measurementsPosition', 'left')
    vi.resetModules()
    const fresh = await import('./view')
    expect(fresh.getZoomWheelStop()).toBe(2)
    expect(fresh.getSmoothCamera()).toBe(true)
    expect(fresh.getMeasurementsPosition()).toBe('bottom-right')
  })
  it('restores what was stored', async () => {
    localStorage.setItem('hew.settings.zoomWheelSpeed', '0')
    localStorage.setItem('hew.settings.smoothCamera', 'false')
    vi.resetModules()
    const fresh = await import('./view')
    expect(fresh.getZoomWheelSpeed()).toBe(0.5)
    expect(fresh.getSmoothCamera()).toBe(false)
  })
})

describe('settings-changed from another window (desktop shell)', () => {
  afterEach(() => {
    // @ts-expect-error — jsdom has no such property unless a test set it.
    delete window.__TAURI_INTERNALS__
  })
  it('applies each setting from its own payload key and ignores the rest', async () => {
    // A minimal Tauri runtime: the real event API registers its callbacks
    // through transformCallback, which is where the 'settings-changed'
    // handlers can be driven from.
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
    const fresh = await import('./view')
    await vi.waitFor(() => expect(callbacks).toHaveLength(6))
    const handlers = callbacks
    expect(handlers.length).toBeGreaterThan(0)
    const cb = vi.fn()
    fresh.subscribeShowToolPointer(cb)
    const send = (payload: unknown): void => handlers.forEach((h) => h({ payload }))
    send({ showToolPointer: false })
    expect(fresh.getShowToolPointer()).toBe(false)
    expect(cb).toHaveBeenCalledWith(false)
    send({ zoomWheelSpeed: 4, smoothCamera: false, measurementsPosition: 'bottom-right', showToolHints: false })
    expect(fresh.getZoomWheelStop()).toBe(4)
    expect(fresh.getSmoothCamera()).toBe(false)
    expect(fresh.getMeasurementsPosition()).toBe('bottom-right')
    expect(fresh.getShowToolHints()).toBe(false)
    send({ outlinerSort: 'name' })
    expect(fresh.getOutlinerSort()).toBe('name')
    send({ outlinerSort: 'sideways' })
    expect(fresh.getOutlinerSort()).toBe('name')
    // Another setting's payload, junk values, and non-objects leave it alone.
    const calls = cb.mock.calls.length
    send({ theme: 'dark' })
    send({ showToolPointer: 'garbage' })
    send(null)
    expect(fresh.getShowToolPointer()).toBe(false)
    expect(cb.mock.calls.length).toBe(calls)
  })
})

describe('mayReapplyToolCursor', () => {
  const idle = { shiftPan: false, shiftFov: false, fovDrag: false, eyedropper: false }
  it('repaints the tool cursor when nothing transient is live', () => {
    expect(mayReapplyToolCursor(idle)).toBe(true)
  })
  it.each(['shiftPan', 'shiftFov', 'fovDrag', 'eyedropper'] as const)('holds off while %s is live', (k) => {
    expect(mayReapplyToolCursor({ ...idle, [k]: true })).toBe(false)
  })
})

describe('zoom wheel speed', () => {
  it('has five multiplicative stops with 1 in the middle', () => {
    expect([...ZOOM_WHEEL_FACTORS]).toEqual([0.5, 0.71, 1, 1.41, 2])
  })
  it('scales the wheel dolly step (OrbitControls zoomSpeed) by the chosen stop', () => {
    setZoomWheelStop(0)
    expect(getZoomWheelSpeed()).toBe(0.5)
    setZoomWheelStop(4)
    expect(getZoomWheelSpeed()).toBe(2)
  })
  it('clamps an out-of-range stop to the default', () => {
    expect(zoomWheelFactor(9)).toBe(1)
  })
  it('persists under hew.settings.zoomWheelSpeed', () => {
    setZoomWheelStop(3)
    expect(localStorage.getItem('hew.settings.zoomWheelSpeed')).toBe('3')
  })
})

describe('smooth camera', () => {
  it('animates only when on, with a real duration, and no reduced-motion', () => {
    expect(shouldAnimateCamera(600, false, true)).toBe(true)
    expect(shouldAnimateCamera(600, false, false)).toBe(false)
    expect(shouldAnimateCamera(600, true, true)).toBe(false)
    expect(shouldAnimateCamera(0, false, true)).toBe(false)
  })
  it('turns orbit inertia off with it, and when precise orbit is held', () => {
    expect(inertiaEnabled(false, true)).toBe(true)
    expect(inertiaEnabled(false, false)).toBe(false)
    expect(inertiaEnabled(true, true)).toBe(false)
  })
  it('persists under hew.settings.smoothCamera', () => {
    setSmoothCamera(false)
    expect(localStorage.getItem('hew.settings.smoothCamera')).toBe('false')
  })
})

describe('cursor settings', () => {
  it('plain arrow replaces the tool cursor when the pointer is off', () => {
    const tool = vi.fn(() => 'url(x) 1 1, auto')
    expect(resolveToolCursor(true, tool)).toBe('url(x) 1 1, auto')
    expect(resolveToolCursor(false, tool)).toBe(PLAIN_CURSOR)
    expect(tool).toHaveBeenCalledTimes(1)
  })
  it('persist under their own keys', () => {
    setShowToolHints(false)
    setShowToolPointer(false)
    expect(localStorage.getItem('hew.settings.showToolHints')).toBe('false')
    expect(localStorage.getItem('hew.settings.showToolPointer')).toBe('false')
  })
})

describe('measurements position', () => {
  it('persists and notifies subscribers', () => {
    const cb = vi.fn()
    const off = subscribeMeasurementsPosition(cb)
    setMeasurementsPosition('bottom-right')
    expect(cb).toHaveBeenCalledWith('bottom-right')
    expect(localStorage.getItem('hew.settings.measurementsPosition')).toBe('bottom-right')
    off()
  })
  it('syncs from another tab via the storage event', () => {
    window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.measurementsPosition', newValue: 'bottom-right' }))
    expect(getMeasurementsPosition()).toBe('bottom-right')
    window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.measurementsPosition', newValue: 'garbage' }))
    expect(getMeasurementsPosition()).toBe('bottom-right')
  })
  it('syncs the other settings from the storage event too', () => {
    window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.zoomWheelSpeed', newValue: '4' }))
    expect(getZoomWheelStop()).toBe(4)
    window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.smoothCamera', newValue: 'false' }))
    expect(getSmoothCamera()).toBe(false)
  })
})
