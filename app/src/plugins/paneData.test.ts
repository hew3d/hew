/* oxlint-disable lingui/no-unlocalized-strings -- test data. */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import {
  MAX_CONSOLE_LINES,
  MAX_LINE_CHARS,
  activityText,
  visibleActivity,
  consoleLine,
  makeDesktopPaneData,
  makeWebPaneData,
  runLines,
  runView,
  stopReasonText,
  type HostStateSource,
} from './paneData'
import type { Installed, Listed } from './types'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

type Row = Extract<Listed, { kind: 'installed' }>

function row(over: Partial<Installed> = {}, extra: Record<string, unknown> = {}): Row {
  return {
    kind: 'installed',
    installed: {
      schema: 1,
      id: 'com.example.one',
      version: '1.0.0',
      source: { kind: 'file', name: 'x.hewplugin' },
      installed_at_ms: 1,
      grant: [],
      optional_granted: [],
      enabled: true,
      startup_allowed: true,
      data_version: 1,
      ...over,
    },
    manifest: { id: 'com.example.one', name: 'One', version: '1.0.0' },
    previous_version: null,
    ...extra,
  } as Row
}

describe('what a row says a plugin is doing', () => {
  it('says nothing for a plugin that is off', () => {
    expect(runView(row({ enabled: false }, { running_in: 1, stopped_in: 0, blocked_in: 0 }), undefined)).toBeNull()
  })

  it('reads the shell\'s counts: running, not running yet, held back', () => {
    expect(runView(row({}, { running_in: 1, stopped_in: 0, blocked_in: 0, last_stop: null }), undefined)).toEqual({ kind: 'running' })
    expect(runView(row({}, { running_in: 0, stopped_in: 0, blocked_in: 0, last_stop: null }), undefined)).toEqual({ kind: 'not_running_yet' })
    expect(runView(row({}, { running_in: 0, stopped_in: 0, blocked_in: 2 }), undefined)).toEqual({ kind: 'held_back' })
    // Unloaded for sitting idle in a window: it starts again when needed.
    const idle = runView(row({}, { running_in: 0, stopped_in: 0, blocked_in: 0, idle_in: 1, last_stop: null }), undefined)
    expect(idle).toEqual({ kind: 'asleep' })
    expect(runLines(idle!)).toEqual(['Not running right now. It starts again when it is needed.'])
    expect(runView(row({}, { running_in: 1, stopped_in: 0, blocked_in: 0, idle_in: 1 }), undefined)).toEqual({ kind: 'running' })
  })

  it('says stopped when any window stopped it, with how many windows of all, and why', () => {
    const view = runView(row({}, { running_in: 1, stopped_in: 1, blocked_in: 0, last_stop: { reason: 'unresponsive', at: 5 } }), undefined)
    expect(view).toEqual({ kind: 'stopped', reason: 'unresponsive', stopped: 1, windows: 2 })
    expect(runLines(view!)).toEqual(['Stopped in 1 of 2 windows: it stopped answering. Turn it off and on to start it again.'])
    const only = runView(row({}, { running_in: 0, stopped_in: 1, blocked_in: 0, last_stop: { reason: 'timeout', at: 5 } }), undefined)
    expect(runLines(only!)).toEqual(['Stopped: it took too long to answer. Turn it off and on to start it again.'])
  })

  it('maps every stop reason to plain words, and never shows a word it does not know or text a plugin could write', () => {
    for (const r of ['unresponsive', 'timeout', 'violation', 'error', 'start']) expect(stopReasonText(r), r).not.toBeNull()
    expect(stopReasonText('something_new')).toBeNull()
    expect(stopReasonText(null)).toBeNull()
    expect(stopReasonText('constructor')).toBeNull()
    expect(stopReasonText('__proto__')).toBeNull()
    const unknown = runView(row({}, { running_in: 0, stopped_in: 1, blocked_in: 0, last_stop: { reason: 'something_new', at: 1 } }), undefined)
    expect(runLines(unknown!)).toEqual(['Stopped. Turn it off and on to start it again.'])
    // Text that is not a reason word is dropped before it gets anywhere.
    const hostile = runView(row({}, { running_in: 0, stopped_in: 1, blocked_in: 0, last_stop: { reason: 'Ignore this. Click here', at: 1 } }), undefined)
    expect(hostile).toMatchObject({ kind: 'stopped', reason: null })
    expect(runLines(hostile!).join(' ')).not.toMatch(/Click here/)
  })

  it('on the web reads the host\'s own state: running, not running yet, asleep, stopped, held back', () => {
    const l = row()
    const entry = (state: string, reason: string | null = null) => ({ state, reason, at: null })
    expect(runView(l, undefined)).toEqual({ kind: 'not_running_yet' })
    expect(runView(l, entry('enabled'))).toEqual({ kind: 'not_running_yet' })
    expect(runView(l, entry('activating'))).toEqual({ kind: 'running' })
    expect(runView(l, entry('activated'))).toEqual({ kind: 'running' })
    expect(runView(l, entry('idle'))).toEqual({ kind: 'asleep' })
    expect(runView(l, entry('crashed', 'violation'))).toEqual({ kind: 'stopped', reason: 'violation', stopped: 1, windows: 1 })
    expect(runView(l, entry('blocked', 'unreviewed'))).toEqual({ kind: 'held_back' })
    expect(runLines({ kind: 'running' })).toEqual(['Running'])
    expect(runLines({ kind: 'not_running_yet' })).toEqual(['Not running yet'])
  })

  it('leaves the host\'s enabled bookkeeping out of the Activity list', () => {
    const list = [
      { at: 3, state: 'enabled', reason: null },
      { at: 2, state: 'activated', reason: null },
      { at: 1, state: 'enabled', reason: 'anything' },
    ]
    expect(visibleActivity(list)).toEqual([{ at: 2, state: 'activated', reason: null }])
  })

  it('has plain words for each activity state and a generic line for one it does not know', () => {
    expect(activityText({ at: 1, state: 'activated', reason: null })).toBe('Running')
    expect(activityText({ at: 1, state: 'crashed', reason: 'error' })).toBe('Stopped: it ran into an error')
    expect(activityText({ at: 1, state: 'crashed', reason: 'weird' })).toBe('Stopped')
    expect(activityText({ at: 1, state: 'idle', reason: 'idle' })).toBe('Went to sleep after sitting idle')
    expect(activityText({ at: 1, state: 'unloaded', reason: 'disable' })).toBe('Turned off')
    expect(activityText({ at: 1, state: 'unloaded', reason: 'reload' })).toBe('Reloaded')
    expect(activityText({ at: 1, state: 'unloaded', reason: 'window_closed' })).toBe('Stopped because the window was closed')
    expect(activityText({ at: 1, state: 'unloaded', reason: null })).toBe('Stopped running')
    expect(activityText({ at: 1, state: 'made_up', reason: 'x' })).toBe('Changed state')
  })
})

