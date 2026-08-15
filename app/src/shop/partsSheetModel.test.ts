import { describe, it, expect } from 'vitest'
import { buildPartsSheetSections, totalPartCount, visiblePartCount } from './partsSheetModel'
import type { Scene as WasmScene } from '../wasm/loader'
import { nodeKey } from '../panels/treeModel'
import { tagPathKey } from '../panels/tagModel'

/** A mesh stand-in matching objectBounds.test.ts's `makeMesh`. */
function makeMesh(positions: Float32Array) {
  return { positions: () => positions, free: () => {} }
}

/** A fake Scene exposing only what `buildPartsSheetSections` (and the
 *  `unionHiddenLeafIds`/`worldBoundsForSelection` it delegates to) read. */
function makeScene(overrides: Partial<{
  object_ids: bigint[]
  group_ids: bigint[]
  instance_ids: bigint[]
  top_level_nodes: { kind: string; id: bigint }[]
  group_members: Record<string, { kind: string; id: bigint }[]>
  object_name: Record<string, string>
  group_name: Record<string, string>
  instance_name: Record<string, string>
  instance_def: Record<string, bigint>
  component_name: Record<string, string>
  node_tags: Record<string, string[]>
  object_mesh: Record<string, Float32Array>
  tag_meta_paths: string[]
}> = {}): WasmScene {
  return {
    object_ids: () => overrides.object_ids ?? [],
    group_ids: () => overrides.group_ids ?? [],
    instance_ids: () => overrides.instance_ids ?? [],
    top_level_nodes: () => overrides.top_level_nodes ?? [],
    group_members: (id: bigint) => overrides.group_members?.[String(id)] ?? [],
    object_name: (id: bigint) => overrides.object_name?.[String(id)],
    group_name: (id: bigint) => overrides.group_name?.[String(id)],
    instance_name: (id: bigint) => overrides.instance_name?.[String(id)],
    instance_def: (id: bigint) => overrides.instance_def?.[String(id)],
    component_name: (id: bigint) => overrides.component_name?.[String(id)],
    node_tags: (_kind: number, id: bigint) => overrides.node_tags?.[String(id)] ?? [],
    object_mesh: (id: bigint) => {
      const positions = overrides.object_mesh?.[String(id)]
      if (positions === undefined) throw new Error('UnknownObject')
      return makeMesh(positions)
    },
    tag_meta_paths: () => overrides.tag_meta_paths ?? [],
  } as unknown as WasmScene
}

const NOSET = new Set<string>()

describe('buildPartsSheetSections — empty document', () => {
  it('returns no sections at all', () => {
    expect(buildPartsSheetSections(makeScene(), NOSET, NOSET)).toEqual([])
  })
})

describe('buildPartsSheetSections — untagged document', () => {
  it('puts every top-level part in one "Parts" catch-all section', () => {
    const scene = makeScene({
      object_ids: [1n],
      top_level_nodes: [{ kind: 'object', id: 1n }],
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 2, 3]) },
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    expect(sections).toHaveLength(1)
    expect(sections[0]).toMatchObject({ key: '__parts-sheet-unfiled__', label: 'Parts', path: null, hidden: false })
    expect(sections[0].rows).toEqual([
      { node: { kind: 'object', id: 1n }, label: 'Object 1', depth: 0, extentsM: [1, 2, 3], hidden: false },
    ])
  })

  it('excludes a sketch node from top_level_nodes defensively', () => {
    const scene = makeScene({ top_level_nodes: [{ kind: 'sketch', id: 1n }] })
    expect(buildPartsSheetSections(scene, NOSET, NOSET)).toEqual([])
  })
})

