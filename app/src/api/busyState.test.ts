import { afterEach, describe, expect, it, vi } from 'vitest'
import { getBusy, setBusy, subscribeBusy, type BusyState } from './busyState'

const state = (label: string | null): BusyState => ({
  plugin: 'com.example.parts',
  label,
  startedAt: 0,
  cancel: () => {},
})

describe('busyState', () => {
  afterEach(() => setBusy(null))

  it('starts free and holds what was set', () => {
    expect(getBusy()).toBeNull()
    const busy = state('Cut list')
    setBusy(busy)
    expect(getBusy()).toBe(busy)
    setBusy(null)
    expect(getBusy()).toBeNull()
  })

  it('tells subscribers about each change, once, until they unsubscribe', () => {
    const listener = vi.fn()
    const unsubscribe = subscribeBusy(listener)
    const busy = state(null)
    setBusy(busy)
    setBusy(busy) // no change, no call
    setBusy(null)
    expect(listener).toHaveBeenCalledTimes(2)
    unsubscribe()
    setBusy(state('x'))
    expect(listener).toHaveBeenCalledTimes(2)
  })
})
