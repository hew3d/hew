import { describe, expect, it } from 'vitest'
import { selectionMaterials } from './selectionMaterials'
import type { Scene as WasmScene } from '../wasm/loader'

const NONE = 0xffff_ffff_ffff_ffffn

interface Fake {
  objects: Record<string, bigint[]>
  groups: Record<string, { kind: string; id: bigint }[]>
  instances: Record<string, bigint[]>
  /** Per instance, the material each expanded member inherits. */
  inherited: Record<string, bigint[]>
  objectMaterialsCalls: bigint[]
}

function fake(f: Partial<Omit<Fake, 'objectMaterialsCalls'>>): { scene: WasmScene; calls: bigint[] } {
  const calls: bigint[] = []
  const scene = {
    object_materials: (id: bigint) => {
      calls.push(id)
      return f.objects?.[id.toString()] ?? []
    },
    group_members: (id: bigint) => f.groups?.[id.toString()] ?? [],
    instance_expanded_members: (id: bigint) => f.instances?.[id.toString()] ?? [],
    instance_expanded_inherited: (id: bigint) =>
      f.inherited?.[id.toString()] ?? (f.instances?.[id.toString()] ?? []).map(() => NONE),
  } as unknown as WasmScene
  return { scene, calls }
}

describe('selectionMaterials', () => {
  it("lists a painted placement's material before its members'", () => {
    const { scene } = fake({
      objects: { '1': [10n] },
      instances: { '5': [1n, 1n] },
      inherited: { '5': [30n, NONE] },
    })
    expect(selectionMaterials(scene, [{ kind: 'instance', id: 5n }])).toEqual([30n, 10n])
  })

  it('lists each material once, in first-seen order', () => {
    const { scene } = fake({ objects: { '1': [10n, 11n], '2': [11n, 12n] } })
    expect(
      selectionMaterials(scene, [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }]),
    ).toEqual([10n, 11n, 12n])
  })

  it('walks a group recursively, and survives a group that contains itself', () => {
    const { scene } = fake({
      objects: { '1': [10n], '2': [20n] },
      groups: {
        '1': [{ kind: 'object', id: 1n }, { kind: 'group', id: 2n }],
        '2': [{ kind: 'object', id: 2n }, { kind: 'group', id: 1n }],
      },
    })
    expect(selectionMaterials(scene, [{ kind: 'group', id: 1n }])).toEqual([10n, 20n])
  })

  it('reads an instance through its expanded members (nested instances already descended)', () => {
    const { scene } = fake({ objects: { '7': [70n], '8': [80n, 70n] }, instances: { '5': [7n, 8n] } })
    expect(selectionMaterials(scene, [{ kind: 'instance', id: 5n }])).toEqual([70n, 80n])
  })

  it('a member reached from two places contributes once', () => {
    const { scene } = fake({
      objects: { '1': [10n] },
      groups: { '1': [{ kind: 'object', id: 1n }, { kind: 'object', id: 1n }] },
      instances: { '5': [1n] },
    })
    expect(
      selectionMaterials(scene, [{ kind: 'group', id: 1n }, { kind: 'instance', id: 5n }]),
    ).toEqual([10n])
  })

  it('counts hidden members: it describes the object, not what is shown', () => {
    // The walk never consults visibility; a hidden member is just a member.
    const { scene, calls } = fake({
      objects: { '1': [10n], '2': [20n] },
      groups: { '1': [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }] },
    })
    ;(scene as unknown as { node_user_hidden: () => boolean }).node_user_hidden = () => true
    expect(selectionMaterials(scene, [{ kind: 'group', id: 1n }])).toEqual([10n, 20n])
    expect(calls).toEqual([1n, 2n])
  })

  it('a sketch adds nothing', () => {
    const { scene } = fake({})
    expect(selectionMaterials(scene, [{ kind: 'sketch', id: 3n }])).toEqual([])
  })
})
