import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import * as Diag from './diagnosticLog'
import * as inputRecorder from '../recording/inputRecorder'
import { generateBugReport, setStoreForTest, resetForTest, type ReportableScene } from './reportBug'
import type { ReproducerStore } from '../io/reproducerStore'

function fakeScene(overrides: Partial<ReportableScene> = {}): ReportableScene {
  return {
    save: () => new Uint8Array([1, 2, 3]),
    state_hash: () => 123n,
    ...overrides,
  }
}

function fakeStore(): ReproducerStore & { calls: Array<{ name: string; json: string }> } {
  const calls: Array<{ name: string; json: string }> = []
  return {
    calls,
    write: async (name: string, json: string) => {
      calls.push({ name, json })
      return `/fake/path/${name}`
    },
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
  inputRecorder.stop()
  inputRecorder.take() // clear any residue
})

afterEach(() => {
  inputRecorder.stop()
  inputRecorder.take()
})

describe('generateBugReport', () => {
  it('writes a bundle with the expected manifest fields and populated hew/input/log', async () => {
    const scene = fakeScene()
    const store = fakeStore()
    setStoreForTest(store)

    Diag.logUi('test', 'INFO', { msg: 'hello-from-log' })

    inputRecorder.start()
    inputRecorder.recordPointer('pointerdown', 1, 2, {
      button: 0,
      buttons: 1,
      shiftKey: false,
      altKey: false,
      ctrlKey: false,
      metaKey: false,
    } as PointerEvent)

    const result = await generateBugReport(scene, 'user-report')

    expect(result).toEqual({ ok: true, path: `/fake/path/${store.calls[0]?.name}` })
    expect(store.calls).toHaveLength(1)
    expect(store.calls[0].name).toMatch(/^bug-report-.*\.json$/)

    const bundle = JSON.parse(store.calls[0].json)
    expect(bundle.manifest.reason).toBe('user-report')
    expect(typeof bundle.manifest.ts).toBe('number')
    expect(typeof bundle.manifest.appVersion).toBe('string')
    expect(bundle.manifest.stateHash).toBe('123')
    expect(typeof bundle.manifest.userAgent).toBe('string')
    expect(typeof bundle.manifest.os).toBe('string')
    expect(typeof bundle.manifest.gpu).toBe('string')

    expect(bundle.hew).toBe(bytesToBase64(new Uint8Array([1, 2, 3])))
    const decoded = Uint8Array.from(atob(bundle.hew), (c) => c.charCodeAt(0))
    expect(Array.from(decoded)).toEqual([1, 2, 3])

    expect(bundle.log).toContain('hello-from-log')

    expect(bundle.input).toHaveLength(1)
    expect(bundle.input[0].kind).toBe('pointerdown')

    // peek() must not have cleared the recorder's buffer.
    expect(inputRecorder.peek()).toHaveLength(1)
  })

  it('does not disrupt an ongoing recording (peek, not take)', async () => {
    const scene = fakeScene()
    setStoreForTest(fakeStore())

    inputRecorder.start()
    inputRecorder.recordPointer('pointerdown', 0, 0, {
      button: 0,
      buttons: 1,
      shiftKey: false,
      altKey: false,
      ctrlKey: false,
      metaKey: false,
    } as PointerEvent)

    await generateBugReport(scene)

    expect(inputRecorder.isActive()).toBe(true)
    expect(inputRecorder.take()).toHaveLength(1)
  })

  it('still produces a bundle (hew null) when the scene throws on every method', async () => {
    const scene = fakeScene({
      save: () => {
        throw new Error('save boom')
      },
      state_hash: () => {
        throw new Error('state_hash boom')
      },
    })
    const store = fakeStore()
    setStoreForTest(store)

    await expect(generateBugReport(scene, 'throwing-scene')).resolves.not.toThrow()

    const bundle = JSON.parse(store.calls[0].json)
    expect(bundle.hew).toBeNull()
    expect(bundle.manifest.stateHash).toBe('0')
  })

  it('reports failure (never throws) when the store write throws', async () => {
    const scene = fakeScene()
    setStoreForTest({
      write: async () => {
        throw new Error('disk full')
      },
    })

    await expect(generateBugReport(scene)).resolves.toEqual({ ok: false, path: null })
  })

  it('reports a web download (ok, no path) when the store returns null', async () => {
    const scene = fakeScene()
    setStoreForTest({
      write: async () => null, // WebReproducerStore downloads and resolves null
    })

    await expect(generateBugReport(scene)).resolves.toEqual({ ok: true, path: null })
  })

  it('defaults the reason to "user-report"', async () => {
    const scene = fakeScene()
    const store = fakeStore()
    setStoreForTest(store)

    await generateBugReport(scene)

    const bundle = JSON.parse(store.calls[0].json)
    expect(bundle.manifest.reason).toBe('user-report')
  })
})