describe('buildPartsSheetSections — tag sections', () => {
  it('splits tagged and untagged top-level nodes into a tag section plus "Unfiled"', () => {
    const scene = makeScene({
      object_ids: [1n, 2n],
      top_level_nodes: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }],
      node_tags: { '1': ['Roof'] },
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]), '2': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    expect(sections.map((s) => s.label)).toEqual(['Roof', 'Unfiled'])
    expect(sections[0].rows.map((r) => r.node.id)).toEqual([1n])
    expect(sections[1].rows.map((r) => r.node.id)).toEqual([2n])
  })

  it('flattens a nested tag path into its own section, distinct from its parent', () => {
    const scene = makeScene({
      object_ids: [1n],
      top_level_nodes: [{ kind: 'object', id: 1n }],
      node_tags: { '1': ['Structure/Roof'] },
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    // Two sections: the empty "Structure" parent (no node tagged there
    // exactly) and "Structure / Roof" (the actual tagged node).
    expect(sections.map((s) => s.label)).toEqual(['Structure', 'Structure / Roof'])
    expect(sections[0].rows).toEqual([])
    expect(sections[1].rows.map((r) => r.node.id)).toEqual([1n])
    expect(sections[1].key).toBe(tagPathKey(['Structure', 'Roof']))
  })

  it('a section\'s master-toggle hidden state reads hiddenTagPaths directly', () => {
    const scene = makeScene({
      object_ids: [1n],
      top_level_nodes: [{ kind: 'object', id: 1n }],
      node_tags: { '1': ['Roof'] },
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const hiddenTagPaths = new Set([tagPathKey(['Roof'])])
    const sections = buildPartsSheetSections(scene, NOSET, hiddenTagPaths)
    expect(sections[0].hidden).toBe(true)
    // Hiding the tag also hides every row under it (unionHiddenLeafIds).
    expect(sections[0].rows[0].hidden).toBe(true)
  })

  it('gives no untagged document a superfluous "Unfiled" label — only "Parts" when nothing is tagged', () => {
    const scene = makeScene({
      object_ids: [1n],
      top_level_nodes: [{ kind: 'object', id: 1n }],
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    expect(buildPartsSheetSections(scene, NOSET, NOSET)[0].label).toBe('Parts')
  })
})

describe('buildPartsSheetSections — group nesting', () => {
  it('indents a group\'s members one level deeper, recursively', () => {
    const scene = makeScene({
      group_ids: [100n],
      object_ids: [1n, 2n],
      top_level_nodes: [{ kind: 'group', id: 100n }],
      group_members: {
        '100': [{ kind: 'object', id: 1n }, { kind: 'group', id: 101n }],
        '101': [{ kind: 'object', id: 2n }],
      },
      group_name: { '100': 'Frame' },
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]), '2': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    expect(sections).toHaveLength(1)
    const rows = sections[0].rows
    expect(rows.map((r) => ({ id: r.node.id, kind: r.node.kind, depth: r.depth }))).toEqual([
      { id: 100n, kind: 'group', depth: 0 },
      { id: 1n, kind: 'object', depth: 1 },
      { id: 101n, kind: 'group', depth: 1 },
      { id: 2n, kind: 'object', depth: 2 },
    ])
  })

  it('a group reads hidden only once every one of its leaves is hidden', () => {
    const scene = makeScene({
      group_ids: [100n],
      object_ids: [1n, 2n],
      top_level_nodes: [{ kind: 'group', id: 100n }],
      group_members: { '100': [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }] },
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]), '2': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const oneHidden = new Set([nodeKey({ kind: 'object', id: 1n })])
    const bothHidden = new Set([nodeKey({ kind: 'object', id: 1n }), nodeKey({ kind: 'object', id: 2n })])

    const partial = buildPartsSheetSections(scene, oneHidden, NOSET)
    expect(partial[0].rows[0].hidden).toBe(false) // the group row itself

    const full = buildPartsSheetSections(scene, bothHidden, NOSET)
    expect(full[0].rows[0].hidden).toBe(true)
  })

  it('an empty group never reads hidden', () => {
    const scene = makeScene({
      group_ids: [100n],
      top_level_nodes: [{ kind: 'group', id: 100n }],
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    expect(sections[0].rows[0].hidden).toBe(false)
  })
})

describe('buildPartsSheetSections — instance is a leaf', () => {
  it('never recurses into an instance\'s definition members', () => {
    const scene = {
      ...makeScene({
        instance_ids: [10n],
        top_level_nodes: [{ kind: 'instance', id: 10n }],
        instance_name: { '10': 'Leg' },
      }),
      instance_pose: () => undefined,
    } as unknown as WasmScene
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    expect(sections[0].rows).toHaveLength(1)
    expect(sections[0].rows[0]).toMatchObject({ node: { kind: 'instance', id: 10n }, label: 'Leg' })
  })
})

describe('buildPartsSheetSections — isolate integration', () => {
  it('unions isolate\'s complement into every OTHER row\'s hidden flag, leaving the isolated node itself visible', () => {
    const scene = makeScene({
      object_ids: [1n, 2n, 3n],
      top_level_nodes: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }, { kind: 'object', id: 3n }],
      object_mesh: {
        '1': new Float32Array([0, 0, 0, 1, 1, 1]),
        '2': new Float32Array([0, 0, 0, 1, 1, 1]),
        '3': new Float32Array([0, 0, 0, 1, 1, 1]),
      },
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET, { kind: 'object', id: 1n })
    const hiddenById = new Map(sections[0].rows.map((r) => [r.node.id, r.hidden]))
    expect(hiddenById.get(1n)).toBe(false)
    expect(hiddenById.get(2n)).toBe(true)
    expect(hiddenById.get(3n)).toBe(true)
  })

  it('composes with a sheet-driven hide rather than replacing it — the manually-hidden part stays hidden after isolate ends', () => {
    const scene = makeScene({
      object_ids: [1n, 2n],
      top_level_nodes: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }],
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]), '2': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const hiddenKeys = new Set([nodeKey({ kind: 'object', id: 2n })])
    // Isolating object 1 while object 2 is already sheet-hidden: still hidden either way.
    const whileIsolating = buildPartsSheetSections(scene, hiddenKeys, NOSET, { kind: 'object', id: 1n })
    expect(whileIsolating[0].rows.find((r) => r.node.id === 2n)?.hidden).toBe(true)
    // isolatedNode omitted (isolate ended): object 2 is STILL hidden — the sheet's own hide, untouched by isolate ending.
    const afterShowAll = buildPartsSheetSections(scene, hiddenKeys, NOSET)
    expect(afterShowAll[0].rows.find((r) => r.node.id === 2n)?.hidden).toBe(true)
    expect(afterShowAll[0].rows.find((r) => r.node.id === 1n)?.hidden).toBe(false)
  })
})

describe('totalPartCount', () => {
  it('sums rows across every section', () => {
    const scene = makeScene({
      object_ids: [1n, 2n],
      top_level_nodes: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }],
      node_tags: { '1': ['Roof'] },
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]), '2': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    expect(totalPartCount(sections)).toBe(2)
  })

  it('is zero for an empty document', () => {
    expect(totalPartCount(buildPartsSheetSections(makeScene(), NOSET, NOSET))).toBe(0)
  })
})

