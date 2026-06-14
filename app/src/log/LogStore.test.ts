import { describe, it, expect, beforeEach } from 'vitest'
import * as LogStore from './LogStore'

// Reset module state between tests by clearing and resetting via the public API
beforeEach(() => {
  LogStore.clear()
})

describe('LogStore', () => {
  it('starts empty after clear()', () => {
    expect(LogStore.getEntries()).toHaveLength(0)
  })

  it('appends entries in order', () => {
    LogStore.append('info', 'test', 'first')
    LogStore.append('warn', 'test', 'second')
    LogStore.append('error', 'test', 'third')

    const entries = LogStore.getEntries()
    expect(entries).toHaveLength(3)
    expect(entries[0].message).toBe('first')
    expect(entries[0].level).toBe('info')
    expect(entries[1].message).toBe('second')
    expect(entries[1].level).toBe('warn')
    expect(entries[2].message).toBe('third')
    expect(entries[2].level).toBe('error')
  })

  it('assigns unique monotone IDs', () => {
    LogStore.append('info', 's', 'a')
    LogStore.append('info', 's', 'b')
    const entries = LogStore.getEntries()
    expect(entries[1].id).toBeGreaterThan(entries[0].id)
  })

  it('entries have a timestamp (Date)', () => {
    LogStore.append('info', 's', 'msg')
    const entry = LogStore.getEntries()[0]
    expect(entry.timestamp).toBeInstanceOf(Date)
  })

  it('subscribe() fires immediately with current snapshot', () => {
    LogStore.append('info', 's', 'pre-existing')
    const received: number[] = []
    const unsub = LogStore.subscribe((entries) => {
      received.push(entries.length)
    })
    expect(received).toHaveLength(1)
    expect(received[0]).toBe(1)
    unsub()
  })

  it('subscribe() fires on each new append', () => {
    const counts: number[] = []
    const unsub = LogStore.subscribe((entries) => {
      counts.push(entries.length)
    })
    // Initial delivery
    expect(counts).toEqual([0])

    LogStore.append('info', 's', 'a')
    LogStore.append('warn', 's', 'b')
    expect(counts).toEqual([0, 1, 2])
    unsub()
  })

  it('unsubscribe() stops further notifications', () => {
    const counts: number[] = []
    const unsub = LogStore.subscribe((entries) => {
      counts.push(entries.length)
    })
    unsub()
    LogStore.append('info', 's', 'after unsub')
    // Only the initial call, nothing after unsubscribe
    expect(counts).toEqual([0])
  })

  it('clear() notifies subscribers with empty array', () => {
    LogStore.append('error', 's', 'x')
    const snapshots: number[] = []
    const unsub = LogStore.subscribe((e) => snapshots.push(e.length))
    // initial delivery of 1 entry
    expect(snapshots).toEqual([1])
    LogStore.clear()
    expect(snapshots).toEqual([1, 0])
    unsub()
  })

  it('caps entries at MAX_ENTRIES (500)', () => {
    for (let i = 0; i < 510; i++) {
      LogStore.append('info', 's', `msg ${i}`)
    }
    expect(LogStore.getEntries()).toHaveLength(500)
    // Should keep the most recent 500
    expect(LogStore.getEntries()[499].message).toBe('msg 509')
  })

  it('log convenience helpers work', () => {
    LogStore.log.info('src', 'info msg')
    LogStore.log.warn('src', 'warn msg')
    LogStore.log.error('src', 'err msg')
    const entries = LogStore.getEntries()
    expect(entries[0].level).toBe('info')
    expect(entries[1].level).toBe('warn')
    expect(entries[2].level).toBe('error')
  })

  it('source field is preserved', () => {
    LogStore.append('info', 'console', 'from console')
    LogStore.append('error', 'kernel', 'from kernel')
    const entries = LogStore.getEntries()
    expect(entries[0].source).toBe('console')
    expect(entries[1].source).toBe('kernel')
  })
})
