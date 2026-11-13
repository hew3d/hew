import { describe, it, expect } from 'vitest'
import {
  entityLabel,
  makeLabelFor,
  resolveLabel,
  breadcrumb,
  isTreeRowDimmed,
  nextSelection,
  mergeSelection,
  canBoolean,
  canGroup,
  canUngroup,
  nodeEq,
  nodeRefFromJs,
  nodeKindToNumber,
  canMakeComponent,
  canPlaceInstance,
  canExplodeInstance,
  canMakeUnique,
  canBooleanInComponent,
  stripTagSuffix,
  collectLeafIds,
  buildTreeIndexMap,
  treeIndexOf,
  nodeKey,
  structuralSelection,
  pruneDeadSelection,
  collectDescendants,
  filterTreeKeys,
  dropTargetFor,
  placementLabel,
  type LabelScene,
  type NodeRef,
} from './treeModel'

// Draft refs (the drawing on a plane) carry the plane's canonical context
// string and the kernel's 16-hex unit/edge/curve/region key.
const CTX1 = '{"plane":[0,0,0,0,0,1]}'
const CTX2 = '{"plane":[0,0,2,0,0,1]}'
const hex = (n: number): string => n.toString(16).padStart(16, '0')
const dr = (kind: NodeRef['kind'], ctx: string, n: number): NodeRef => ({ kind, id: BigInt(n), ctx, key: hex(n) })

describe('nodeEq / nodeKey — draft-edge plane scoping', () => {
  it('two edges with the same key on DIFFERENT planes are distinct', () => {
    const a = dr('draft-edge', CTX1, 5)
    const b = dr('draft-edge', CTX2, 5)
    expect(nodeEq(a, b)).toBe(false)
    expect(nodeKey(a)).not.toBe(nodeKey(b))
  })

  it('the same edge equals itself and keys stably', () => {
    const a = dr('draft-edge', CTX1, 5)
    const b = dr('draft-edge', CTX1, 5)
    expect(nodeEq(a, b)).toBe(true)
    expect(nodeKey(a)).toBe(nodeKey(b))
  })

  it('an edge never equals a sketch or an object with the same id', () => {
    const edge = dr('draft-edge', CTX1, 5)
    expect(nodeEq(edge, { kind: 'sketch', id: 5n })).toBe(false)
    expect(nodeEq(edge, { kind: 'object', id: 5n })).toBe(false)
    expect(nodeKey(edge)).not.toBe(nodeKey({ kind: 'sketch', id: 5n }))
  })

  it('a draft key is kind, plane and kernel key', () => {
    expect(nodeKey(dr('draft-edge', CTX1, 5))).toBe(`draft-edge:${CTX1}:${hex(5)}`)
  })

  it('plain node keys are unchanged by the optional ctx field being absent', () => {
    expect(nodeKey({ kind: 'object', id: 3n })).toBe('object:3')
  })
})

describe('nodeEq / nodeKey — draft-island plane scoping', () => {
  it('two islands with the same unit key on DIFFERENT planes are distinct', () => {
    const a = dr('draft-island', CTX1, 10)
    const b = dr('draft-island', CTX2, 10)
    expect(nodeEq(a, b)).toBe(false)
    expect(nodeKey(a)).not.toBe(nodeKey(b))
  })

  it('the same island equals itself and keys stably', () => {
    const a = dr('draft-island', CTX1, 10)
    const b = dr('draft-island', CTX1, 10)
    expect(nodeEq(a, b)).toBe(true)
    expect(nodeKey(a)).toBe(nodeKey(b))
  })

  it('a draft-curve never equals a draft-island with the same key/plane (kind differs)', () => {
    const island = dr('draft-island', CTX1, 5)
    const curve = dr('draft-curve', CTX1, 5)
    expect(nodeEq(island, curve)).toBe(false)
    expect(nodeKey(island)).not.toBe(nodeKey(curve))
  })

  it('an island never equals a plain object with the same id (plane-scoped vs unscoped keys differ)', () => {
    const island = dr('draft-island', CTX1, 5)
    expect(nodeEq(island, { kind: 'object', id: 5n })).toBe(false)
    expect(nodeKey(island)).not.toBe(nodeKey({ kind: 'object', id: 5n }))
  })
})

