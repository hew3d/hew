/**
 * Tests for `buildAncestorChain` / `resolvePickToSelectable` — the
 * outermost-first pick-selection logic in Viewport.tsx.
 *
 * These are exported solely for this suite (Viewport itself only calls them
 * internally). Only pure functions are exercised here — no rendering, no
 * three.js/WebGL — so a real `Viewport` mount is unnecessary; the mock Scene
 * below follows the same plain-object pattern as `panels/scenePanels.test.tsx`.
 */

import { describe, expect, it } from 'vitest'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { buildAncestorChain, resolvePickToSelectable } from './Viewport'

/**
 * Build a mock WasmScene backed by an explicit parent map, keyed by
 * `${kind}:${id}` → parent group id (kind 1). `node_parent(2, id)` (instance →
 * group) and `node_parent(1, id)` (group → group) both look up this map;
 * `node_parent(0, id)` (object → group) does too, so one map covers every
 * ancestor-walk kind used by `buildAncestorChain`.
 */
function makeScene(parents: Record<string, bigint>): WasmScene {
  return {
    node_parent: (kind: number, id: bigint) => parents[`${kind}:${id}`],
  } as unknown as WasmScene
}

describe('buildAncestorChain', () => {
  it('rooted at a plain object with no group ancestors returns just the object', () => {
    const scene = makeScene({})
    expect(buildAncestorChain(scene, 1n)).toEqual([{ kind: 'object', id: 1n }])
  })

  it('rooted at an object walks group ancestors innermost to outermost', () => {
    // object 1 -> group 10 -> group 20 (top)
    const scene = makeScene({ '0:1': 10n, '1:10': 20n })
    expect(buildAncestorChain(scene, 1n)).toEqual([
      { kind: 'object', id: 1n },
      { kind: 'group', id: 10n },
      { kind: 'group', id: 20n },
    ])
  })

  it('rooted at an instance (instanceId given) starts the chain at kind 2, not the object', () => {
    // instance 5 has no group parent — top-level instance.
    const scene = makeScene({})
    expect(buildAncestorChain(scene, 1n, 5n)).toEqual([{ kind: 'instance', id: 5n }])
  })

  it('a nested instance walks group ancestors via node_parent(2, ...) then node_parent(1, ...)', () => {
    // instance 5 -> group 30 (top)
    const scene = makeScene({ '2:5': 30n })
    expect(buildAncestorChain(scene, 1n, 5n)).toEqual([
      { kind: 'instance', id: 5n },
      { kind: 'group', id: 30n },
    ])
  })
})

describe('resolvePickToSelectable — top level', () => {
  it('picking a top-level instance selects that instance (chain length 1)', () => {
    const scene = makeScene({})
    const result = resolvePickToSelectable(scene, 1n, [], 5n)
    expect(result).toEqual({ kind: 'instance', id: 5n })
  })

  it('picking a nested instance selects the outermost wrapper group', () => {
    // instance 5 -> group 30 -> group 40 (top)
    const scene = makeScene({ '2:5': 30n, '1:30': 40n })
    const result = resolvePickToSelectable(scene, 1n, [], 5n)
    expect(result).toEqual({ kind: 'group', id: 40n })
  })

  it('picking a plain nested object still resolves to its outermost group (unchanged behavior)', () => {
    const scene = makeScene({ '0:1': 10n, '1:10': 20n })
    const result = resolvePickToSelectable(scene, 1n, [])
    expect(result).toEqual({ kind: 'group', id: 20n })
  })

  it('picking a plain top-level object with no group ancestors selects the object itself', () => {
    const scene = makeScene({})
    const result = resolvePickToSelectable(scene, 1n, [])
    expect(result).toEqual({ kind: 'object', id: 1n })
  })
})

describe('resolvePickToSelectable — inside an active group context', () => {
  it('picking a direct-child group of the active group resolves to that group', () => {
    // group 10 (child) -> group 20 (active context)
    const scene = makeScene({ '0:1': 10n, '1:10': 20n })
    const ctx: NodeRef[] = [{ kind: 'group', id: 20n }]
    const result = resolvePickToSelectable(scene, 1n, ctx)
    expect(result).toEqual({ kind: 'group', id: 10n })
  })

  it('picking a direct-child instance of the active group resolves to that instance', () => {
    // instance 5 -> group 20 (active context)
    const scene = makeScene({ '2:5': 20n })
    const ctx: NodeRef[] = [{ kind: 'group', id: 20n }]
    const result = resolvePickToSelectable(scene, 1n, ctx, 5n)
    expect(result).toEqual({ kind: 'instance', id: 5n })
  })

  it('picking a direct-child plain object of the active group resolves to that object', () => {
    // object 1 -> group 20 (active context), no further ancestors
    const scene = makeScene({ '0:1': 20n })
    const ctx: NodeRef[] = [{ kind: 'group', id: 20n }]
    const result = resolvePickToSelectable(scene, 1n, ctx)
    expect(result).toEqual({ kind: 'object', id: 1n })
  })

  it('a pick whose chain never reaches the active group is out of scope (null)', () => {
    const scene = makeScene({ '0:1': 10n }) // object 1 -> group 10 (unrelated to 20)
    const ctx: NodeRef[] = [{ kind: 'group', id: 20n }]
    const result = resolvePickToSelectable(scene, 1n, ctx)
    expect(result).toBeNull()
  })

  it('a pick nested two levels inside the active group resolves to the direct child, not the leaf', () => {
    // object 1 -> group 10 -> group 20 (active context)
    const scene = makeScene({ '0:1': 10n, '1:10': 20n })
    const ctx: NodeRef[] = [{ kind: 'group', id: 20n }]
    const result = resolvePickToSelectable(scene, 1n, ctx)
    expect(result).toEqual({ kind: 'group', id: 10n })
  })
})

describe('resolvePickToSelectable — inside an active instance context', () => {
  it('a pick inside the entered instance resolves to the picked definition-member object', () => {
    const scene = makeScene({})
    const ctx: NodeRef[] = [{ kind: 'instance', id: 5n }]
    const result = resolvePickToSelectable(scene, 1n, ctx, 5n)
    expect(result).toEqual({ kind: 'object', id: 1n })
  })

  it('a pick outside the entered instance is out of scope (null)', () => {
    const scene = makeScene({})
    const ctx: NodeRef[] = [{ kind: 'instance', id: 5n }]
    const result = resolvePickToSelectable(scene, 1n, ctx, 6n)
    expect(result).toBeNull()
  })
})

describe('resolvePickToSelectable — inside an active object context', () => {
  it('any pick is out of scope (null) — an object has no children to drill into', () => {
    const scene = makeScene({})
    const ctx: NodeRef[] = [{ kind: 'object', id: 1n }]
    const result = resolvePickToSelectable(scene, 2n, ctx)
    expect(result).toBeNull()
  })
})
