import { describe, expect, it } from 'vitest'
import type { Scene as WasmScene } from '../wasm/loader'
import { cutListRows } from './cutList'

/** A mesh stand-in (objectBounds.test.ts's `makeMesh`). */
function makeMesh(positions: Float32Array) {
  return { positions: () => positions, free: () => {} }
}
/** A 1 × 2 × 3 m box's corner positions. */
const BOX = new Float32Array([0, 0, 0, 1, 2, 3])

/** The subset of Scene `buildPartsSheetSections` reads (partsSheetModel.test.ts's fake). */
const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
/** A 90° turn about Z (x → y, y → −x). */
const TURNED = [0, -1, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0]

function makeScene(o: {
  object_ids?: bigint[]
  group_ids?: bigint[]
  instance_ids?: bigint[]
  top_level_nodes?: { kind: string; id: bigint }[]
  group_members?: Record<string, { kind: string; id: bigint }[]>
  object_name?: Record<string, string>
  group_name?: Record<string, string>
  node_tags?: Record<string, string[]>
  tag_meta_paths?: string[]
  /** instance id → { def, pose (row-major 3×4), member object ids } */
  instances?: Record<string, { def: bigint; pose: number[]; members: bigint[] }>
  component_name?: Record<string, string>
}): WasmScene {
  const inst = (id: bigint) => o.instances?.[String(id)]
  return {
    object_ids: () => o.object_ids ?? [],
    group_ids: () => o.group_ids ?? [],
    instance_ids: () => o.instance_ids ?? [],
    top_level_nodes: () => o.top_level_nodes ?? [],
    group_members: (id: bigint) => o.group_members?.[String(id)] ?? [],
    object_name: (id: bigint) => o.object_name?.[String(id)],
    group_name: (id: bigint) => o.group_name?.[String(id)],
    instance_name: () => undefined,
    instance_def: (id: bigint) => inst(id)?.def,
    instance_pose: (id: bigint) => (inst(id) === undefined ? undefined : new Float64Array(inst(id)!.pose)),
    instance_expanded_members: (id: bigint) => new BigUint64Array(inst(id)?.members ?? []),
    instance_expanded_local_poses: (id: bigint) => new Float64Array((inst(id)?.members ?? []).flatMap(() => IDENTITY)),
    component_name: (id: bigint) => o.component_name?.[String(id)],
    node_tags: (_k: number, id: bigint) => o.node_tags?.[String(id)] ?? [],
    object_mesh: () => makeMesh(BOX),
    tag_meta_paths: () => o.tag_meta_paths ?? [],
  } as unknown as WasmScene
}

describe('cutListRows', () => {
  it('lists a part ONCE however many tags it carries, folds identical parts, and skips group containers', () => {
    // Four legs (identical boxes) each under three tags, one tabletop
    // untagged, all inside a group "Table" — the parts sheet repeats each
    // leg under every tag; the cut list must not.
    const legs = [1n, 2n, 3n, 4n]
    const scene = makeScene({
      object_ids: [...legs, 5n],
      group_ids: [10n],
      top_level_nodes: [{ kind: 'group', id: 10n }],
      group_members: { '10': [...legs.map((id) => ({ kind: 'object', id })), { kind: 'object', id: 5n }] },
      object_name: { '1': 'Vertical Leg', '2': 'Vertical Leg', '3': 'Vertical Leg', '4': 'Vertical Leg', '5': 'Top' },
      group_name: { '10': 'Table' },
      node_tags: { '1': ['Assembled', 'Cut List', 'Parts/Legs'], '2': ['Assembled', 'Cut List', 'Parts/Legs'], '3': ['Assembled', 'Parts/Legs'], '4': ['Assembled'] },
    })
    const rows = cutListRows(scene, 'mm')
    expect(rows.map((r) => [r.label, r.qty])).toEqual([
      ['Vertical Leg', 4],
      ['Top', 1],
    ])
    // Cells carry their unit; there is no separate "lengths in" line to need.
    expect(rows[0].l).toBe('1000 mm')
    expect(rows[0].h).toBe('3000 mm')
  })

  it('folds every instance of one component definition however it is turned in the world, with the definition\'s own L × W × H', () => {
    // Two placements of definition 7 (a 1 × 2 × 3 m box): one upright, one
    // turned 90° so its world box reads 2 × 1 × 3 — one part, Qty 2, dims
    // from the definition, not from either placement.
    const scene = makeScene({
      object_ids: [100n],
      instance_ids: [21n, 22n],
      top_level_nodes: [{ kind: 'instance', id: 21n }, { kind: 'instance', id: 22n }],
      instances: { '21': { def: 7n, pose: IDENTITY, members: [100n] }, '22': { def: 7n, pose: TURNED, members: [100n] } },
      component_name: { '7': 'Vertical Leg' },
    })
    const rows = cutListRows(scene, 'mm')
    expect(rows).toHaveLength(1)
    expect(rows[0].qty).toBe(2)
    expect([rows[0].l, rows[0].w, rows[0].h]).toEqual(['1000 mm', '2000 mm', '3000 mm'])
  })

  it('an instance rescaled on its own is a different part with its real size', () => {
    // Definition 7 placed twice: one as is, one stretched ×2 along its own x
    // (and turned): two rows, the second 2000 × 2000 × 3000.
    const TURNED_X2 = [0, -1, 0, 0, 2, 0, 0, 0, 0, 0, 1, 0]
    const scene = makeScene({
      object_ids: [100n],
      instance_ids: [21n, 22n],
      top_level_nodes: [{ kind: 'instance', id: 21n }, { kind: 'instance', id: 22n }],
      instances: { '21': { def: 7n, pose: IDENTITY, members: [100n] }, '22': { def: 7n, pose: TURNED_X2, members: [100n] } },
      component_name: { '7': 'Leg' },
    })
    const rows = cutListRows(scene, 'mm')
    expect(rows.map((r) => [r.qty, r.l, r.w, r.h])).toEqual([
      [1, '1000 mm', '2000 mm', '3000 mm'],
      [1, '2000 mm', '2000 mm', '3000 mm'],
    ])
  })
})
