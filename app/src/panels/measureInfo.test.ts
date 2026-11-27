import { describe, it, expect } from 'vitest'
import { measureSelection } from './measureInfo'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from './treeModel'

interface Leaf {
  volume: number | null
  area: number
  min: [number, number, number]
  max: [number, number, number]
}

/** A mock scene whose `measure_json` mimics the kernel: a node's answer is
 * the sum over its leaves (groups) or the posed leaf (instances). */
function makeScene(opts: {
  leaves: Record<string, Leaf[]>
  groups?: Record<string, NodeRef[]>
  frames?: Record<string, { extents: number[]; source: string }>
  generation?: () => number
  epoch?: () => number
  errors?: Record<string, string>
  calls?: string[]
}): WasmScene {
  return {
    history_generation: () => opts.generation?.() ?? 1,
    document_epoch: () => opts.epoch?.() ?? 0,
    group_members: (id: bigint) => (opts.groups?.[String(id)] ?? []).map((n) => ({ kind: n.kind, id: n.id })),
    measure_json: (kind: number, id: bigint) => {
      const key = `${kind}:${id}`
      opts.calls?.push(key)
      const err = opts.errors?.[key]
      if (err !== undefined) throw new Error(err)
      const ls = opts.leaves[key]
      if (ls === undefined) throw new Error('NotPlaced: no world placement for that node')
      // Mimics the kernel: volume sums the CLOSED leaves only, and is null
      // only when no leaf is closed; `open_leaves` counts the rest.
      let volume: number | null = null
      let openLeaves = 0
      let area = 0
      const min = [Infinity, Infinity, Infinity]
      const max = [-Infinity, -Infinity, -Infinity]
      for (const l of ls) {
        if (l.volume === null) openLeaves++
        else volume = (volume ?? 0) + l.volume
        area += l.area
        for (let i = 0; i < 3; i++) {
          min[i] = Math.min(min[i], l.min[i])
          max[i] = Math.max(max[i], l.max[i])
        }
      }
      return JSON.stringify({ volume, area, bbox: { min, max }, extents: [0, 0, 0], leaf_count: ls.length, open_leaves: openLeaves })
    },
    part_frame_json: (kind: number, id: bigint) => {
      const f = opts.frames?.[`${kind}:${id}`]
      if (f === undefined) throw new Error('NoPartFrame')
      return JSON.stringify(f)
    },
  } as unknown as WasmScene
}

const box = (v: number | null, a: number, max: [number, number, number], min: [number, number, number] = [0, 0, 0]): Leaf => ({
  volume: v,
  area: a,
  min,
  max,
})
const obj = (id: number): NodeRef => ({ kind: 'object', id: BigInt(id) })
const grp = (id: number): NodeRef => ({ kind: 'group', id: BigInt(id) })
const inst = (id: number): NodeRef => ({ kind: 'instance', id: BigInt(id) })

