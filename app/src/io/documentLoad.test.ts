import { describe, it, expect, vi } from 'vitest'
import {
  loadHewBytes,
  isPanicError,
  isSceneEmpty,
  seedHiddenKeysFromRegistry,
  seedHiddenTagPathsFromRegistry,
  unionHiddenLeafIds,
} from './documentLoad'
import type { Scene as WasmScene } from '../wasm/loader'
import { nodeKey, type NodeRef } from '../panels/treeModel'
import { tagPathKey } from '../panels/tagModel'

describe('isPanicError', () => {
  it('recognizes the borrow-lock signatures', () => {
    expect(isPanicError('recursive use of an object detected which would lead to unsafe aliasing')).toBe(true)
    expect(isPanicError('internal error: entered unreachable code')).toBe(true)
    expect(isPanicError('RECURSIVE USE OF AN OBJECT')).toBe(true) // case-insensitive
  })

  it('leaves an ordinary refusal message alone', () => {
    expect(isPanicError('UnknownObject: stale handle')).toBe(false)
    expect(isPanicError('')).toBe(false)
  })
})

/** A fake Scene exposing only what `loadHewBytes`/the seed helpers read. */
function makeScene(overrides: Partial<{
  load: (bytes: Uint8Array) => void
  object_ids: bigint[]
  group_ids: bigint[]
  instance_ids: bigint[]
  sketch_ids: bigint[]
  tag_meta_paths: string[]
  tag_meta_hidden: number[]
  user_hidden_kinds: number[]
  user_hidden_ids: bigint[]
  /** Group membership, keyed by group id (string) — `unionHiddenLeafIds`'s
   *  `getGroupMembers` seam. */
  group_members: Record<string, NodeRef[]>
  /** Tag paths per node, keyed by `${kind}:${id}` — `node_tags`'s seam,
   *  already `/`-joined the way the real wasm accessor returns it. */
  node_tags: Record<string, string[]>
}> = {}): WasmScene {
  return {
    load: overrides.load ?? (() => {}),
    object_ids: () => overrides.object_ids ?? [],
    group_ids: () => overrides.group_ids ?? [],
    instance_ids: () => overrides.instance_ids ?? [],
    sketch_ids: () => overrides.sketch_ids ?? [],
    tag_meta_paths: () => overrides.tag_meta_paths ?? [],
    tag_meta_hidden: () => overrides.tag_meta_hidden ?? [],
    user_hidden_kinds: () => overrides.user_hidden_kinds ?? [],
    user_hidden_ids: () => overrides.user_hidden_ids ?? [],
    group_members: (groupId: bigint) => overrides.group_members?.[String(groupId)] ?? [],
    node_tags: (kindNum: number, id: bigint) => {
      const kind = ['object', 'group', 'instance'][kindNum]
      return overrides.node_tags?.[`${kind}:${id}`] ?? []
    },
  } as unknown as WasmScene
}

describe('loadHewBytes', () => {
  it('returns ok on a successful parse', () => {
    const load = vi.fn()
    const scene = makeScene({ load })
    const bytes = new Uint8Array([1, 2, 3])
    const result = loadHewBytes(scene, bytes)
    expect(result).toEqual({ ok: true })
    expect(load).toHaveBeenCalledWith(bytes)
  })

  it('reports a non-panic error without setting panicked', () => {
    const err = new Error('UnknownFormat: bad magic bytes')
    const scene = makeScene({
      load: () => {
        throw err
      },
    })
    const result = loadHewBytes(scene, new Uint8Array())
    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toBe(err)
      expect(result.panicked).toBe(false)
    }
  })

  it('flags a panic-shaped error as panicked', () => {
    const scene = makeScene({
      load: () => {
        throw new Error('recursive use of an object')
      },
    })
    const result = loadHewBytes(scene, new Uint8Array())
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.panicked).toBe(true)
  })

  it('handles a thrown non-Error value', () => {
    const scene = makeScene({
      load: () => {
        // eslint-disable-next-line @typescript-eslint/no-throw-literal
        throw 'unreachable'
      },
    })
    const result = loadHewBytes(scene, new Uint8Array())
    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toBe('unreachable')
      expect(result.panicked).toBe(true)
    }
  })
})

describe('isSceneEmpty', () => {
  it('is true for a document with no entities of any kind', () => {
    expect(isSceneEmpty(makeScene())).toBe(true)
  })

  it('is false if any single registry is non-empty', () => {
    expect(isSceneEmpty(makeScene({ object_ids: [1n] }))).toBe(false)
    expect(isSceneEmpty(makeScene({ group_ids: [1n] }))).toBe(false)
    expect(isSceneEmpty(makeScene({ instance_ids: [1n] }))).toBe(false)
    expect(isSceneEmpty(makeScene({ sketch_ids: [1n] }))).toBe(false)
  })
})

