import { describe, it, expect } from 'vitest'
import {
  entityLabel,
  resolveLabel,
  breadcrumb,
  isTreeRowDimmed,
  nextSelection,
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
  stripTagSuffix,
  collectLeafIds,
  buildTreeIndexMap,
  nodeKey,
  structuralSelection,
  pruneDeadSelection,
  type NodeRef,
} from './treeModel'

describe('nodeEq / nodeKey — sketch-edge scoping', () => {
  it('two edges with the same id in DIFFERENT sketches are distinct', () => {
    const a: NodeRef = { kind: 'sketch-edge', id: 5n, sketch: 1n }
    const b: NodeRef = { kind: 'sketch-edge', id: 5n, sketch: 2n }
    expect(nodeEq(a, b)).toBe(false)
    expect(nodeKey(a)).not.toBe(nodeKey(b))
  })

  it('the same edge equals itself and keys stably', () => {
    const a: NodeRef = { kind: 'sketch-edge', id: 5n, sketch: 1n }
    const b: NodeRef = { kind: 'sketch-edge', id: 5n, sketch: 1n }
    expect(nodeEq(a, b)).toBe(true)
    expect(nodeKey(a)).toBe(nodeKey(b))
  })

  it('an edge never equals its owning sketch or an object with the same id', () => {
    const edge: NodeRef = { kind: 'sketch-edge', id: 5n, sketch: 1n }
    expect(nodeEq(edge, { kind: 'sketch', id: 1n })).toBe(false)
    expect(nodeEq(edge, { kind: 'object', id: 5n })).toBe(false)
    expect(nodeKey(edge)).not.toBe(nodeKey({ kind: 'sketch', id: 1n }))
  })

  it('plain node keys are unchanged by the optional sketch field being absent', () => {
    expect(nodeKey({ kind: 'object', id: 3n })).toBe('object:3')
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

  // Sketch-scoped NodeRefs have no kernel NodeId: letting one through the
  // gate forwards its id into the object handle space downstream, where the
  // slotmaps' reused bit patterns can silently alias an unrelated live node.
  it('false for any sketch-kind selection (no kernel NodeId — id-space guard)', () => {
    const sk: NodeRef = { kind: 'sketch', id: 5n }
    const island: NodeRef = { kind: 'sketch-island', id: 6n, sketch: 5n }
    const edge: NodeRef = { kind: 'sketch-edge', id: 7n, sketch: 5n }
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
    const curve: NodeRef = { kind: 'sketch-curve', id: 6n, sketch: 5n }
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
      { kind: 'sketch-island', id: 6n, sketch: 5n },
    ])).toBeNull()
    expect(structuralSelection([
      { kind: 'sketch-edge', id: 7n, sketch: 5n },
    ])).toBeNull()
    expect(structuralSelection([
      { kind: 'sketch-curve', id: 8n, sketch: 5n },
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

  it('true for two top-level objects, object+group, and two groups', () => {
    expect(canBoolean([a, b], topLevel, live)).toBe(true)
    expect(canBoolean([a, g], topLevel, live)).toBe(true)
    expect(canBoolean([g, { kind: 'group', id: 5n }], topLevel, live)).toBe(true)
  })

  it('requires exactly two distinct operands', () => {
    expect(canBoolean([a], topLevel, live)).toBe(false)
    expect(canBoolean([a, a], topLevel, live)).toBe(false)
    expect(canBoolean([a, b, g], topLevel, live)).toBe(false)
  })

  it('false when either operand is NESTED inside a group — the gate must match the kernel GroupedOperand refusal', () => {
    const nestedFirst = (n: NodeRef) => (n.id === 1n ? 99n : undefined)
    expect(canBoolean([a, b], nestedFirst, live)).toBe(false)
    const nestedSecond = (n: NodeRef) => (n.id === 3n ? 99n : undefined)
    expect(canBoolean([a, g], nestedSecond, live)).toBe(false)
    const bothNested = (_n: NodeRef) => 99n
    expect(canBoolean([a, b], bothNested, live)).toBe(false)
  })

  it('false for instances and non-live operands', () => {
    expect(canBoolean([a, inst], topLevel, live)).toBe(false)
    const bStale = (n: NodeRef) => n.id !== 2n
    expect(canBoolean([a, b], topLevel, bStale)).toBe(false)
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

describe('pruneDeadSelection — drop handles the document no longer holds', () => {
  /** Minimal liveness view over plain arrays; sketch sub-entity queries
   * throw on a dead sketch like the kernel's typed errors do. */
  function view(opts: {
    objects?: bigint[]
    groups?: bigint[]
    instances?: bigint[]
    sketches?: bigint[]
    edges?: Record<string, bigint[]>       // sketch id -> live edge ids
    islands?: Record<string, bigint[]>     // sketch id -> live island ids
  }) {
    const sketches = opts.sketches ?? []
    return {
      object_ids: () => opts.objects ?? [],
      group_ids: () => opts.groups ?? [],
      instance_ids: () => opts.instances ?? [],
      sketch_ids: () => sketches,
      sketch_edge_endpoints: (s: bigint, e: bigint) =>
        (opts.edges?.[s.toString()] ?? []).includes(e) ? new Float64Array(6) : undefined,
      sketch_curve_chain: (s: bigint, e: bigint) => {
        if (!sketches.includes(s)) throw new Error('UnknownSketch')
        const live = opts.edges?.[s.toString()] ?? []
        if (!live.includes(e)) throw new Error('UnknownEdge')
        return BigUint64Array.from([e])
      },
      sketch_island_edges: (s: bigint, i: bigint) => {
        if (!sketches.includes(s)) throw new Error('UnknownSketch')
        const live = opts.islands?.[s.toString()] ?? []
        if (!live.includes(i)) throw new Error('UnknownIsland')
        return BigUint64Array.from([1n])
      },
    }
  }

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

  it('prunes sketch-scoped kinds: dead edges, curves, and islands go; live ones stay', () => {
    const v = view({ sketches: [4n], edges: { '4': [10n] }, islands: { '4': [20n] } })
    const sel: NodeRef[] = [
      { kind: 'sketch-edge', id: 10n, sketch: 4n },
      { kind: 'sketch-edge', id: 11n, sketch: 4n },   // dead edge
      { kind: 'sketch-edge', id: 10n, sketch: 5n },   // dead sketch
      { kind: 'sketch-curve', id: 10n, sketch: 4n },
      { kind: 'sketch-curve', id: 12n, sketch: 4n },  // dead (throws)
      { kind: 'sketch-island', id: 20n, sketch: 4n },
      { kind: 'sketch-island', id: 21n, sketch: 4n }, // dead (throws)
    ]
    expect(pruneDeadSelection(v, sel)).toEqual([
      { kind: 'sketch-edge', id: 10n, sketch: 4n },
      { kind: 'sketch-curve', id: 10n, sketch: 4n },
      { kind: 'sketch-island', id: 20n, sketch: 4n },
    ])
  })

  it('an empty selection passes through untouched', () => {
    const sel: NodeRef[] = []
    expect(pruneDeadSelection(view({}), sel)).toBe(sel)
  })
})
