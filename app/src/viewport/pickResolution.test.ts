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
import { nodeKey, type NodeRef } from '../panels/treeModel'
import { buildAncestorChain, flattenSessionScope, resolvePickToSelectable } from './Viewport'

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

// ---------------------------------------------------------------------------
// resolvePickToSelectable — session scope (docs/design/group-session.md):
// the 5th param generalized from an object-id Set (component-session-only,
// pre-groups) to a nodeKey Set covering any node kind (a group session's
// direct members can be objects, nested groups, or instances).
// ---------------------------------------------------------------------------

describe('resolvePickToSelectable — session scope', () => {
  it('a top-level pick whose resolved node key is in scope passes through unchanged', () => {
    const scene = makeScene({})
    const scope = new Set([nodeKey({ kind: 'object', id: 1n })])
    expect(resolvePickToSelectable(scene, 1n, [], undefined, scope)).toEqual({ kind: 'object', id: 1n })
  })

  it('a top-level pick resolving to a GROUP (a group session member) passes through when its key is in scope', () => {
    // Generalizes the old object-only session scope: a group session's
    // direct member can itself be a group (drilling further opens a nested
    // session), not just a plain object like every component session's
    // members always were.
    const scope = new Set([nodeKey({ kind: 'group', id: 20n })])
    // object 1 wrapped by group 20 with no further ancestors — the pick
    // resolves (unscoped) to the outermost group, id 20.
    const wrapped = makeScene({ '0:1': 20n })
    expect(resolvePickToSelectable(wrapped, 1n, [], undefined, scope)).toEqual({ kind: 'group', id: 20n })
  })

  it('a top-level pick resolving to a node key NOT in scope is rejected (a dimmed sibling outside the session)', () => {
    const scene = makeScene({})
    const scope = new Set([nodeKey({ kind: 'object', id: 2n })]) // some OTHER object
    expect(resolvePickToSelectable(scene, 1n, [], undefined, scope)).toBeNull()
  })

  it('null/undefined scope disables filtering entirely — pre-session behavior preserved', () => {
    const scene = makeScene({})
    expect(resolvePickToSelectable(scene, 1n, [], undefined, null)).toEqual({ kind: 'object', id: 1n })
    expect(resolvePickToSelectable(scene, 1n, [], undefined, undefined)).toEqual({ kind: 'object', id: 1n })
  })
})

// ---------------------------------------------------------------------------
// flattenSessionScope (docs/design/group-session.md, adversarial-review
// finding 2): flattens a session's direct-member list — now sourced from
// the kernel's own live `Scene.session_members()`, not an app-side
// open-time `top_level_nodes()` baseline diff (the old
// `computeGroupSessionScope`, which went stale across an undo/redo re-entry
// into an earlier bracket of the same group's session) — into the leaf
// object/instance ids the renderer's isolation fade and marquee/Select-All
// need.
// ---------------------------------------------------------------------------

function makeGroupMembersScene(groupMembers: Record<string, NodeRef[]> = {}): WasmScene {
  return {
    group_members: (id: bigint) => groupMembers[`${id}`] ?? [],
  } as unknown as WasmScene
}

describe('flattenSessionScope', () => {
  it('a direct member OBJECT contributes its own id to objectIds', () => {
    const scene = makeGroupMembersScene()
    const { objectIds, instanceIds } = flattenSessionScope(scene, [
      { kind: 'object', id: 2n },
      { kind: 'object', id: 3n },
    ])
    expect(objectIds).toEqual(new Set([2n, 3n]))
    expect(instanceIds).toEqual(new Set())
  })

  it('flattens a nested member GROUP into its leaf object/instance ids via group_members', () => {
    const scene = makeGroupMembersScene({
      '30': [{ kind: 'object', id: 6n }, { kind: 'instance', id: 7n }],
    })
    const { objectIds, instanceIds } = flattenSessionScope(scene, [{ kind: 'group', id: 30n }])
    expect(objectIds).toEqual(new Set([6n]))
    expect(instanceIds).toEqual(new Set([7n]))
  })

  it('a direct member INSTANCE contributes its own id to instanceIds, not objectIds', () => {
    const scene = makeGroupMembersScene()
    const { objectIds, instanceIds } = flattenSessionScope(scene, [{ kind: 'instance', id: 8n }])
    expect(objectIds).toEqual(new Set())
    expect(instanceIds).toEqual(new Set([8n]))
  })

  it('no direct members yields empty sets, not a throw', () => {
    const scene = makeGroupMembersScene()
    const { objectIds, instanceIds } = flattenSessionScope(scene, [])
    expect(objectIds).toEqual(new Set())
    expect(instanceIds).toEqual(new Set())
  })
})
