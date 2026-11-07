/**
 * The editing scope reaches the renderer and the kernel together
 * (docs/design/xray-edit-scope-pick.md, "App"): one helper feeds both, so
 * the dimming and the pick skip (Shaded and X-ray) cannot drift apart, and every push
 * starts a new pick generation.
 */
import { describe, it, expect, vi } from 'vitest'
import { pushActiveScope } from './pickScope'
import { pickPolicyGeneration } from '../tools/pickPolicy'

function doubles() {
  const lit: Array<[Set<bigint> | null, Set<bigint> | null]> = []
  const renderer = { setActiveContext: vi.fn((a: Set<bigint> | null, b: Set<bigint> | null) => { lit.push([a, b]) }) }
  const kernel = { set_pick_scope: vi.fn(), clear_pick_scope: vi.fn() }
  return { renderer, kernel, lit }
}

describe('pushActiveScope', () => {
  it('hands the kernel exactly the sets the renderer receives', () => {
    const { renderer, kernel } = doubles()
    pushActiveScope(renderer, kernel, new Set([3n, 5n]), new Set([9n]), true)
    const [rObjects, rInstances] = renderer.setActiveContext.mock.calls[0]
    const [kObjects, kInstances, kFrame] = kernel.set_pick_scope.mock.calls[0]
    expect(kFrame).toBe(true)
    expect(Array.from(kObjects as BigUint64Array).sort()).toEqual(Array.from(rObjects as Set<bigint>).sort())
    expect(Array.from(kInstances as BigUint64Array)).toEqual(Array.from(rInstances as Set<bigint>))
    expect(kernel.clear_pick_scope).not.toHaveBeenCalled()
  })

  it('treats a missing instance set as empty for the kernel and keeps it null for the renderer', () => {
    const { renderer, kernel } = doubles()
    pushActiveScope(renderer, kernel, new Set([3n]), null, false)
    expect(renderer.setActiveContext).toHaveBeenCalledWith(new Set([3n]), null)
    const [objects, instances, frame] = kernel.set_pick_scope.mock.calls[0]
    expect(frame).toBe(false)
    expect(Array.from(objects as BigUint64Array)).toEqual([3n])
    expect(Array.from(instances as BigUint64Array)).toEqual([])
  })

  it('a placement-only scope names instances and no objects', () => {
    const { renderer, kernel } = doubles()
    pushActiveScope(renderer, kernel, null, new Set([7n]), false)
    expect(renderer.setActiveContext).toHaveBeenCalledWith(null, new Set([7n]))
    const [objects, instances] = kernel.set_pick_scope.mock.calls[0]
    expect(Array.from(objects as BigUint64Array)).toEqual([])
    expect(Array.from(instances as BigUint64Array)).toEqual([7n])
  })

  it('clears the kernel scope when nothing is lit (the top level), renderer included', () => {
    const { renderer, kernel } = doubles()
    pushActiveScope(renderer, kernel, null, null, true)
    expect(renderer.setActiveContext).toHaveBeenCalledWith(null, null)
    expect(kernel.clear_pick_scope).toHaveBeenCalledTimes(1)
    expect(kernel.set_pick_scope).not.toHaveBeenCalled()
  })

  it('tolerates a renderer that is not up yet and still sets the kernel scope', () => {
    const { kernel } = doubles()
    pushActiveScope(undefined, kernel, new Set([1n]), new Set(), true)
    expect(kernel.set_pick_scope).toHaveBeenCalledTimes(1)
  })

  it('bumps the pick generation on every push, so memoized picks miss', () => {
    const { renderer, kernel } = doubles()
    const g0 = pickPolicyGeneration()
    pushActiveScope(renderer, kernel, new Set([1n]), null, true)
    const g1 = pickPolicyGeneration()
    expect(g1).toBeGreaterThan(g0)
    pushActiveScope(renderer, kernel, null, null, true)
    expect(pickPolicyGeneration()).toBeGreaterThan(g1)
  })
})
