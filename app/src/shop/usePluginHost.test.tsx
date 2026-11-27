// @vitest-environment jsdom
import { renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'

const start = vi.fn(() => () => undefined)
vi.mock('../preview/loaderRoots', () => ({ loadPluginHost: () => Promise.resolve(start) }))
vi.mock('../preview', () => ({ loaderEnabled: () => true }))
vi.mock('../api/eventPump', () => ({ pumpEvents: vi.fn() }))

import { useShopPluginHost } from './usePluginHost'

function scene() {
  const events: string[] = []
  return {
    events,
    scene: {
      note_document_event: vi.fn((kind: string, name: string) => void events.push(`${kind}:${name}`)),
    } as never,
  }
}

const pagehide = (persisted: boolean): void => {
  const e = new Event('pagehide') as Event & { persisted: boolean }
  Object.defineProperty(e, 'persisted', { value: persisted })
  window.dispatchEvent(e)
}

describe('the Shop plugin host lifecycle', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
  afterEach(() => vi.clearAllMocks())

  async function mounted() {
    const s = scene()
    const hook = renderHook(() =>
      useShopPluginHost({ scene: s.scene, bridge: {} as never, toast: () => undefined, title: () => 'T', harness: () => null }),
    )
    await vi.waitFor(() => expect(start).toHaveBeenCalled())
    return { ...s, hook }
  }

  it('closes the first model before opening a second in place', async () => {
    const { events, hook, scene: sc } = await mounted()
    hook.result.current.noteOpened(sc, 'a.hew')
    hook.result.current.noteOpened(sc, 'b.hew')
    expect(events).toEqual(['opened:a.hew', 'closed:a.hew', 'opened:b.hew'])
  })

  it('sends a close on pagehide only when the page is not kept for the back/forward cache', async () => {
    const { events, hook, scene: sc } = await mounted()
    hook.result.current.noteOpened(sc, 'a.hew')
    pagehide(true)
    expect(events).toEqual(['opened:a.hew'])
    pagehide(false)
    expect(events).toEqual(['opened:a.hew', 'closed:T'])
    // Nothing is open now: a later open does not close one again.
    hook.result.current.noteOpened(sc, 'b.hew')
    expect(events).toEqual(['opened:a.hew', 'closed:T', 'opened:b.hew'])
  })

  it('sends no close on pagehide when no document was ever announced, and only one when one was', async () => {
    const { events, hook, scene: sc } = await mounted()
    pagehide(false)
    expect(events).toEqual([])
    hook.result.current.noteOpened(sc, 'a.hew')
    pagehide(false)
    pagehide(false)
    expect(events).toEqual(['opened:a.hew', 'closed:T'])
  })
})
