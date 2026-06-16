import { describe, it, expect } from 'vitest'
import { entityLabel, breadcrumb, isDimmed, nextSelection } from './treeModel'

describe('entityLabel', () => {
  it('is 1-based per kind', () => {
    expect(entityLabel('object', 0)).toBe('Object 1')
    expect(entityLabel('object', 2)).toBe('Object 3')
    expect(entityLabel('sketch', 0)).toBe('Sketch 1')
  })
})

describe('breadcrumb', () => {
  const a = 10n
  const b = 20n
  const objects = [a, b]

  it('is just Model at top level', () => {
    expect(breadcrumb(null, objects)).toEqual([{ label: 'Model', contextId: null }])
  })

  it('appends the entered object with its 1-based label', () => {
    expect(breadcrumb(b, objects)).toEqual([
      { label: 'Model', contextId: null },
      { label: 'Object 2', contextId: b },
    ])
  })

  it('collapses to Model when the context is not a known object', () => {
    expect(breadcrumb(999n, objects)).toEqual([{ label: 'Model', contextId: null }])
  })
})

describe('isDimmed', () => {
  it('dims nothing at top level', () => {
    expect(isDimmed(10n, null)).toBe(false)
  })

  it('dims everything except the active context', () => {
    expect(isDimmed(10n, 10n)).toBe(false)
    expect(isDimmed(20n, 10n)).toBe(true)
  })
})

describe('nextSelection', () => {
  it('replaces on a plain click', () => {
    expect(nextSelection([10n], 20n, false)).toEqual([20n])
  })

  it('clears on an empty click', () => {
    expect(nextSelection([10n, 20n], null, false)).toEqual([])
  })

  it('appends a new id additively, preserving order', () => {
    expect(nextSelection([10n], 20n, true)).toEqual([10n, 20n])
  })

  it('toggles an already-selected id off additively', () => {
    expect(nextSelection([10n, 20n], 10n, true)).toEqual([20n])
  })
})