describe('stripTagSuffix', () => {
  it('returns the name unchanged when there is no tag suffix', () => {
    expect(stripTagSuffix('Counter Base')).toBe('Counter Base')
    expect(stripTagSuffix('')).toBe('')
  })

  it('strips the __HEWTAG__ portion and everything after it', () => {
    expect(stripTagSuffix('Roof Truss A__HEWTAG__Structure')).toBe('Roof Truss A')
    expect(stripTagSuffix('Wall__HEWTAG__Exterior')).toBe('Wall')
  })

  it('handles the underscore-mangled delimiter SketchUp exports', () => {
    // Empty display (unnamed group) → empty string.
    expect(stripTagSuffix('___HEWTAG__Exterior_Foundation')).toBe('')
    expect(stripTagSuffix('Wall___HEWTAG__Roof_Framing')).toBe('Wall')
  })
})

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

  it('strips __HEWTAG__ suffix from kernel names for display', () => {
    expect(resolveLabel('Roof Truss A__HEWTAG__Structure', undefined, 'object', 0)).toBe('Roof Truss A')
    expect(resolveLabel('Wall__HEWTAG__Exterior', undefined, 'group', 0)).toBe('Wall')
  })

  it('falls back to entityLabel when kernel name is absent', () => {
    expect(resolveLabel(undefined, undefined, 'object', 0)).toBe('Object 1')
    expect(resolveLabel(undefined, undefined, 'group', 2)).toBe('Group 3')
    expect(resolveLabel(undefined, undefined, 'instance', 0)).toBe('Component 1')
  })

  it('uses the def name for an instance with no own name', () => {
    expect(resolveLabel(undefined, 'TableDef', 'instance', 0)).toBe('TableDef')
  })

  it('shows "Instance Name (Definition Name)" when an instance has its own name', () => {
    expect(resolveLabel('My Table', 'TableDef', 'instance', 0)).toBe('My Table (TableDef)')
  })

  it('drops the parenthetical when instance and definition names coincide', () => {
    expect(resolveLabel('Table', 'Table', 'instance', 0)).toBe('Table')
  })

  it('shows just the instance name when the definition is unnamed', () => {
    expect(resolveLabel('My Table', undefined, 'instance', 0)).toBe('My Table')
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

  it('handles the sketch kind without throwing (sentinel -1 — no kernel NodeId)', () => {
    expect(() => nodeKindToNumber('sketch')).not.toThrow()
    expect(nodeKindToNumber('sketch')).toBe(-1)
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

  // Nested components: a selected instance folds in as a nested member of
  // the new definition (the kernel accepts it), so the gate must open —
  // this used to refuse while nested definitions were unsupported.
  it('true when an instance is in the selection (it becomes a nested member)', () => {
    expect(canMakeComponent([a, inst], noParent)).toBe(true)
  })

  it('true for a single object (the common case)', () => {
    expect(canMakeComponent([a], noParent)).toBe(true)
  })

  it('false for an empty selection', () => {
    expect(canMakeComponent([], noParent)).toBe(false)
  })

  it('true for a single instance (wrapping one component in another)', () => {
    expect(canMakeComponent([inst], noParent)).toBe(true)
  })

  it('false for nodes with different parents', () => {
    const mixedParent = (n: NodeRef) => n.id === 1n ? 99n : 100n
    expect(canMakeComponent([a, b], mixedParent)).toBe(false)
  })

  // Sketch-scoped NodeRefs have no kernel NodeId: letting one through the
  // gate forwards its id into the object handle space downstream, where the
  // slotmaps' reused bit patterns can silently alias an unrelated live node.
  it('false for any sketch-kind selection (no kernel NodeId — id-space guard)', () => {
    const sk: NodeRef = { kind: 'sketch', id: 5n }
    const island = dr('draft-island', CTX1, 6)
    const edge = dr('draft-edge', CTX1, 7)
    expect(canMakeComponent([sk], noParent)).toBe(false)
    expect(canMakeComponent([island], noParent)).toBe(false)
    expect(canMakeComponent([a, sk], noParent)).toBe(false)
    expect(canMakeComponent([a, edge], noParent)).toBe(false)
  })
})

describe('canGroup — sketch-kind guard', () => {
  const a: NodeRef = { kind: 'object', id: 1n }
  const b: NodeRef = { kind: 'object', id: 2n }
  const noParent = (_n: NodeRef) => undefined

  it('false when any sketch-kind node is in the selection', () => {
    const sk: NodeRef = { kind: 'sketch', id: 5n }
    const curve = dr('draft-curve', CTX1, 6)
    expect(canGroup([a, sk], noParent)).toBe(false)
    expect(canGroup([a, b, curve], noParent)).toBe(false)
  })
})

describe('structuralSelection — the node-id-space boundary', () => {
  it('collapses object/group/instance selections to parallel kind/id arrays', () => {
    const sel = structuralSelection([
      { kind: 'object', id: 1n },
      { kind: 'group', id: 2n },
      { kind: 'instance', id: 3n },
    ])
    expect(sel).not.toBeNull()
    expect(Array.from(sel!.kinds)).toEqual([0, 1, 2])
    expect(Array.from(sel!.ids)).toEqual([1n, 2n, 3n])
  })

  it('refuses (null) when ANY node is sketch-scoped — a sketch id must never enter the node-id handle space', () => {
    expect(structuralSelection([{ kind: 'sketch', id: 5n }])).toBeNull()
    expect(structuralSelection([
      { kind: 'object', id: 1n },
      dr('draft-island', CTX1, 6),
    ])).toBeNull()
    expect(structuralSelection([
      dr('draft-edge', CTX1, 7),
    ])).toBeNull()
    expect(structuralSelection([
      dr('draft-curve', CTX1, 8),
    ])).toBeNull()
    expect(structuralSelection([
      dr('draft-region', CTX1, 9),
    ])).toBeNull()
  })

  it('an empty selection collapses to empty arrays (caller guards emptiness itself)', () => {
    const sel = structuralSelection([])
    expect(sel).not.toBeNull()
    expect(sel!.kinds.length).toBe(0)
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

describe('collectLeafIds', () => {
  it('a plain object is its own leaf', () => {
    const result = collectLeafIds({ kind: 'object', id: 1n }, () => [])
    expect(result).toEqual({ objectIds: [1n], instanceIds: [] })
  })

  it('a plain instance is its own leaf', () => {
    const result = collectLeafIds({ kind: 'instance', id: 5n }, () => [])
    expect(result).toEqual({ objectIds: [], instanceIds: [5n] })
  })

  it('a sketch contributes no leaves', () => {
    const result = collectLeafIds({ kind: 'sketch', id: 9n }, () => [])
    expect(result).toEqual({ objectIds: [], instanceIds: [] })
  })

  it('a group expands to its direct object and instance members', () => {
    const members: Record<string, NodeRef[]> = {
      '10': [{ kind: 'object', id: 1n }, { kind: 'instance', id: 2n }],
    }
    const result = collectLeafIds({ kind: 'group', id: 10n }, (id) => members[String(id)] ?? [])
    expect(result.objectIds).toEqual([1n])
    expect(result.instanceIds).toEqual([2n])
  })

  it('recurses through nested subgroups (imported components arrive as a group-of-groups)', () => {
    // group 10 -> [object 1, group 20 -> [instance 2, group 30 -> [object 3]]]
    const members: Record<string, NodeRef[]> = {
      '10': [{ kind: 'object', id: 1n }, { kind: 'group', id: 20n }],
      '20': [{ kind: 'instance', id: 2n }, { kind: 'group', id: 30n }],
      '30': [{ kind: 'object', id: 3n }],
    }
    const result = collectLeafIds({ kind: 'group', id: 10n }, (id) => members[String(id)] ?? [])
    expect(result.objectIds.sort()).toEqual([1n, 3n].sort())
    expect(result.instanceIds).toEqual([2n])
  })

  it('an empty group contributes no leaves', () => {
    const result = collectLeafIds({ kind: 'group', id: 10n }, () => [])
    expect(result).toEqual({ objectIds: [], instanceIds: [] })
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

describe('canBoolean', () => {
  const a: NodeRef = { kind: 'object', id: 1n }
  const b: NodeRef = { kind: 'object', id: 2n }
  const g: NodeRef = { kind: 'group', id: 3n }
  const inst: NodeRef = { kind: 'instance', id: 4n }

  const topLevel = (_n: NodeRef) => undefined
  const live = (_n: NodeRef) => true

  it('true for two top-level objects, object+group, two groups, and an instance operand', () => {
    expect(canBoolean([a, b], topLevel, live)).toBe(true)
    expect(canBoolean([a, g], topLevel, live)).toBe(true)
    expect(canBoolean([g, { kind: 'group', id: 5n }], topLevel, live)).toBe(true)
    // A component instance is a valid operand (playtest finding 4): the
    // kernel refuses it directly, but the app transparently explodes it and
    // retries (Viewport.tsx's runBoolean) — the gate must not disable the
    // command for a selection that flow can actually handle.
    expect(canBoolean([a, inst], topLevel, live)).toBe(true)
    expect(canBoolean([inst, { kind: 'instance', id: 6n }], topLevel, live)).toBe(true)
  })

  it('requires two or more distinct operands', () => {
    expect(canBoolean([a], topLevel, live)).toBe(false)
    expect(canBoolean([], topLevel, live)).toBe(false)
    expect(canBoolean([a, a], topLevel, live)).toBe(false)
    expect(canBoolean([a, b, a], topLevel, live)).toBe(false)
    expect(canBoolean([a, b, g], topLevel, live)).toBe(true)
    expect(canBoolean([a, b, g, inst], topLevel, live)).toBe(true)
  })

  it('every one of three operands must be live and top-level', () => {
    const gNested = (n: NodeRef) => (n.id === 3n ? 99n : undefined)
    expect(canBoolean([a, b, g], gNested, live)).toBe(false)
    const gStale = (n: NodeRef) => n.id !== 3n
    expect(canBoolean([a, b, g], topLevel, gStale)).toBe(false)
  })

  it('false when either operand is NESTED inside a group — the gate must match the kernel GroupedOperand refusal', () => {
    const nestedFirst = (n: NodeRef) => (n.id === 1n ? 99n : undefined)
    expect(canBoolean([a, b], nestedFirst, live)).toBe(false)
    const nestedSecond = (n: NodeRef) => (n.id === 3n ? 99n : undefined)
    expect(canBoolean([a, g], nestedSecond, live)).toBe(false)
    const bothNested = (_n: NodeRef) => 99n
    expect(canBoolean([a, b], bothNested, live)).toBe(false)
  })

  it('false for non-live operands (a stale/hidden id, whatever the kind)', () => {
    const bStale = (n: NodeRef) => n.id !== 2n
    expect(canBoolean([a, b], topLevel, bStale)).toBe(false)
    const instStale = (n: NodeRef) => n.id !== 4n
    expect(canBoolean([a, inst], topLevel, instStale)).toBe(false)
  })
})

// component-edit-parity.md phase A2.
describe('canBooleanInComponent', () => {
  const a: NodeRef = { kind: 'object', id: 1n }
  const b: NodeRef = { kind: 'object', id: 2n }
  const g: NodeRef = { kind: 'group', id: 3n }
  const inst: NodeRef = { kind: 'instance', id: 4n }
  const member = (n: NodeRef) => n.id === 1n || n.id === 2n

  it('true for two distinct member objects', () => {
    expect(canBooleanInComponent([a, b], member)).toBe(true)
  })

  it('requires two or more distinct operands', () => {
    const c: NodeRef = { kind: 'object', id: 5n }
    const memberOfThree = (n: NodeRef) => n.id === 1n || n.id === 2n || n.id === 5n
    expect(canBooleanInComponent([a], member)).toBe(false)
    expect(canBooleanInComponent([a, a], member)).toBe(false)
    expect(canBooleanInComponent([a, b, a], memberOfThree)).toBe(false)
    expect(canBooleanInComponent([a, b, c], memberOfThree)).toBe(true)
    expect(canBooleanInComponent([a, b, c], member)).toBe(false)
  })

  it('false for a group or instance operand — a definition has no nested groups/instances', () => {
    expect(canBooleanInComponent([a, g], member)).toBe(false)
    expect(canBooleanInComponent([a, inst], member)).toBe(false)
  })

  it('false when either operand is not a member of the entered definition', () => {
    const notMember: NodeRef = { kind: 'object', id: 99n }
    expect(canBooleanInComponent([a, notMember], member)).toBe(false)
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
    expect(nextSelection([a], b, 'replace')).toEqual([b])
  })

  it('clears on an empty click', () => {
    expect(nextSelection([a, b], null, 'replace')).toEqual([])
  })

  it('keeps the selection on a modified empty click (toggle/add/subtract on air)', () => {
    expect(nextSelection([a, b], null, 'toggle')).toEqual([a, b])
    expect(nextSelection([a, b], null, 'add')).toEqual([a, b])
    expect(nextSelection([a, b], null, 'subtract')).toEqual([a, b])
  })

  it('toggle appends a new node, preserving order', () => {
    expect(nextSelection([a], b, 'toggle')).toEqual([a, b])
  })

  it('toggle removes an already-selected node', () => {
    expect(nextSelection([a, b], a, 'toggle')).toEqual([b])
  })

  it('add appends a new node and never removes a selected one', () => {
    expect(nextSelection([a], b, 'add')).toEqual([a, b])
    expect(nextSelection([a, b], a, 'add')).toEqual([a, b])
  })

  it('subtract removes a selected node and never adds an unselected one', () => {
    expect(nextSelection([a, b], a, 'subtract')).toEqual([b])
    expect(nextSelection([a], b, 'subtract')).toEqual([a])
  })

  it('treats object and group with same id as distinct', () => {
    const sameIdGroup: NodeRef = { kind: 'group', id: 10n }
    // a is {object,10n}; sameIdGroup is {group,10n} — different nodes
    expect(nextSelection([a], sameIdGroup, 'toggle')).toEqual([a, sameIdGroup])
    expect(nextSelection([a, g], g, 'subtract')).toEqual([a])
  })
})

describe('mergeSelection (marquee / Select All / Invert)', () => {
  const a: NodeRef = { kind: 'object', id: 10n }
  const b: NodeRef = { kind: 'object', id: 20n }
  const c: NodeRef = { kind: 'group', id: 30n }
  const e = dr('draft-edge', CTX1, 4)

  it('replace hands back the picked nodes (an empty pick clears)', () => {
    expect(mergeSelection([a], [b, c], 'replace')).toEqual([b, c])
    expect(mergeSelection([a], [], 'replace')).toEqual([])
  })

  it('add merges without duplicates, keeping the existing order first', () => {
    expect(mergeSelection([a, b], [b, c], 'add')).toEqual([a, b, c])
  })

  it('add returns the same array when every picked node is already selected', () => {
    const cur = [a, b]
    expect(mergeSelection(cur, [b, a], 'add')).toBe(cur)
  })

  it('subtract drops the picked nodes and keeps the rest in order', () => {
    expect(mergeSelection([a, b, c], [b], 'subtract')).toEqual([a, c])
    const cur = [a]
    expect(mergeSelection(cur, [b], 'subtract')).toBe(cur)
  })

  it('toggle flips each picked node: selected ones leave, unselected ones join', () => {
    expect(mergeSelection([a, b], [b, c], 'toggle')).toEqual([a, c])
  })

  it('toggle with nothing picked leaves the selection untouched', () => {
    const cur = [a, b]
    expect(mergeSelection(cur, [], 'toggle')).toBe(cur)
  })

  it('keys drawn sub-entities by their plane too', () => {
    const eOther = dr('draft-edge', CTX2, 4)
    expect(mergeSelection([e], [eOther], 'add')).toEqual([e, eOther])
    expect(mergeSelection([e], [eOther], 'subtract')).toEqual([e])
  })
})

describe('buildTreeIndexMap', () => {
  const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
  const grp = (id: bigint): NodeRef => ({ kind: 'group', id })
  const inst = (id: bigint): NodeRef => ({ kind: 'instance', id })

  it('indexes top-level nodes by their position in the tree, not per kind', () => {
    // Top level: [object 1n, group 2n, object 3n] — the Outliner numbers
    // rows by container position, so object 3n is index 2, not "second object".
    const map = buildTreeIndexMap([obj(1n), grp(2n), obj(3n)], () => [])
    expect(map.get(nodeKey(obj(1n)))).toBe(0)
    expect(map.get(nodeKey(grp(2n)))).toBe(1)
    expect(map.get(nodeKey(obj(3n)))).toBe(2)
  })

  it('numbers group members within the group, restarting from 0', () => {
    const members = new Map<bigint, NodeRef[]>([[2n, [inst(4n), obj(5n)]]])
    const map = buildTreeIndexMap(
      [obj(1n), grp(2n)],
      (id) => members.get(id) ?? [],
    )
    // The nested object is "Object 2" in the Outliner (position 1 in its
    // group) even though it is the second object globally too — the flat
    // object_ids() list would call it index 1 only by coincidence here; the
    // instance before it is what forces the container-relative answer.
    expect(map.get(nodeKey(inst(4n)))).toBe(0)
    expect(map.get(nodeKey(obj(5n)))).toBe(1)
  })

  it('recurses through nested groups', () => {
    const members = new Map<bigint, NodeRef[]>([
      [2n, [grp(6n)]],
      [6n, [obj(7n)]],
    ])
    const map = buildTreeIndexMap([grp(2n)], (id) => members.get(id) ?? [])
    expect(map.get(nodeKey(grp(6n)))).toBe(0)
    expect(map.get(nodeKey(obj(7n)))).toBe(0)
  })

  it('returns an empty map for an empty document', () => {
    expect(buildTreeIndexMap([], () => []).size).toBe(0)
  })
})

describe('collectDescendants', () => {
  const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
  const grp = (id: bigint): NodeRef => ({ kind: 'group', id })

  it('returns the direct children of a leaf-only container', () => {
    const out = collectDescendants([obj(1n), obj(2n)], () => [])
    expect(out).toEqual([obj(1n), obj(2n)])
  })

  it('recurses through nested groups, including the intermediate groups themselves', () => {
    const members = new Map<bigint, NodeRef[]>([
      [10n, [grp(11n)]],
      [11n, [obj(1n)]],
    ])
    const out = collectDescendants([grp(10n)], (id) => members.get(id) ?? [])
    // Every level, not just the leaves: the intermediate group (11n) is
    // itself a descendant, since it can carry its own independent hidden
    // key (App.tsx's `handleSetHiddenMany`/`toggleContainerVisibility`
    // needs exactly this to clear a hidden grandchild).
    expect(out).toEqual([grp(10n), grp(11n), obj(1n)])
  })

  it('returns an empty array for no children', () => {
    expect(collectDescendants([], () => [])).toEqual([])
  })
})

describe('filterTreeKeys', () => {
  const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
  const grp = (id: bigint): NodeRef => ({ kind: 'group', id })
  const labels = new Map<string, string>([
    [nodeKey(grp(10n)), 'Chassis'],
    [nodeKey(obj(1n)), 'Bridge Arch'],
    [nodeKey(obj(2n)), 'Wheel'],
  ])
  const getLabel = (n: NodeRef): string => labels.get(nodeKey(n)) ?? ''
  const members = new Map<bigint, NodeRef[]>([[10n, [obj(1n), obj(2n)]]])
  const getChildren = (n: NodeRef): NodeRef[] => (n.kind === 'group' ? members.get(n.id) ?? [] : [])
  const topNodes = [grp(10n)]

  it('returns null for a blank query — "no filter active"', () => {
    expect(filterTreeKeys(topNodes, getChildren, getLabel, '')).toBeNull()
    expect(filterTreeKeys(topNodes, getChildren, getLabel, '   ')).toBeNull()
  })

  it('matches case-insensitively on the same label text the row renders', () => {
    const result = filterTreeKeys(topNodes, getChildren, getLabel, 'bridge')
    expect(result).not.toBeNull()
    expect(result?.matches.has(nodeKey(obj(1n)))).toBe(true)
    expect(result?.matches.has(nodeKey(obj(2n)))).toBe(false)
  })

  it('marks a non-matching ancestor of a match as an ancestor, not a match', () => {
    const result = filterTreeKeys(topNodes, getChildren, getLabel, 'bridge')
    expect(result?.matches.has(nodeKey(grp(10n)))).toBe(false)
    expect(result?.ancestors.has(nodeKey(grp(10n)))).toBe(true)
  })

  it('a group that matches by its own name does not pull in its non-matching children', () => {
    const result = filterTreeKeys(topNodes, getChildren, getLabel, 'chassis')
    expect(result?.matches.has(nodeKey(grp(10n)))).toBe(true)
    expect(result?.matches.has(nodeKey(obj(1n)))).toBe(false)
    expect(result?.ancestors.has(nodeKey(obj(1n)))).toBe(false)
  })

  it('an empty result carries empty match/ancestor sets, not null', () => {
    const result = filterTreeKeys(topNodes, getChildren, getLabel, 'nonexistent')
    expect(result).not.toBeNull()
    expect(result?.matches.size).toBe(0)
    expect(result?.ancestors.size).toBe(0)
  })
})

describe('pruneDeadSelection — drop handles the document no longer holds', () => {
  /** Minimal liveness view over plain arrays. A drawn thing is alive while
   * its plane's `plane_units_json` listing holds it; a plane the kernel no
   * longer has throws, like its typed errors do. A drawn curve is a RUN in
   * its unit's `curves` list (the facets carrying a `curve`), keyed 0xc0. */
  function view(opts: {
    objects?: bigint[]
    groups?: bigint[]
    instances?: bigint[]
    sketches?: bigint[]
    /** ctx -> listing JSON; a ctx not in the map throws. */
    planes?: Record<string, string>
  }) {
    return {
      object_ids: () => opts.objects ?? [],
      group_ids: () => opts.groups ?? [],
      instance_ids: () => opts.instances ?? [],
      sketch_ids: () => opts.sketches ?? [],
      plane_units_json: (ctx: string) => {
        const l = opts.planes?.[ctx]
        if (l === undefined) throw new Error('StalePlane')
        return l
      },
    }
  }

  const topo = { kind: 'topology', object: 1 }
  const free = { kind: 'free', sketch: 4 }
  const line = (key: number, x: number, store: object = free) => ({ key: hex(key), a: [x, 0, 0], b: [x + 1, 0, 0], store, curve: null })
  const arc = (key: number, x: number) => ({ key: hex(key), a: [x, 0, 0], b: [x + 1, 0, 0], store: free, curve: { center: [0, 0, 0], radius: 1 } })
  const listing = (units: { key: number; edges: object[] }[]) =>
    JSON.stringify({
      units: units.map((u) => {
        const arcs = (u.edges as { key: string; curve: unknown }[]).filter((e) => e.curve)
        return {
          key: hex(u.key),
          edges: u.edges,
          curves: arcs.length === 0
            ? []
            : [{ key: hex(0xc0), curve_key: hex(0xcc), kind: 'circle', center: [0, 0, 0], radius: 1, edges: arcs.map((e) => e.key) }],
        }
      }),
      regions: [{ key: hex(30), outer: [], holes: [] }],
    })

  const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
  const grp = (id: bigint): NodeRef => ({ kind: 'group', id })
  const inst = (id: bigint): NodeRef => ({ kind: 'instance', id })
  const sk = (id: bigint): NodeRef => ({ kind: 'sketch', id })

  it('keeps live nodes of every structural kind and drops dead ones', () => {
    const v = view({ objects: [1n], groups: [2n], instances: [3n], sketches: [4n] })
    const sel = [obj(1n), obj(9n), grp(2n), grp(8n), inst(3n), inst(7n), sk(4n), sk(6n)]
    expect(pruneDeadSelection(v, sel)).toEqual([obj(1n), grp(2n), inst(3n), sk(4n)])
  })

  it('returns the SAME array when nothing died (no render churn)', () => {
    const v = view({ objects: [1n, 2n] })
    const sel = [obj(1n), obj(2n)]
    expect(pruneDeadSelection(v, sel)).toBe(sel)
  })

  it('keeps live component-definition objects and sketches supplied by the active scope', () => {
    const v = view({ planes: { [CTX1]: listing([{ key: 20, edges: [line(13, 0)] }]) } })
    const sel: NodeRef[] = [
      obj(11n),
      sk(12n),
      dr('draft-edge', CTX1, 13),
    ]
    expect(pruneDeadSelection(v, sel, [11n], [12n])).toBe(sel)
    // Without the scope the scoped object and sketch are dead; a drawn edge
    // lives by its plane's listing, not by sketch scope, so it stays.
    expect(pruneDeadSelection(v, sel)).toEqual([dr('draft-edge', CTX1, 13)])
  })

  it('prunes drawn kinds: dead edges, curves, islands, and regions go; live ones stay', () => {
    const v = view({
      planes: {
        [CTX1]: listing([
          { key: 20, edges: [line(10, 0), arc(14, 5), arc(15, 6)] },
        ]),
      },
    })
    const sel: NodeRef[] = [
      dr('draft-edge', CTX1, 10),
      dr('draft-edge', CTX1, 11),     // dead edge
      dr('draft-edge', CTX2, 10),     // dead plane (listing throws)
      { kind: 'draft-curve', id: BigInt(0xc0), ctx: CTX1, key: hex(0xc0) },
      { kind: 'draft-curve', id: 12n, ctx: CTX1, key: hex(12) },  // dead curve (no facet carries it)
      dr('draft-island', CTX1, 20),
      dr('draft-island', CTX1, 21),   // dead island
      dr('draft-region', CTX1, 30),
      dr('draft-region', CTX1, 31),   // dead region
    ]
    expect(pruneDeadSelection(v, sel)).toEqual([
      dr('draft-edge', CTX1, 10),
      { kind: 'draft-curve', id: BigInt(0xc0), ctx: CTX1, key: hex(0xc0) },
      dr('draft-island', CTX1, 20),
      dr('draft-region', CTX1, 30),
    ])
  })

  it('a draft-curve keyed by the circle\'s curve key (a double-click widening) lives while any of its runs does', () => {
    const v = view({ planes: { [CTX1]: listing([{ key: 20, edges: [line(10, 0), arc(14, 5), arc(15, 6)] }]) } })
    const whole: NodeRef = { kind: 'draft-curve', id: BigInt(0xcc), ctx: CTX1, key: hex(0xcc) }
    const sel: NodeRef[] = [whole]
    expect(pruneDeadSelection(v, sel)).toBe(sel)
    const gone = view({ planes: { [CTX1]: listing([{ key: 20, edges: [line(10, 0)] }]) } })
    expect(pruneDeadSelection(gone, sel)).toEqual([])
  })

  it('an empty selection passes through untouched', () => {
    const sel: NodeRef[] = []
    expect(pruneDeadSelection(view({}), sel)).toBe(sel)
  })

  describe('draft liveness (was imprint liveness)', () => {
    // A drawn thing is alive exactly while its plane still lists its key,
    // whichever store (topology, owned, free) its edges live in.
    const onFace = listing([{ key: 10, edges: [line(1, 0, topo), line(2, 1, topo)] }])

    it('keeps a live island whose unit key is still listed', () => {
      const v = view({ objects: [1n], planes: { [CTX1]: onFace } })
      const sel: NodeRef[] = [dr('draft-island', CTX1, 10)]
      expect(pruneDeadSelection(v, sel)).toBe(sel)
    })

    it('keeps a live line (a chord) whose edge key is still listed', () => {
      const v = view({ objects: [1n], planes: { [CTX1]: onFace } })
      const sel: NodeRef[] = [dr('draft-edge', CTX1, 2)]
      expect(pruneDeadSelection(v, sel)).toBe(sel)
    })

    it('drops a ref whose key is no longer listed (pushed into a boss, or a chord re-cut)', () => {
      const v = view({ objects: [1n], planes: { [CTX1]: onFace } })
      const sel: NodeRef[] = [dr('draft-island', CTX1, 10), dr('draft-island', CTX1, 999)]
      expect(pruneDeadSelection(v, sel)).toEqual([dr('draft-island', CTX1, 10)])
    })

    it('drops a ref whose plane is gone', () => {
      const v = view({ objects: [], planes: {} })
      const sel: NodeRef[] = [dr('draft-island', CTX1, 10)]
      expect(pruneDeadSelection(v, sel)).toEqual([])
    })

    it('drops every draft ref when the scene double lacks plane_units_json entirely', () => {
      const bare = {
        object_ids: () => [1n],
        group_ids: () => [],
        instance_ids: () => [],
        sketch_ids: () => [],
        // plane_units_json intentionally omitted.
      }
      const sel: NodeRef[] = [dr('draft-island', CTX1, 10)]
      expect(pruneDeadSelection(bare, sel)).toEqual([])
    })
  })
})

describe('dropTargetFor', () => {
  // g contains b and h (nested); h contains c.
  const a: NodeRef = { kind: 'object', id: 1n }
  const b: NodeRef = { kind: 'object', id: 2n }
  const c: NodeRef = { kind: 'object', id: 3n }
  const g: NodeRef = { kind: 'group', id: 10n }
  const h: NodeRef = { kind: 'group', id: 11n }
  const inst: NodeRef = { kind: 'instance', id: 20n }
  const sk = dr('draft-island', CTX1, 30)

  const members: Record<string, NodeRef[]> = {
    '10': [b, h],
    '11': [c],
  }
  const getGroupMembers = (groupId: bigint): NodeRef[] => members[groupId.toString()] ?? []
  const view = (sessionOpen = false) => ({ getGroupMembers, sessionOpen })

  it('moving an object into a group resolves to that group', () => {
    expect(dropTargetFor([a], g, view())).toEqual({ group: 10n })
  })

  it('moving to the Model root resolves to undefined (top level)', () => {
    expect(dropTargetFor([a], 'root', view())).toEqual({ group: undefined })
  })

  it('refuses dropping a group onto itself', () => {
    expect(dropTargetFor([g], g, view())).toBeNull()
  })

  it('refuses dropping a group onto its own descendant', () => {
    expect(dropTargetFor([g], h, view())).toBeNull()
  })

  it('allows dropping an already-nested node back onto its current parent (a no-op move)', () => {
    expect(dropTargetFor([h], g, view())).toEqual({ group: 10n })
  })

  it('refuses a target that is neither a group nor root (an instance row)', () => {
    expect(dropTargetFor([a], inst, view())).toBeNull()
  })

  it('refuses a target that is a sketch row', () => {
    expect(dropTargetFor([a], sk, view())).toBeNull()
  })

  it('refuses dragging a sketch-scoped node — it has no kernel NodeId', () => {
    expect(dropTargetFor([sk], g, view())).toBeNull()
  })

  it('refuses every drop while a group/component session is open', () => {
    expect(dropTargetFor([a], g, view(true))).toBeNull()
    expect(dropTargetFor([a], 'root', view(true))).toBeNull()
  })

  it('refuses an empty drag', () => {
    expect(dropTargetFor([], g, view())).toBeNull()
  })

  it('refuses a multi-node drag when one of the dragged groups would contain the target', () => {
    expect(dropTargetFor([a, g], h, view())).toBeNull()
  })
})

describe('treeIndexOf (group session label)', () => {
  const grp = (id: bigint): NodeRef => ({ kind: 'group', id })

  it('two unnamed groups: opening the second reads "Group 2", not "Group 1"', () => {
    const top = [grp(10n), grp(20n)]
    const index = treeIndexOf(grp(20n), top, () => [])
    expect(resolveLabel(undefined, undefined, 'group', index)).toBe('Group 2')
    expect(resolveLabel(undefined, undefined, 'group', treeIndexOf(grp(10n), top, () => []))).toBe('Group 1')
  })

  it('counts a nested group by its place inside its parent', () => {
    const members = new Map<bigint, NodeRef[]>([[10n, [{ kind: 'object', id: 1n }, grp(30n)]]])
    expect(treeIndexOf(grp(30n), [grp(10n)], (id) => members.get(id) ?? [])).toBe(1)
  })

  it('returns null for a node that is not in the tree', () => {
    expect(treeIndexOf(grp(99n), [grp(10n)], () => [])).toBeNull()
  })

  it('an unplaced unnamed group is labeled "Group", never a wrong number', () => {
    const index = treeIndexOf(grp(99n), [grp(10n)], () => [])
    expect(resolveLabel(undefined, undefined, 'group', index)).toBe('Group')
  })
})

describe('makeLabelFor — sketches', () => {
  const scene = (ids: bigint[]) => ({
    group_name: () => undefined,
    object_name: () => undefined,
    instance_name: () => undefined,
    instance_def: () => undefined,
    component_name: () => undefined,
    sketch_ids: () => ids,
  })
  it('numbers a listed sketch by its position', () => {
    const label = makeLabelFor(scene([4n, 7n]), new Map())
    expect(label({ kind: 'sketch', id: 7n })).toBe('Sketch 2')
  })
  it('an unplaced sketch is "Sketch", never a wrong "Sketch 1"', () => {
    const label = makeLabelFor(scene([4n]), new Map())
    expect(label({ kind: 'sketch', id: 99n })).toBe('Sketch')
  })
})

describe('placementLabel', () => {
  const scene = (opts: {
    linked: boolean
    instanceName?: string
    defName?: string
    members?: { id: bigint; name?: string }[]
  }): LabelScene => ({
    group_name: () => undefined,
    object_name: (id) => opts.members?.find((m) => m.id === id)?.name,
    instance_name: () => opts.instanceName,
    instance_def: () => 100n,
    component_name: () => opts.defName,
    is_linked: () => opts.linked,
    component_member_objects: () => (opts.members ?? []).map((m) => m.id),
  })

  it('placementLabel_prefers_instance_then_definition_then_single_member_then_copies', () => {
    const members = [{ id: 5n, name: 'Leg' }]
    expect(placementLabel(scene({ linked: true, instanceName: 'Front leg', defName: 'Leg set', members }), 1n, 0)).toBe('Front leg')
    expect(placementLabel(scene({ linked: true, defName: 'Leg set', members }), 1n, 0)).toBe('Leg set')
    expect(placementLabel(scene({ linked: true, members }), 1n, 0)).toBe('Leg')
    expect(placementLabel(scene({ linked: true, members: [{ id: 5n }] }), 1n, 0)).toBe('Copies')
    expect(placementLabel(scene({ linked: true, members: [{ id: 5n, name: 'A' }, { id: 6n, name: 'B' }] }), 1n, 0)).toBe('Copies')
  })

  it('an_ordinary_component_keeps_its_label', () => {
    const s = scene({ linked: false, instanceName: 'Left', defName: 'Chair' })
    expect(placementLabel(s, 1n, 0)).toBe('Left (Chair)')
    expect(placementLabel(scene({ linked: false, defName: 'Chair' }), 1n, 0)).toBe('Chair')
    expect(placementLabel(scene({ linked: false }), 1n, 2)).toBe(resolveLabel(undefined, undefined, 'instance', 2))
  })
})
