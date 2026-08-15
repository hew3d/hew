import { describe, it, expect } from 'vitest'
import { resolveInspect, isWholePartKind } from './inspect'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Snap } from '../tools/types'
import type { NodeRef } from '../panels/treeModel'

/** A mesh stand-in matching objectBounds.test.ts's `makeMesh` — only
 *  `positions`/`free` matter to `meshWorldBounds`. */
function makeMesh(positions: Float32Array) {
  return { positions: () => positions, free: () => {} }
}

/** A fake Scene exposing only what `resolveInspect` (and the
 *  `worldBoundsForSelection` it delegates the node branch to) read. */
function makeScene(overrides: Partial<{
  object_name: Record<string, string>
  group_name: Record<string, string>
  instance_name: Record<string, string>
  instance_def: Record<string, bigint>
  component_name: Record<string, string>
  top_level_nodes: { kind: string; id: bigint }[]
  group_members: Record<string, { kind: string; id: bigint }[]>
  node_tags: Record<string, string[]>
  object_mesh: Record<string, Float32Array>
  edge_endpoints: Float64Array | undefined
  edge_endpoints_in_instance: Float64Array | undefined
  face_boundary: Float32Array
  instance_pose: Record<string, ArrayLike<number>>
  instance_expanded_members: Record<string, bigint[]>
  instance_expanded_local_poses: Record<string, Float64Array>
}> = {}): WasmScene {
  return {
    object_name: (id: bigint) => overrides.object_name?.[String(id)],
    group_name: (id: bigint) => overrides.group_name?.[String(id)],
    instance_name: (id: bigint) => overrides.instance_name?.[String(id)],
    instance_def: (id: bigint) => overrides.instance_def?.[String(id)],
    component_name: (id: bigint) => overrides.component_name?.[String(id)],
    top_level_nodes: () => overrides.top_level_nodes ?? [],
    group_members: (id: bigint) => overrides.group_members?.[String(id)] ?? [],
    node_tags: (_kind: number, id: bigint) => overrides.node_tags?.[String(id)] ?? [],
    object_mesh: (id: bigint) => {
      const positions = overrides.object_mesh?.[String(id)]
      if (positions === undefined) throw new Error('UnknownObject')
      return makeMesh(positions)
    },
    edge_endpoints: () => overrides.edge_endpoints,
    edge_endpoints_in_instance: () => overrides.edge_endpoints_in_instance,
    face_boundary: () => overrides.face_boundary ?? new Float32Array(),
    instance_pose: (id: bigint) => overrides.instance_pose?.[String(id)],
    instance_expanded_members: (id: bigint) => overrides.instance_expanded_members?.[String(id)] ?? [],
    instance_expanded_local_poses: (id: bigint) => overrides.instance_expanded_local_poses?.[String(id)] ?? new Float64Array(),
  } as unknown as WasmScene
}

const OBJECT_NODE: NodeRef = { kind: 'object', id: 1n }

describe('isWholePartKind', () => {
  it('accepts object/group/instance and rejects sketch kinds', () => {
    expect(isWholePartKind('object')).toBe(true)
    expect(isWholePartKind('group')).toBe(true)
    expect(isWholePartKind('instance')).toBe(true)
    expect(isWholePartKind('sketch')).toBe(false)
    expect(isWholePartKind('sketch-edge')).toBe(false)
  })
})

describe('resolveInspect — empty tap', () => {
  it('returns null when neither a node nor a snap resolved', () => {
    const scene = makeScene()
    expect(resolveInspect(scene, null, null)).toBeNull()
  })
})