describe('visiblePartCount', () => {
  it('equals totalPartCount when nothing is hidden', () => {
    const scene = makeScene({
      object_ids: [1n, 2n],
      top_level_nodes: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }],
      object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]), '2': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const sections = buildPartsSheetSections(scene, NOSET, NOSET)
    expect(visiblePartCount(sections)).toBe(totalPartCount(sections))
    expect(visiblePartCount(sections)).toBe(2)
  })

  it('excludes rows hidden by a manual hide, a tag hide, or isolate', () => {
    const scene = makeScene({
      object_ids: [1n, 2n, 3n],
      top_level_nodes: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }, { kind: 'object', id: 3n }],
      object_mesh: {
        '1': new Float32Array([0, 0, 0, 1, 1, 1]),
        '2': new Float32Array([0, 0, 0, 1, 1, 1]),
        '3': new Float32Array([0, 0, 0, 1, 1, 1]),
      },
    })
    const hiddenKeys = new Set([nodeKey({ kind: 'object', id: 2n })])
    const sections = buildPartsSheetSections(scene, hiddenKeys, NOSET, { kind: 'object', id: 1n })
    // Object 1 is isolated (stays visible), 2 is manually hidden, 3 is
    // hidden by isolate's own complement — only 1 of 3 reads visible.
    expect(totalPartCount(sections)).toBe(3)
    expect(visiblePartCount(sections)).toBe(1)
  })

  it('is zero for an empty document', () => {
    expect(visiblePartCount(buildPartsSheetSections(makeScene(), NOSET, NOSET))).toBe(0)
  })
})
