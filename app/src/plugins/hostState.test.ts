/**
 * The page's record of what its plugin host reported, which the Plugins
 * pane reads on the web.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { activity, consoleLines, hostStates, MAX_ACTIVITY, MAX_CONSOLE_LINES, noteConsole, noteHostState, resetHostState, subscribeConsole, subscribeHostStates } from './hostState'

describe('the host state record', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
  afterEach(() => resetHostState())

  it('keeps the last state, every change oldest first up to its bound, and tells listeners', () => {
    const fn = vi.fn()
    const stop = subscribeHostStates(fn)
    const before = hostStates()
    noteHostState('com.example.a', 'activating', null, 1)
    noteHostState('com.example.a', 'activated', null, 2)
    expect(hostStates()).not.toBe(before)
    expect(hostStates()['com.example.a']).toEqual({ state: 'activated', reason: null, at: 2 })
    expect(activity('com.example.a').map((e) => e.state)).toEqual(['activating', 'activated'])
    expect(fn).toHaveBeenCalledTimes(2)
    stop()
    for (let i = 0; i < MAX_ACTIVITY + 10; i++) noteHostState('com.example.a', i % 2 === 0 ? 'idle' : 'activated', null, 10 + i)
    expect(activity('com.example.a')).toHaveLength(MAX_ACTIVITY)
    expect(activity('com.example.a').at(-1)?.at).toBe(10 + MAX_ACTIVITY + 9)
    expect(activity('com.example.b')).toEqual([])
    expect(fn).toHaveBeenCalledTimes(2)
  })

  it('keeps the last 500 console lines per plugin and tells that plugin\'s listeners only', () => {
    const a = vi.fn()
    const b = vi.fn()
    subscribeConsole('com.example.a', a)
    subscribeConsole('com.example.b', b)
    noteConsole('com.example.a', Array.from({ length: MAX_CONSOLE_LINES + 3 }, (_, i) => ({ level: 'info' as const, text: `l${i}`, at: i })))
    expect(consoleLines('com.example.a')).toHaveLength(MAX_CONSOLE_LINES)
    expect(consoleLines('com.example.a')[0].text).toBe('l3')
    expect(a).toHaveBeenCalledTimes(1)
    expect(b).not.toHaveBeenCalled()
  })
})
