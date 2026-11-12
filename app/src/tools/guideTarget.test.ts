import { describe, it, expect, vi } from 'vitest'
import { GuideTarget } from './guideTarget'
import type { Ray } from '../viewport/math'

const ray: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** A target with `selected` as the app's guide and an acquirer hitting
 *  `hit`, whose hooks keep `selected` in step as the Viewport does. */
function make(selected: bigint | null, hit: bigint | null) {
  const target = new GuideTarget()
  target.selected = selected
  const acquire = vi.fn(() => {
    if (hit !== null) target.selected = hit
    return hit
  })
  const onCommit = vi.fn((g: bigint | null) => { target.selected = g })
  target.setHooks(acquire, onCommit)
  return { target, acquire, onCommit }
}

describe('GuideTarget.start', () => {
  it('acts on nodes, without picking, while nodes are selected', () => {
    const t = make(null, 9n)
    expect(t.target.start(ray, false, true)).toBeNull()
    expect(t.acquire).not.toHaveBeenCalled()
  })

  it('takes the guide under the click when none is selected', () => {
    expect(make(null, 9n).target.start(ray, true, false)).toBe(9n)
  })

  it('keeps a selected guide without picking unless retargeting', () => {
    const t = make(7n, 9n)
    expect(t.target.start(ray, true, false)).toBe(7n)
    expect(t.acquire).not.toHaveBeenCalled()
  })

  it('retargets to the clicked guide when asked; a miss keeps the selected one', () => {
    expect(make(7n, 9n).target.start(ray, true, true)).toBe(9n)
    expect(make(7n, null).target.start(ray, true, true)).toBe(7n)
  })
})

describe('GuideTarget landing', () => {
  it('a selected guide stays selected; a copy of it becomes the selection', () => {
    const t = make(7n, null)
    t.target.start(ray, true, false)
    t.target.landed(7n)
    expect(t.onCommit).toHaveBeenLastCalledWith(7n)
    t.target.landed(50n)
    expect(t.onCommit).toHaveBeenLastCalledWith(50n)
  })

  it('a click-taken guide gives the selection back, landed or ended', () => {
    const t = make(null, 9n)
    t.target.start(ray, true, false)
    t.target.landed(9n)
    expect(t.onCommit).toHaveBeenLastCalledWith(null)

    const u = make(7n, 9n)
    u.target.start(ray, true, true)
    u.target.ended()
    expect(u.onCommit).toHaveBeenLastCalledWith(7n)
    expect(u.target.selected).toBe(7n)
  })

  it('ended() is quiet for a guide selected beforehand, and a new gesture forgets the old restore', () => {
    const t = make(7n, null)
    t.target.start(ray, true, false)
    t.target.ended()
    expect(t.onCommit).not.toHaveBeenCalled()

    const u = make(null, 9n)
    u.target.start(ray, true, false)
    u.target.start(ray, false, true) // the next gesture acts on nodes
    u.target.ended()
    expect(u.onCommit).not.toHaveBeenCalled()
  })
})

describe('GuideTarget.canAcquire', () => {
  it('tells whether a click can take a guide (a picker was given)', () => {
    const target = new GuideTarget()
    expect(target.canAcquire).toBe(false)
    target.setHooks(() => null, vi.fn())
    expect(target.canAcquire).toBe(true)
    target.setHooks(null, vi.fn())
    expect(target.canAcquire).toBe(false)
  })
})