describe('seedHiddenTagPathsFromRegistry', () => {
  it('collects only the paths flagged hidden, split and trimmed', () => {
    const scene = makeScene({
      tag_meta_paths: ['Roof', ' Walls / Interior ', 'Floor'],
      tag_meta_hidden: [1, 0, 1],
    })
    const seeded = seedHiddenTagPathsFromRegistry(scene)
    expect(seeded.size).toBe(2)
    expect(seeded.has(tagPathKey(['Roof']))).toBe(true)
    expect(seeded.has(tagPathKey(['Floor']))).toBe(true)
  })

  it('is empty when nothing is registered hidden', () => {
    const scene = makeScene({ tag_meta_paths: ['Roof'], tag_meta_hidden: [0] })
    expect(seedHiddenTagPathsFromRegistry(scene).size).toBe(0)
  })
})

describe('seedHiddenKeysFromRegistry', () => {
  it('maps kind indices (0=object, 1=group, 2=instance) to nodeKey strings', () => {
    const scene = makeScene({
      user_hidden_kinds: [0, 1, 2],
      user_hidden_ids: [10n, 20n, 30n],
    })
    const seeded = seedHiddenKeysFromRegistry(scene)
    expect(seeded).toEqual(new Set(['object:10', 'group:20', 'instance:30']))
  })

  it('skips an unrecognized kind index rather than throwing', () => {
    const scene = makeScene({ user_hidden_kinds: [9], user_hidden_ids: [1n] })
    expect(seedHiddenKeysFromRegistry(scene).size).toBe(0)
  })
})

describe('unionHiddenLeafIds', () => {
  it('returns empty sets for two empty inputs', () => {
    const scene = makeScene()
    expect(unionHiddenLeafIds(scene, new Set(), new Set())).toEqual({ objectIds: [], instanceIds: [] })
  })

  it('resolves a manually-hidden plain object/instance directly', () => {
    const scene = makeScene()
    const hiddenKeys = new Set([nodeKey({ kind: 'object', id: 1n }), nodeKey({ kind: 'instance', id: 2n })])
    const result = unionHiddenLeafIds(scene, hiddenKeys, new Set())
    expect(result.objectIds).toEqual([1n])
    expect(result.instanceIds).toEqual([2n])
  })

  it('expands a manually-hidden group to every leaf object/instance, recursively', () => {
    const scene = makeScene({
      group_members: {
        '100': [{ kind: 'object', id: 1n }, { kind: 'group', id: 101n }],
        '101': [{ kind: 'object', id: 2n }, { kind: 'instance', id: 10n }],
      },
    })
    const hiddenKeys = new Set([nodeKey({ kind: 'group', id: 100n })])
    const result = unionHiddenLeafIds(scene, hiddenKeys, new Set())
    expect(result.objectIds.sort()).toEqual([1n, 2n])
    expect(result.instanceIds).toEqual([10n])
  })

  it('resolves a hidden tag path to every node tagged at or under it', () => {
    const scene = makeScene({
      object_ids: [1n, 2n],
      node_tags: {
        'object:1': ['Structure/Roof'],
        'object:2': ['Structure'],
      },
    })
    // Hiding "Structure" covers both the exact match (object 2) and the
    // descendant "Structure/Roof" (object 1) — isPathUnder's contract.
    const result = unionHiddenLeafIds(scene, new Set(), new Set([tagPathKey(['Structure'])]))
    expect(result.objectIds.sort()).toEqual([1n, 2n])
  })

  it('does NOT cover a sibling tag path, only an exact match or descendant', () => {
    const scene = makeScene({
      object_ids: [1n, 2n],
      node_tags: {
        'object:1': ['Structure/Roof'],
        'object:2': ['Walls'],
      },
    })
    const result = unionHiddenLeafIds(scene, new Set(), new Set([tagPathKey(['Structure'])]))
    expect(result.objectIds).toEqual([1n])
  })

  it('unions manual hides and tag hides, deduplicating a leaf covered by both', () => {
    const scene = makeScene({
      object_ids: [1n, 2n],
      node_tags: { 'object:1': ['Roof'] },
    })
    const hiddenKeys = new Set([nodeKey({ kind: 'object', id: 1n }), nodeKey({ kind: 'object', id: 2n })])
    const result = unionHiddenLeafIds(scene, hiddenKeys, new Set([tagPathKey(['Roof'])]))
    expect(result.objectIds.sort()).toEqual([1n, 2n])
  })

  it('ignores a tag-path set when no node carries any tag', () => {
    const scene = makeScene({ object_ids: [1n] })
    const result = unionHiddenLeafIds(scene, new Set(), new Set([tagPathKey(['Nonexistent'])]))
    expect(result).toEqual({ objectIds: [], instanceIds: [] })
  })
})
