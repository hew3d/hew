import { describe, it, expect } from 'vitest'
import { isolateHiddenFor } from './isolate'
import type { NodeRef } from '../panels/treeModel'

describe('isolateHiddenFor', () => {
  it('hides every other object when isolating a plain object', () => {
    const result = isolateHiddenFor(
      { kind: 'object', id: 2n },
      [1n, 2n, 3n],
      [10n, 11n],
      () => [],
    )
    expect(result.hiddenObjectIds).toEqual([1n, 3n])
    expect(result.hiddenInstanceIds).toEqual([10n, 11n])
  })

  it('hides every other instance when isolating an instance', () => {
    const result = isolateHiddenFor(
      { kind: 'instance', id: 10n },
      [1n, 2n],
      [10n, 11n],
      () => [],
    )
    expect(result.hiddenObjectIds).toEqual([1n, 2n])
    expect(result.hiddenInstanceIds).toEqual([11n])
  })

  it('keeps every member of an isolated group, recursively through a nested group', () => {
    const members: Record<string, NodeRef[]> = {
      '100': [{ kind: 'object', id: 1n }, { kind: 'group', id: 101n }],
      '101': [{ kind: 'object', id: 2n }, { kind: 'instance', id: 10n }],
    }
    const result = isolateHiddenFor(
      { kind: 'group', id: 100n },
      [1n, 2n, 3n],
      [10n, 11n],
      (groupId) => members[String(groupId)] ?? [],
    )
    expect(result.hiddenObjectIds).toEqual([3n])
    expect(result.hiddenInstanceIds).toEqual([11n])
  })

  it('hides everything when isolating an empty group', () => {
    const result = isolateHiddenFor({ kind: 'group', id: 5n }, [1n], [10n], () => [])
    expect(result.hiddenObjectIds).toEqual([1n])
    expect(result.hiddenInstanceIds).toEqual([10n])
  })
})
