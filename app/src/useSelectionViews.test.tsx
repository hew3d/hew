import { describe, it, expect } from 'vitest'
import { renderHook } from '@testing-library/react'
import { useSelectionViews } from './useSelectionViews'
import type { SelItem } from './panels/treeModel'

const box: SelItem = { kind: 'object', id: 1n }
const grp: SelItem = { kind: 'group', id: 2n }
const guide: SelItem = { kind: 'guide', id: 3n }
const dim: SelItem = { kind: 'annotation', id: 4n }

describe('useSelectionViews', () => {
  it('splits the selection into per-kind views in pick order', () => {
    const { result } = renderHook(() => useSelectionViews([guide, box, dim, grp]))
    expect(result.current.nodes).toEqual([box, grp])
    expect(result.current.guides).toEqual([3n])
    expect(result.current.annotations).toEqual([4n])
  })

  it('keeps a view identical while its own slice is unchanged', () => {
    const { result, rerender } = renderHook((p: { sel: SelItem[] }) => useSelectionViews(p.sel), {
      initialProps: { sel: [box] as SelItem[] },
    })
    const first = result.current
    // Adding a guide changes the guides, not the nodes.
    rerender({ sel: [box, guide] })
    expect(result.current.nodes).toBe(first.nodes)
    expect(result.current.guides).not.toBe(first.guides)
    expect(result.current.annotations).toBe(first.annotations)
    // A new array holding the same entries changes nothing at all.
    const second = result.current
    rerender({ sel: [box, guide] })
    expect(result.current).toBe(second)
    // A node joining changes the nodes.
    rerender({ sel: [box, guide, grp] })
    expect(result.current.nodes).not.toBe(second.nodes)
    expect(result.current.guides).toBe(second.guides)
  })
})
