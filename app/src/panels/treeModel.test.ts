import { describe, it, expect } from 'vitest'
import {
  entityLabel,
  resolveLabel,
  breadcrumb,
  isTreeRowDimmed,
  nextSelection,
  canGroup,
  canUngroup,
  nodeEq,
  nodeRefFromJs,
  nodeKindToNumber,
  canMakeComponent,
  canPlaceInstance,
  canExplodeInstance,
  canMakeUnique,
  type NodeRef,
} from './treeModel'

describe('entityLabel', () => {
  it('is 1-based per kind', () => {
    expect(entityLabel('object', 0)).toBe('Object 1')
    expect(entityLabel('object', 2)).toBe('Object 3')
    expect(entityLabel('sketch', 0)).toBe('Sketch 1')
    expect(entityLabel('group', 0)).toBe('Group 1')
    expect(entityLabel('group', 2)).toBe('Group 3')
    expect(entityLabel('instance', 0)).toBe('Component 1')
    expect(entityLabel('instance', 2)).toBe('Component 3')
  })
})

describe('resolveLabel', () => {
  it('returns the kernel name when present', () => {
    expect(resolveLabel('Counter_Base', undefined, 'object', 0)).toBe('Counter_Base')
    expect(resolveLabel('My Group', undefined, 'group', 2)).toBe('My Group')
    expect(resolveLabel('Chair', undefined, 'instance', 0)).toBe('Chair')
  })

  it('falls back to entityLabel when kernel name is absent', () => {
    expect(resolveLabel(undefined, undefined, 'object', 0)).toBe('Object 1')
    expect(resolveLabel(undefined, undefined, 'group', 2)).toBe('Group 3')
    expect(resolveLabel(undefined, undefined, 'instance', 0)).toBe('Component 1')
  })

  it('uses the def name for an instance with no own name', () => {
    expect(resolveLabel(undefined, 'TableDef', 'instance', 0)).toBe('TableDef')
  })

  it('prefers the instance own name over the def name', () => {
    expect(resolveLabel('My Table', 'TableDef', 'instance', 0)).toBe('My Table')
  })

  it('ignores defName for non-instance kinds (falls through to entityLabel)', () => {
    // defName is only meaningful for instances; other kinds fall back to positional
    expect(resolveLabel(undefined, 'SomeDefName', 'object', 1)).toBe('Object 2')
    expect(resolveLabel(undefined, 'SomeDefName', 'group', 1)).toBe('Group 2')
  })

  it('falls back to positional when instance has neither own name nor def name', () => {
    expect(resolveLabel(undefined, undefined, 'instance', 3)).toBe('Component 4')
  })
})

describe('nodeKindToNumber', () => {
  it('maps object → 0, group → 1, instance → 2', () => {
    expect(nodeKindToNumber('object')).toBe(0)
    expect(nodeKindToNumber('group')).toBe(1)
    expect(nodeKindToNumber('instance')).toBe(2)
  })
})

describe('canMakeComponent', () => {
  const a: NodeRef = { kind: 'object', id: 1n }
  const b: NodeRef = { kind: 'object', id: 2n }
  const g: NodeRef = { kind: 'group', id: 3n }
  const inst: NodeRef = { kind: 'instance', id: 4n }

  const noParent = (_n: NodeRef) => undefined

  it('true for 2 sibling objects', () => {
    expect(canMakeComponent([a, b], noParent)).toBe(true)
  })

  it('true for object + group at top level', () => {
    expect(canMakeComponent([a, g], noParent)).toBe(true)
  })

  it('false when an instance is in the selection', () => {
    expect(canMakeComponent([a, inst], noParent)).toBe(false)
  })

  it('true for a single object (the common case)', () => {
    expect(canMakeComponent([a], noParent)).toBe(true)
  })

  it('false for an empty selection', () => {
    expect(canMakeComponent([], noParent)).toBe(false)
  })

  it('false for a single instance', () => {
    expect(canMakeComponent([inst], noParent)).toBe(false)
  })

  it('false for nodes with different parents', () => {
    const mixedParent = (n: NodeRef) => n.id === 1n ? 99n : 100n
    expect(canMakeComponent([a, b], mixedParent)).toBe(false)
  })
})