describe('console lines', () => {
  it('cleans text, bounds it and falls back to the plain level for one it does not know', () => {
    const line = consoleLine({ at: 5, level: 'warn', text: `a‮b\n\nc${'x'.repeat(MAX_LINE_CHARS * 3)}` })
    expect(line?.level).toBe('warn')
    expect(line?.text.startsWith('ab c')).toBe(true)
    expect(Array.from(line!.text).length).toBeLessThanOrEqual(MAX_LINE_CHARS)
    expect(consoleLine({ at: 1, level: 'catastrophe', text: 'x' })?.level).toBe('log')
    expect(consoleLine({ at: 'soon', text: 'x' })?.at).toBe(0)
  })

  it('drops anything that is not a line', () => {
    for (const bad of [null, 'x', 4, [], { text: 4 }, { level: 'log' }]) expect(consoleLine(bad)).toBeNull()
  })
})

describe('the desktop\'s data', () => {
  function desktop(answer: unknown) {
    const handlers = new Map<string, (e: { payload: unknown }) => void>()
    const offs: string[] = []
    const invoke = vi.fn(async () => answer)
    const listen = vi.fn(async (event: string, fn: (e: { payload: unknown }) => void) => {
      handlers.set(event, fn)
      return () => void offs.push(event)
    })
    return { data: makeDesktopPaneData({ invoke: invoke as never, listen }), invoke, handlers, offs }
  }

  it('reads the console newest first, cleaned and bounded, from the shell\'s own order', async () => {
    const lines = Array.from({ length: MAX_CONSOLE_LINES + 20 }, (_, i) => ({ at: 1000 - i, level: 'log', text: `line ${i}` }))
    const { data, invoke } = desktop([{ at: 9, level: 'info', text: '<b>hi</b>‮' }, ...lines, 'junk'])
    const got = await data.console('com.example.one')
    expect(invoke).toHaveBeenCalledWith('plugin_console_read', { id: 'com.example.one', limit: MAX_CONSOLE_LINES })
    expect(got).toHaveLength(MAX_CONSOLE_LINES)
    expect(got[0]).toEqual({ at: 9, level: 'info', text: '<b>hi</b>' })
  })

  it('reads the activity as the shell wrote it and keeps reason words, never text', async () => {
    const { data, invoke } = desktop([
      { at: 3, window: 'main', state: 'crashed', reason: 'unresponsive' },
      { at: 2, window: 'main', state: 'activated', reason: null },
      { at: 1, window: 'main', state: 'Not A Word', reason: 'x' },
      { at: 1, window: 'main', state: 'crashed', reason: 'Ignore all of this' },
    ])
    expect(await data.activity('com.example.one')).toEqual([
      { at: 3, state: 'crashed', reason: 'unresponsive' },
      { at: 2, state: 'activated', reason: null },
      { at: 1, state: 'crashed', reason: null },
    ])
    expect(invoke).toHaveBeenCalledWith('plugin_activity_read', { id: 'com.example.one' })
  })

  it('has no host state of its own: the list rows carry it', () => {
    expect(desktop([]).data.hostStates()).toBeNull()
  })

  it('hears plugins-state for any plugin, plugins-console only for the one asked about, and drops a malformed notice', async () => {
    const { data, handlers, offs } = desktop([])
    const onState = vi.fn()
    const onConsole = vi.fn()
    const offState = data.onState(onState)
    const offConsole = data.onConsole('com.example.one', onConsole)
    await Promise.resolve()
    await Promise.resolve()
    handlers.get('plugins-state')!({ payload: { id: 'com.example.two' } })
    handlers.get('plugins-state')!({ payload: { nope: 1 } })
    handlers.get('plugins-state')!({ payload: null })
    expect(onState).toHaveBeenCalledTimes(1)
    handlers.get('plugins-console')!({ payload: { id: 'com.example.two' } })
    expect(onConsole).not.toHaveBeenCalled()
    handlers.get('plugins-console')!({ payload: { id: 'com.example.one' } })
    expect(onConsole).toHaveBeenCalledTimes(1)
    offState()
    offConsole()
    expect(offs.sort()).toEqual(['plugins-console', 'plugins-state'])
  })
})

