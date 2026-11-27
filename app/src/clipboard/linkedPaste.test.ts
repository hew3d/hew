/**
 * Linked paste keeps the copied thing's color. A wrap moves a painted
 * Object's base onto its placement, so the pasted placement must take the
 * source placement's own material (a plain paste carries it the same way).
 * The scene is a mock: the real round trip is in linkedCopies.wasm.test.ts.
 */
import { describe, expect, it } from 'vitest'
import type { Scene as WasmScene } from '../wasm/loader'
import { pasteLinked } from './linkedPaste'

const NONE = 0xffff_ffff_ffff_ffffn
const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
const shift = (dx: number): Float64Array => Float64Array.from([1, 0, 0, dx, 0, 1, 0, 0, 0, 0, 1, 0])

function mockScene(
  objectBase: Map<bigint, bigint>,
  instanceMaterial: Map<bigint, bigint>,
  /** Placements whose own material the shared solid's base covers. */
  coveredByBase: Set<bigint> = new Set(),
) {
  const calls: string[] = []
  let nextInstance = 500n
  const scene = {
    extract_item_placement: () => Float64Array.from(IDENTITY),
    history_bracket_open: () => calls.push('open'),
    history_bracket_commit: () => calls.push('commit'),
    history_bracket_abort: () => calls.push('abort'),
    wrap_linked: (_kind: number, id: bigint) => {
      // The kernel's rule: an Object's base moves onto its placement.
      const wrapped = nextInstance++
      const base = objectBase.get(id)
      if (base !== undefined) {
        instanceMaterial.set(wrapped, base)
        objectBase.delete(id)
      }
      calls.push(`wrap:${id}`)
      return wrapped
    },
    instance_def: () => 77n,
    instance_pose: () => Float64Array.from(IDENTITY),
    place_instance: () => {
      const id = nextInstance++
      calls.push(`place:${id}`)
      return id
    },
    instance_material: (id: bigint) => instanceMaterial.get(id) ?? NONE,
    // The kernel's rule: a base on the shared solid shows on every copy.
    instance_appearance_material: (id: bigint) => (coveredByBase.has(id) ? NONE : (instanceMaterial.get(id) ?? NONE)),
    set_instance_material: (id: bigint, m: bigint) => {
      if (m === NONE) instanceMaterial.delete(id)
      else instanceMaterial.set(id, m)
      calls.push(`paint:${id}:${m}`)
      return BigUint64Array.from([id])
    },
  } as unknown as WasmScene
  return { scene, calls }
}

describe('pasteLinked keeps the color', () => {
  it('a_linked_paste_of_a_painted_object_is_painted_like_it', () => {
    const red = 7n
    const objectBase = new Map([[1n, red]])
    const instanceMaterial = new Map<bigint, bigint>()
    const { scene, calls } = mockScene(objectBase, instanceMaterial)
    const { result, sources } = pasteLinked(scene, [{ kind: 'object', id: 1n }], null, shift(5))
    const pasted = result.rootIds[0]
    expect(instanceMaterial.get(sources[0].id)).toBe(red)
    expect(instanceMaterial.get(pasted)).toBe(red)
    expect(calls[0]).toBe('open')
    expect(calls.at(-1)).toBe('commit')
  })

  it('a_linked_paste_of_a_painted_placement_is_painted_like_it', () => {
    const blue = 9n
    const instanceMaterial = new Map([[40n, blue]])
    const { scene } = mockScene(new Map(), instanceMaterial)
    const { result } = pasteLinked(scene, [{ kind: 'instance', id: 40n }], null, shift(5))
    expect(instanceMaterial.get(result.rootIds[0])).toBe(blue)
  })

  it('an_unpainted_source_paints_nothing', () => {
    const instanceMaterial = new Map<bigint, bigint>()
    const { scene, calls } = mockScene(new Map(), instanceMaterial)
    pasteLinked(scene, [{ kind: 'object', id: 1n }, { kind: 'instance', id: 40n }], null, shift(5))
    expect(calls.some((c) => c.startsWith('paint:'))).toBe(false)
    expect(instanceMaterial.size).toBe(0)
  })

  it('a_copy_whose_paint_hides_under_the_shared_base_pastes_unpainted', () => {
    // The source carries green but shows the set's base: the pasted copy
    // shows the base too, and painting it would move the base off the set.
    const green = 11n
    const instanceMaterial = new Map([[40n, green]])
    const { scene, calls } = mockScene(new Map(), instanceMaterial, new Set([40n]))
    const { result } = pasteLinked(scene, [{ kind: 'instance', id: 40n }], null, shift(5))
    expect(calls.some((c) => c.startsWith('paint:'))).toBe(false)
    expect(instanceMaterial.has(result.rootIds[0])).toBe(false)
    expect(instanceMaterial.get(40n)).toBe(green)
  })
})