describe('measureSelection', () => {
  it('a_box_reports_its_volume_and_area', () => {
    const scene = makeScene({ leaves: { '0:1': [box(0.1, 1.6, [1, 0.5, 0.2])] } })
    const m = measureSelection(scene, [obj(1)])!
    expect(m.volume).toBeCloseTo(0.1)
    expect(m.area).toBeCloseTo(1.6)
    expect(m.size).toEqual([1, 0.5, 0.2])
    expect(m.sizeSource).toBeNull()
  })

  it('a_group_sums_its_leaves', () => {
    const scene = makeScene({
      leaves: { '1:5': [box(1, 6, [1, 1, 1]), box(2, 10, [3, 1, 1], [1, 0, 0])] },
    })
    const m = measureSelection(scene, [grp(5)])!
    expect(m.volume).toBe(3)
    expect(m.area).toBe(16)
    expect(m.size).toEqual([3, 1, 1])
  })

  it('a_scaled_instance_uses_the_pose_determinant', () => {
    // The kernel scales volume by |det(pose)|; the panel passes it through.
    const scene = makeScene({ leaves: { '2:9': [box(8, 24, [2, 2, 2])] } })
    expect(measureSelection(scene, [inst(9)])!.volume).toBe(8)
  })

  it('an_open_leaf_makes_volume_null_and_area_still_sums', () => {
    const scene = makeScene({ leaves: { '0:1': [box(1, 6, [1, 1, 1])], '0:2': [box(null, 2, [1, 1, 0], [0, 0, 0])] } })
    const m = measureSelection(scene, [obj(1), obj(2)])!
    expect(m.volume).toBeNull()
    expect(m.area).toBe(8)
  })

  it('a_mixed_closed_and_open_group_has_null_volume', () => {
    const scene = makeScene({ leaves: { '1:5': [box(1, 6, [1, 1, 1]), box(null, 2, [2, 1, 0], [1, 0, 0])] } })
    const m = measureSelection(scene, [grp(5)])!
    expect(m.volume).toBeNull()
    expect(m.area).toBe(8)
  })

  it('a_closed_plus_mixed_multi_selection_has_null_volume_and_summed_area', () => {
    const scene = makeScene({
      leaves: {
        '0:1': [box(1, 6, [1, 1, 1])],
        '1:5': [box(2, 10, [3, 1, 1], [1, 0, 0]), box(null, 2, [4, 1, 0], [3, 0, 0])],
      },
    })
    const m = measureSelection(scene, [obj(1), grp(5)])!
    expect(m.volume).toBeNull()
    expect(m.area).toBe(18)
  })

  it('a_refusal_makes_the_whole_selection_null_but_not_placed_contributes_nothing', () => {
    const scene = makeScene({
      leaves: { '0:1': [box(1, 6, [1, 1, 1])], '0:2': [box(1, 6, [1, 1, 1])] },
      errors: { '0:2': 'TooManyOccurrences: too many' },
    })
    expect(measureSelection(scene, [obj(1), obj(2)])).toBeNull()
    expect(measureSelection(scene, [obj(1), obj(77)])!.area).toBe(6)
    const none = makeScene({ leaves: { '0:1': [box(1, 6, [1, 1, 1])] }, errors: { '0:3': 'NoGeometry: nothing' } })
    expect(measureSelection(none, [obj(1), obj(3)])!.area).toBe(6)
  })

  it('nested_selected_roots_count_once', () => {
    const calls: string[] = []
    const scene = makeScene({
      leaves: { '1:5': [box(1, 6, [1, 1, 1]), box(2, 10, [3, 1, 1], [1, 0, 0])], '0:1': [box(1, 6, [1, 1, 1])] },
      groups: { '5': [obj(1)] },
      calls,
    })
    const m = measureSelection(scene, [grp(5), obj(1)])!
    expect(m.volume).toBe(3)
    expect(m.area).toBe(16)
    expect(calls).toEqual(['1:5'])
  })

  it('hidden_members_are_included', () => {
    // measure_json counts hidden members; the sum is not filtered here.
    const scene = makeScene({ leaves: { '1:5': [box(1, 6, [1, 1, 1]), box(1, 6, [2, 1, 1], [1, 0, 0])] } })
    expect(measureSelection(scene, [grp(5)])!.volume).toBe(2)
  })

  it('size_uses_the_part_frame_for_a_single_node_and_the_world_box_for_several', () => {
    const scene = makeScene({
      leaves: { '0:1': [box(1, 6, [1, 2, 3])], '0:2': [box(1, 6, [4, 1, 1], [3, 0, 0])] },
      frames: { '0:1': { extents: [3, 2, 1], source: 'attribute' } },
    })
    const one = measureSelection(scene, [obj(1)])!
    expect(one.size).toEqual([3, 2, 1])
    expect(one.sizeSource).toBe('attribute')
    const two = measureSelection(scene, [obj(1), obj(2)])!
    expect(two.size).toEqual([4, 2, 3])
    expect(two.sizeSource).toBeNull()
  })

  it('returns_null_without_geometry_and_memoizes_by_generation', () => {
    const calls: string[] = []
    let gen = 1
    const scene = makeScene({ leaves: { '0:1': [box(1, 6, [1, 1, 1])] }, generation: () => gen, calls })
    expect(measureSelection(scene, [obj(99)])).toBeNull()
    calls.length = 0
    measureSelection(scene, [obj(1)])
    measureSelection(scene, [obj(1)])
    expect(calls).toEqual(['0:1'])
    gen = 2
    measureSelection(scene, [obj(1)])
    expect(calls).toEqual(['0:1', '0:1'])
  })

  it('recomputes_when_the_document_epoch_changes_at_the_same_generation', () => {
    const calls: string[] = []
    let epoch = 0
    const scene = makeScene({ leaves: { '0:1': [box(1, 6, [1, 1, 1])] }, epoch: () => epoch, calls })
    measureSelection(scene, [obj(1)])
    measureSelection(scene, [obj(1)])
    expect(calls).toEqual(['0:1'])
    epoch = 1
    measureSelection(scene, [obj(1)])
    expect(calls).toEqual(['0:1', '0:1'])
  })
})