describe('canPlaceInstance', () => {
  const inst: NodeRef = { kind: 'instance', id: 1n }
  const obj: NodeRef = { kind: 'object', id: 2n }

  it('true for exactly one selected instance', () => {
    expect(canPlaceInstance([inst])).toBe(true)
  })

  it('false for an object', () => {
    expect(canPlaceInstance([obj])).toBe(false)
  })

  it('false for empty selection', () => {
    expect(canPlaceInstance([])).toBe(false)
  })

  it('false for two instances', () => {
    const inst2: NodeRef = { kind: 'instance', id: 3n }
    expect(canPlaceInstance([inst, inst2])).toBe(false)
  })
})

describe('canExplodeInstance', () => {
  const inst: NodeRef = { kind: 'instance', id: 1n }
  const obj: NodeRef = { kind: 'object', id: 2n }
  const grp: NodeRef = { kind: 'group', id: 3n }

  it('true for exactly one selected instance', () => {
    expect(canExplodeInstance([inst])).toBe(true)
  })

  it('false for an object', () => {
    expect(canExplodeInstance([obj])).toBe(false)
  })

  it('false for a group', () => {
    expect(canExplodeInstance([grp])).toBe(false)
  })

  it('false for empty selection', () => {
    expect(canExplodeInstance([])).toBe(false)
  })

  it('false for two instances', () => {
    const inst2: NodeRef = { kind: 'instance', id: 4n }
    expect(canExplodeInstance([inst, inst2])).toBe(false)
  })
})

describe('canMakeUnique', () => {
  const inst: NodeRef = { kind: 'instance', id: 1n }
  const obj: NodeRef = { kind: 'object', id: 2n }
  const grp: NodeRef = { kind: 'group', id: 3n }

  it('true for exactly one selected instance', () => {
    expect(canMakeUnique([inst])).toBe(true)
  })

  it('false for an object', () => {
    expect(canMakeUnique([obj])).toBe(false)
  })

  it('false for a group', () => {
    expect(canMakeUnique([grp])).toBe(false)
  })

  it('false for empty selection', () => {
    expect(canMakeUnique([])).toBe(false)
  })

  it('false for two instances', () => {
    const inst2: NodeRef = { kind: 'instance', id: 4n }
    expect(canMakeUnique([inst, inst2])).toBe(false)
  })
})

describe('nodeEq', () => {
  it('matches same kind and id', () => {
    expect(nodeEq({ kind: 'object', id: 1n }, { kind: 'object', id: 1n })).toBe(true)
  })
  it('does not match different kind', () => {
    expect(nodeEq({ kind: 'object', id: 1n }, { kind: 'group', id: 1n })).toBe(false)
  })
  it('does not match different id', () => {
    expect(nodeEq({ kind: 'object', id: 1n }, { kind: 'object', id: 2n })).toBe(false)
  })
})

describe('nodeRefFromJs', () => {
  it('converts a NodeJs-like value to NodeRef', () => {
    const js = { kind: 'group', id: 5n }
    const ref = nodeRefFromJs(js)
    expect(ref).toEqual({ kind: 'group', id: 5n })
  })
})

describe('breadcrumb', () => {
  const labelFor = (n: NodeRef) =>
    n.kind === 'object' ? `Object ${n.id}` : `Group ${n.id}`

  it('is just Model at top level (empty path)', () => {
    expect(breadcrumb([], labelFor)).toEqual([{ label: 'Model', depth: -1 }])
  })

  it('appends path nodes with their labels and depths', () => {
    const g: NodeRef = { kind: 'group', id: 10n }
    const o: NodeRef = { kind: 'object', id: 20n }
    expect(breadcrumb([g, o], labelFor)).toEqual([
      { label: 'Model', depth: -1 },
      { label: 'Group 10', depth: 0 },
      { label: 'Object 20', depth: 1 },
    ])
  })
})

