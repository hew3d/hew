// @vitest-environment jsdom
/**
 * `startPluginHost`'s wiring: where its test hooks go. Shop Mode passes its
 * own harness object; a document window uses `window.__hew_test`.
 */

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import type { LiveBridgeDeps } from '../api/liveBridge'
import { PluginError } from './pluginError'

// The host module pulls in the kernel module and the React pieces it
// mounts: loaded once, with room for a busy machine.
let startPluginHost: typeof import('./host').startPluginHost
let shellRecorder: typeof import('./host').shellRecorder
beforeAll(async () => {
  ;({ startPluginHost, shellRecorder } = await import('./host'))
}, 60_000)

describe('startPluginHost', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
  afterEach(() => {
    delete (window as { __hew_test?: unknown }).__hew_test
  })

  const bridge: LiveBridgeDeps = { getScene: () => null, getViewportApi: () => null, reconcile: vi.fn() }

  it('puts its hooks on the harness it is given (Shop), not on __hew_test, and takes them away when it stops', async () => {
    const window_ = { marker: true } as Record<string, unknown>
    ;(window as { __hew_test?: unknown }).__hew_test = window_
    const shop: Record<string, unknown> = {}
    const stop = startPluginHost({ bridge, toast: () => {}, platform: 'shop', harness: shop })
    try {
      const hooks = shop.plugins as { states(): unknown; activate(id: string): void; listening(): string[] }
      expect(typeof hooks.states).toBe('function')
      expect(typeof hooks.activate).toBe('function')
      expect(hooks.listening()).toEqual([])
      expect(window_.plugins).toBeUndefined()
    } finally {
      stop()
    }
    expect(shop.plugins).toBeUndefined()
  })

  it('in a document window uses __hew_test', async () => {
    const harness: Record<string, unknown> = {}
    ;(window as { __hew_test?: unknown }).__hew_test = harness
    const stop = startPluginHost({ bridge, toast: () => {} })
    try {
      expect(typeof (harness.plugins as { states?: unknown }).states).toBe('function')
    } finally {
      stop()
    }
  })
})

describe("the shell's record of a plugin's states", () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
  afterEach(() => vi.restoreAllMocks())

  it('stamps each report with a seq that only grows, within a page and across a reload, and rejects when the shell refuses', async () => {
    const reports: Record<string, unknown>[] = []
    let refuse = false
    const invoke = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
      if (command === 'plugin_report_state') reports.push(args ?? {})
      if (refuse) throw new PluginError('report_too_often', 'too often')
      return null as T
    }
    const now = vi.spyOn(Date, 'now').mockReturnValue(1_000)
    const page = shellRecorder(invoke)
    await page.report('com.example.a', 'activating', null)
    await page.report('com.example.a', 'activated', null)
    // The clock went back: the seq still grows.
    now.mockReturnValue(500)
    await page.report('com.example.a', 'idle', 'idle')
    expect(reports.map((r) => r.seq)).toEqual([1_000, 1_001, 1_002])
    expect(reports[2]).toEqual({ id: 'com.example.a', state: 'idle', reason: 'idle', seq: 1_002 })
    // The page reloaded: a new recorder, later on the clock.
    now.mockReturnValue(5_000)
    const reloaded = shellRecorder(invoke)
    await reloaded.report('com.example.a', 'enabled', null)
    expect(reports.at(-1)).toEqual({ id: 'com.example.a', state: 'enabled', seq: 5_000 })
    refuse = true
    await expect(reloaded.report('com.example.a', 'crashed', 'error')).rejects.toMatchObject({ refusal: 'report_too_often' })
  })
})