describe('resolveInspect — edge', () => {
  it('reports the exact endpoint-to-endpoint distance, with the owning part\'s label (never bare "Edge")', () => {
    const scene = makeScene({
      edge_endpoints: new Float64Array([0, 0, 0, 3, 4, 0]),
      object_name: { '1': 'Pen Cup' },
    })
    const snap: Snap = { x: 1, y: 1, z: 0, kind: 'on-edge', elementKind: 'edge', object: 5n, element: 9n }
    expect(resolveInspect(scene, OBJECT_NODE, snap)).toEqual({ kind: 'edge', lengthM: 5, partLabel: 'Pen Cup' })
  })

  it('routes through edge_endpoints_in_instance when the snap carries an instance', () => {
    const scene = makeScene({
      edge_endpoints_in_instance: new Float64Array([0, 0, 0, 0, 0, 2]),
      object_name: { '1': 'Tabletop' },
    })
    const snap: Snap = {
      x: 0, y: 0, z: 1, kind: 'on-edge', elementKind: 'edge', object: 5n, element: 9n, instance: 7n,
    }
    expect(resolveInspect(scene, OBJECT_NODE, snap)).toEqual({ kind: 'edge', lengthM: 2, partLabel: 'Tabletop' })
  })

  it('dismisses (no phantom edge card) when the edge resolved to no whole-part node', () => {
    // shop-mode playtest finding: an edge snap whose owning object did NOT
    // resolve to a selectable node — most importantly a HIDDEN object, which
    // Viewport's pick resolver rejects to `null` — must NOT produce a bare
    // "Part" edge card dimensioning geometry the user can't see. A null node
    // now dismisses, exactly like an empty tap.
    const scene = makeScene({ edge_endpoints: new Float64Array([0, 0, 0, 1, 0, 0]) })
    const snap: Snap = { x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 5n, element: 9n }
    expect(resolveInspect(scene, null, snap)).toBeNull()
  })

  it('falls back to the node card when the edge handle is stale', () => {
    const scene = makeScene({ object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]) } })
    const snap: Snap = { x: 0, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 5n, element: 9n }
    const result = resolveInspect(scene, OBJECT_NODE, snap)
    expect(result?.kind).toBe('node')
  })
})

describe('resolveInspect — face taps', () => {
  // Face hits deliberately do NOT get a card of their own (see
  // resolveInspect's comment): with the widened touch aperture the middle
  // of any part is a face hit, so face precedence would make the
  // whole-part card unreachable from the viewport.
  it('resolves a face hit to the whole node card', () => {
    const scene = makeScene({ object_mesh: { '1': new Float32Array([0, 0, 0, 1, 1, 1]) } })
    const snap: Snap = { x: 1, y: 1, z: 0, kind: 'on-face', elementKind: 'face', object: 5n, element: 2n }
    expect(resolveInspect(scene, OBJECT_NODE, snap)?.kind).toBe('node')
  })

  it('resolves a face hit with no node to null (empty tap)', () => {
    const scene = makeScene()
    const snap: Snap = { x: 1, y: 1, z: 0, kind: 'on-face', elementKind: 'face', object: 5n, element: 2n }
    expect(resolveInspect(scene, null, snap)).toBeNull()
  })
})

describe('resolveInspect — node', () => {
  it('uses the kernel name and first tag when present', () => {
    const scene = makeScene({
      object_name: { '1': 'Leg' },
      node_tags: { '1': ['Legs / Front', 'Painted'] },
      object_mesh: { '1': new Float32Array([0, 0, 0, 0.25, 0.25, 0.5]) },
    })
    const result = resolveInspect(scene, OBJECT_NODE, null)
    expect(result).toEqual({
      kind: 'node',
      node: OBJECT_NODE,
      label: 'Leg',
      tagLabel: 'Legs / Front',
      extentsM: [0.25, 0.25, 0.5],
    })
  })

  it('falls back to a positional label when the kernel name is empty, using the tree position', () => {
    const scene = makeScene({
      top_level_nodes: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }],
      object_mesh: { '2': new Float32Array([0, 0, 0, 1, 1, 1]) },
    })
    const result = resolveInspect(scene, { kind: 'object', id: 2n }, null)
    expect(result?.kind).toBe('node')
    if (result?.kind === 'node') {
      expect(result.label).toBe('Object 2')
      expect(result.tagLabel).toBeNull()
    }
  })

  it('extentsM is null for a node whose mesh contributes nothing (e.g. deleted-but-still-selected)', () => {
    const scene = makeScene()
    const result = resolveInspect(scene, OBJECT_NODE, null)
    expect(result).toMatchObject({ kind: 'node', extentsM: null })
  })

  it('never builds a node card for a sketch-scoped kind', () => {
    const scene = makeScene()
    expect(resolveInspect(scene, { kind: 'sketch', id: 1n }, null)).toBeNull()
  })
})
