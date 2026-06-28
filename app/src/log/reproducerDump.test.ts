import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest'
import * as Diag from './diagnosticLog'
import {
  registerScene,
  dumpReproducer,
  installFailureHandlers,
  resetForTest,
  setStoreForTest,
  type RecordableScene,
} from './reproducerDump'
import type { ReproducerStore } from '../io/reproducerStore'

function fakeScene(overrides: Partial<RecordableScene> = {}): RecordableScene {
  return {
    start_recording: vi.fn(),
    take_recording: vi.fn(() => '{"version":2,"calls":[],"golden_hash":0}'),
    save: vi.fn(() => new Uint8Array([1, 2, 3])),
    state_hash: vi.fn(() => 123n),
    ...overrides,
  }
}

function fakeStore(): ReproducerStore & { calls: Array<{ name: string; json: string }> } {
  const calls: Array<{ name: string; json: string }> = []
  return {
    calls,
    write: vi.fn(async (name: string, json: string) => {
      calls.push({ name, json })
      return `/fake/path/${name}`
    }),
  }
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary)
}

beforeEach(() => {
  resetForTest()
  Diag.clear()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('registerScene', () => {
  it('starts recording on the scene', () => {
    const scene = fakeScene()
    registerScene(scene)
    expect(scene.start_recording).toHaveBeenCalledOnce()
  })

  it('does not throw if start_recording throws', () => {
    const scene = fakeScene({
      start_recording: vi.fn(() => {
        throw new Error('boom')
      }),
    })
    expect(() => registerScene(scene)).not.toThrow()
  })
})

describe('dumpReproducer', () => {
  it('assembles a bundle with manifest, recording, log, and hew', async () => {
    const scene = fakeScene()
    registerScene(scene)
    const store = fakeStore()
    setStoreForTest(store)

    Diag.logUi('test', 'INFO', { msg: 'hello' })

    const path = await dumpReproducer('manual-test')

    expect(path).toBe(store.calls[0] ? `/fake/path/${store.calls[0].name}` : null)
    expect(store.write).toHaveBeenCalledOnce()
    expect(store.calls).toHaveLength(1)
    expect(store.calls[0].name).toMatch(/^reproducer-.*\.json$/)

    const bundle = JSON.parse(store.calls[0].json)
    expect(bundle.manifest.reason).toBe('manual-test')
    expect(bundle.manifest.stateHash).toBe('123')
    expect(typeof bundle.manifest.ts).toBe('number')
    expect(typeof bundle.manifest.appVersion).toBe('string')
    expect(typeof bundle.manifest.userAgent).toBe('string')

    expect(bundle.recording).toBe('{"version":2,"calls":[],"golden_hash":0}')
    expect(bundle.log).toContain('hello')

    expect(bundle.hew).toBe(bytesToBase64(new Uint8Array([1, 2, 3])))
    const decoded = Uint8Array.from(atob(bundle.hew), (c) => c.charCodeAt(0))
    expect(Array.from(decoded)).toEqual([1, 2, 3])
  })

  it('handles a large save() payload via chunked base64 encoding', async () => {
    const big = new Uint8Array(200_000)
    for (let i = 0; i < big.length; i++) big[i] = i % 256
    const scene = fakeScene({ save: vi.fn(() => big) })
    registerScene(scene)
    const store = fakeStore()
    setStoreForTest(store)

    await dumpReproducer('large-payload')

    const bundle = JSON.parse(store.calls[0].json)
    const decoded = Uint8Array.from(atob(bundle.hew), (c) => c.charCodeAt(0))
    expect(decoded.length).toBe(big.length)
    expect(Array.from(decoded.slice(0, 5))).toEqual(Array.from(big.slice(0, 5)))
    expect(Array.from(decoded.slice(-5))).toEqual(Array.from(big.slice(-5)))
  })

  it('still produces a bundle (recording/hew null) when no scene is registered', async () => {
    const store = fakeStore()
    setStoreForTest(store)

    const path = await dumpReproducer('no-scene')

    expect(path).not.toBeNull()
    const bundle = JSON.parse(store.calls[0].json)
    expect(bundle.recording).toBeNull()
    expect(bundle.hew).toBeNull()
    expect(bundle.manifest.stateHash).toBe('0')
  })

  it('does not propagate when the scene throws on every method', async () => {
    const scene = fakeScene({
      take_recording: vi.fn(() => {
        throw new Error('take_recording boom')
      }),
      save: vi.fn(() => {
        throw new Error('save boom')
      }),
      state_hash: vi.fn(() => {
        throw new Error('state_hash boom')
      }),
    })
    registerScene(scene)
    const store = fakeStore()
    setStoreForTest(store)

    await expect(dumpReproducer('throwing-scene')).resolves.not.toThrow()
    const bundle = JSON.parse(store.calls[0].json)
    expect(bundle.recording).toBeNull()
    expect(bundle.hew).toBeNull()
    expect(bundle.manifest.stateHash).toBe('0')
  })

  it('does not propagate when the store write throws', async () => {
    registerScene(fakeScene())
    setStoreForTest({
      write: vi.fn(async () => {
        throw new Error('disk full')
      }),
    })

    await expect(dumpReproducer('store-throws')).resolves.toBeNull()
  })

  it('rate-limits: a second dump within the window is suppressed', async () => {
    registerScene(fakeScene())
    const store = fakeStore()
    setStoreForTest(store)

    const first = await dumpReproducer('first')
    const second = await dumpReproducer('second')

    expect(first).not.toBeNull()
    expect(second).toBeNull()
    expect(store.write).toHaveBeenCalledOnce()
  })

  it('allows a new dump after the rate-limit window elapses', async () => {
    vi.useFakeTimers()
    registerScene(fakeScene())
    const store = fakeStore()
    setStoreForTest(store)

    const first = await dumpReproducer('first')
    vi.advanceTimersByTime(5_001)
    const second = await dumpReproducer('second')

    expect(first).not.toBeNull()
    expect(second).not.toBeNull()
    expect(store.write).toHaveBeenCalledTimes(2)
  })

  it('guards re-entrancy: a dump triggered while one is in flight is suppressed', async () => {
    registerScene(fakeScene())
    let resolveWrite!: (v: string) => void
    const writeMock = vi.fn(
      () => new Promise<string>((resolve) => (resolveWrite = resolve)),
    )
    setStoreForTest({ write: writeMock })

    const inFlight = dumpReproducer('first')
    const reentrant = await dumpReproducer('second')

    expect(reentrant).toBeNull()
    resolveWrite('/fake/path/done')
    await inFlight
    expect(writeMock).toHaveBeenCalledOnce()
  })
})

// This project's vitest environment is 'node' (no jsdom/happy-dom dep, and
// adding one is out of footprint), so there's no real `window` global.
// installFailureHandlers() guards on `typeof window === 'undefined'` and
// no-ops in that case (verified separately below); to exercise the
// listener-attaching path itself, stub a minimal EventTarget as `window` —
// Node's built-in EventTarget/Event classes are sufficient (DOM-only globals
// like ErrorEvent are not available here; tests synthesize equivalents).
function withFakeWindow<T>(fn: (fakeWindow: EventTarget) => T): T {
  const fakeWindow = new EventTarget()
  const original = (globalThis as { window?: unknown }).window
  ;(globalThis as { window?: unknown }).window = fakeWindow
  try {
    return fn(fakeWindow)
  } finally {
    ;(globalThis as { window?: unknown }).window = original
  }
}

describe('installFailureHandlers', () => {
  it('no-ops when window is undefined (this suite\'s default node environment)', () => {
    expect(typeof window).toBe('undefined')
    expect(() => installFailureHandlers()).not.toThrow()
  })

  it('is idempotent (installing twice still only attaches once)', () => {
    withFakeWindow((fakeWindow) => {
      const addSpy = vi.spyOn(fakeWindow, 'addEventListener')
      installFailureHandlers()
      const countAfterFirst = addSpy.mock.calls.length
      expect(countAfterFirst).toBeGreaterThan(0)
      installFailureHandlers()
      expect(addSpy.mock.calls.length).toBe(countAfterFirst)
    })
  })

  it('triggers a dump on a window error event', async () => {
    await withFakeWindow(async (fakeWindow) => {
      installFailureHandlers()
      registerScene(fakeScene())
      const store = fakeStore()
      setStoreForTest(store)

      // `ErrorEvent` is a DOM global, not a Node one (this suite runs in the
      // 'node' environment), so synthesize an equivalent plain Event carrying
      // the `.error`/`.message` fields the handler reads — same approach the
      // unhandledrejection test below uses for its `.reason` field.
      const errorEvent = Object.assign(new Event('error'), {
        error: new Error('boom'),
        message: 'boom',
      })
      fakeWindow.dispatchEvent(errorEvent)

      // dumpReproducer is fire-and-forget (void) inside the handler; flush microtasks.
      await new Promise((r) => setTimeout(r, 0))

      expect(store.write).toHaveBeenCalledOnce()
      const bundle = JSON.parse(store.calls[0].json)
      expect(bundle.manifest.reason).toContain('uncaught-error')
      expect(bundle.manifest.reason).toContain('boom')
    })
  })

  it('triggers a dump on an unhandledrejection event', async () => {
    await withFakeWindow(async (fakeWindow) => {
      installFailureHandlers()
      registerScene(fakeScene())
      const store = fakeStore()
      setStoreForTest(store)

      const rejectionEvent = new Event('unhandledrejection') as Event & {
        reason: unknown
        promise: Promise<unknown>
      }
      Object.defineProperty(rejectionEvent, 'reason', { value: new Error('async-boom') })
      Object.defineProperty(rejectionEvent, 'promise', { value: Promise.reject().catch(() => {}) })
      fakeWindow.dispatchEvent(rejectionEvent)

      await new Promise((r) => setTimeout(r, 0))

      expect(store.write).toHaveBeenCalledOnce()
      const bundle = JSON.parse(store.calls[0].json)
      expect(bundle.manifest.reason).toContain('unhandledrejection')
      expect(bundle.manifest.reason).toContain('async-boom')
    })
  })
})
