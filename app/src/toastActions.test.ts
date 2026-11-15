import { describe, expect, it, vi } from 'vitest'
import { toastActionFor } from './toastActions'
import { requiresPreview } from './test/requiresPreview'

describe('toastActionFor', () => {
  it('a boolean refused for a face with no area offers Model Health on the selection', (ctx) => {
    requiresPreview(ctx, 'remove-degenerate-faces')
    const open = vi.fn()
    const action = toastActionFor('DegenerateInput', open)
    expect(action?.label).toBe('Open Model Health')
    action?.run()
    expect(open).toHaveBeenCalledWith({ scope: 'selection' })
  })

  it('a fillet refused on an object with gaps offers Model Health on the model', () => {
    const open = vi.fn()
    const action = toastActionFor('BlendNeedsSolid', open)
    expect(action?.label).toBe('Open Model Health')
    action?.run()
    expect(open).toHaveBeenCalledWith({ scope: 'document' })
  })

  it('other toasts carry no button', () => {
    expect(toastActionFor('NonPlanarInput', vi.fn())).toBeUndefined()
    expect(toastActionFor(undefined, vi.fn())).toBeUndefined()
  })
})