describe('isTreeRowDimmed', () => {
  const g: NodeRef = { kind: 'group', id: 10n }
  const o: NodeRef = { kind: 'object', id: 20n }
  const other: NodeRef = { kind: 'object', id: 30n }

  it('dims nothing at top level (empty path)', () => {
    expect(isTreeRowDimmed([], g, 0)).toBe(false)
    expect(isTreeRowDimmed([], o, 0)).toBe(false)
  })

  it('dims rows not matching the context node at depth 0', () => {
    expect(isTreeRowDimmed([g], other, 0)).toBe(true)
    expect(isTreeRowDimmed([g], g, 0)).toBe(false)
  })

  it('does not dim rows deeper than the path (they are inside the context)', () => {
    // path has 1 node; rows at depth 1 are children of the context → not dimmed
    expect(isTreeRowDimmed([g], o, 1)).toBe(false)
  })

  it('dims a sibling at depth 1 inside a group context', () => {
    expect(isTreeRowDimmed([g, o], other, 1)).toBe(true)
    expect(isTreeRowDimmed([g, o], o, 1)).toBe(false)
  })
})

describe('canGroup', () => {
  const a: NodeRef = { kind: 'object', id: 1n }
  const b: NodeRef = { kind: 'object', id: 2n }
  const c: NodeRef = { kind: 'group', id: 3n }

  const noParent = (_n: NodeRef) => undefined
  const parentGroup = (_n: NodeRef) => 99n

  it('requires at least 2 distinct nodes', () => {
    expect(canGroup([a], noParent)).toBe(false)
    expect(canGroup([a, a], noParent)).toBe(false)
  })

  it('true for 2 top-level nodes', () => {
    expect(canGroup([a, b], noParent)).toBe(true)
  })

  it('true for 2 nodes sharing the same parent group', () => {
    expect(canGroup([a, b], parentGroup)).toBe(true)
  })

  it('false when nodes have different parents', () => {
    const mixedParent = (n: NodeRef) => n.id === 1n ? 99n : 100n
    expect(canGroup([a, b], mixedParent)).toBe(false)
  })

  it('true for object + group at top level', () => {
    expect(canGroup([a, c], noParent)).toBe(true)
  })
})

describe('canUngroup', () => {
  const g: NodeRef = { kind: 'group', id: 1n }
  const o: NodeRef = { kind: 'object', id: 2n }

  it('true for exactly one selected group', () => {
    expect(canUngroup([g])).toBe(true)
  })

  it('false for an object', () => {
    expect(canUngroup([o])).toBe(false)
  })

  it('false for zero selection', () => {
    expect(canUngroup([])).toBe(false)
  })

  it('false for two groups', () => {
    const g2: NodeRef = { kind: 'group', id: 3n }
    expect(canUngroup([g, g2])).toBe(false)
  })
})

describe('nextSelection (NodeRef)', () => {
  const a: NodeRef = { kind: 'object', id: 10n }
  const b: NodeRef = { kind: 'object', id: 20n }
  const g: NodeRef = { kind: 'group', id: 30n }

  it('replaces on a plain click', () => {
    expect(nextSelection([a], b, false)).toEqual([b])
  })

  it('clears on an empty click', () => {
    expect(nextSelection([a, b], null, false)).toEqual([])
  })

  it('appends a new node additively, preserving order', () => {
    expect(nextSelection([a], b, true)).toEqual([a, b])
  })

  it('toggles an already-selected node off additively', () => {
    expect(nextSelection([a, b], a, true)).toEqual([b])
  })

  it('treats object and group with same id as distinct', () => {
    const sameIdGroup: NodeRef = { kind: 'group', id: 10n }
    // a is {object,10n}; sameIdGroup is {group,10n} — different nodes
    expect(nextSelection([a], sameIdGroup, true)).toEqual([a, sameIdGroup])
  })
})