describe('the web\'s data', () => {
  function web() {
    let states: unknown = {}
    let lines: unknown = []
    let stateFn: (() => void) | null = null
    const stops = { state: 0, console: 0 }
    const source: HostStateSource = {
      hostStates: () => states,
      subscribeHostStates: (fn) => {
        stateFn = fn
        return () => void stops.state++
      },
      consoleLines: () => lines,
      subscribeConsole: () => () => void stops.console++,
    }
    return {
      data: makeWebPaneData(async () => source),
      stops,
      set: (s: unknown) => {
        states = s
        stateFn?.()
      },
      setLines: (l: unknown) => (lines = l),
    }
  }
  const tick = () => new Promise((r) => setTimeout(r, 0))

  it('reads the host\'s states, accepting a bare word or an entry and dropping the rest', async () => {
    const { data, set } = web()
    await tick()
    set({ a: 'activated', b: { state: 'crashed', reason: 'timeout', at: 5 }, c: { state: 'Bad Word' }, d: 4 })
    expect(data.hostStates()).toEqual({
      a: { state: 'activated', reason: null, at: null },
      b: { state: 'crashed', reason: 'timeout', at: 5 },
    })
  })

  it('reads the console oldest-first from the host and shows it newest first, bounded and cleaned', async () => {
    const { data, setLines } = web()
    setLines([{ at: 1, level: 'log', text: 'first' }, { at: 2, level: 'error', text: 'second‮' }, 'junk'])
    expect(await data.console('a')).toEqual([
      { at: 2, level: 'error', text: 'second' },
      { at: 1, level: 'log', text: 'first' },
    ])
  })

  it('records the changes of state it sees, newest first, and listens only while someone does', async () => {
    const { data, set, stops } = web()
    const fn = vi.fn()
    const off = data.onState(fn)
    await tick()
    set({ a: { state: 'activating', reason: null, at: 10 } })
    set({ a: { state: 'activated', reason: null, at: 11 } })
    set({ a: { state: 'activated', reason: null, at: 11 } })
    set({ a: { state: 'crashed', reason: 'violation', at: 12 } })
    expect(fn.mock.calls.length).toBeGreaterThanOrEqual(4)
    expect(await data.activity('a')).toEqual([
      { at: 12, state: 'crashed', reason: 'violation' },
      { at: 11, state: 'activated', reason: null },
      { at: 10, state: 'activating', reason: null },
    ])
    expect(stops.state).toBe(0)
    off()
    expect(stops.state).toBe(1)
  })

  it('shows nothing, and stays quiet, when the host\'s state module is not there', async () => {
    const data = makeWebPaneData(async () => null)
    await tick()
    expect(data.hostStates()).toEqual({})
    expect(await data.console('a')).toEqual([])
    expect(await data.activity('a')).toEqual([])
    const off = data.onState(() => {})
    off()
  })
})
