import { describe, expect, it, vi } from 'vitest'
import { createInferenceStore } from './inferenceStore'

const info = (x: number) => ({ kind: 'on-face', screenX: x, screenY: 0 })

describe('inference store', () => {
  it('notifies subscribers on a change and not on the same value', () => {
    const store = createInferenceStore()
    const seen = vi.fn()
    store.subscribe(seen)
    const a = info(1)
    store.set(a)
    store.set(a)
    expect(seen).toHaveBeenCalledTimes(1)
    store.set(null)
    expect(seen).toHaveBeenCalledTimes(2)
    expect(store.get()).toBeNull()
  })

  it('stops notifying after unsubscribe', () => {
    const store = createInferenceStore()
    const seen = vi.fn()
    const off = store.subscribe(seen)
    off()
    store.set(info(1))
    expect(seen).not.toHaveBeenCalled()
  })
})
