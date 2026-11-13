/**
 * Shop Mode's side of the user's editing context from the bus
 * (`hew.view.enter` / `exit` / `exit_all`, docs/agents/HEW_API.md §6.3).
 *
 * Shop has `activeContext` (a plain list of in-context entries) and no
 * sessions, so an entry here pushes the whole ancestor chain, outermost
 * first, instead of opening frames; `Scene.set_ui_context` reports the
 * entries a fallback can hold (objects and instances) and drops groups.
 * Pure helpers over the entries, so the chain logic is testable without the
 * component; `ShopApp` mirrors the result into the Scene.
 */

import type { Scene } from '../wasm/pkg/wasm_api'
import { isSketchScopedKind, type NodeRef } from '../panels/treeModel'

/**
 * The entries `target` puts on `activeContext`: every container above it
 * (walking `node_parent`) and the target itself, outermost first. A
 * sketch-scoped target has no container chain and enters nothing.
 */
export function shopEnterChain(scene: Pick<Scene, 'node_parent'>, target: NodeRef): NodeRef[] {
  if (isSketchScopedKind(target.kind)) return []
  const chain: NodeRef[] = []
  let cur: NodeRef | undefined = target
  while (cur !== undefined) {
    chain.push(cur)
    const kindNum = cur.kind === 'group' ? 1 : cur.kind === 'instance' ? 2 : 0
    const parent = scene.node_parent(kindNum, cur.id)
    cur = parent !== undefined ? { kind: 'group', id: parent } : undefined
  }
  return chain.reverse()
}

/** The entries left after truncating to combined depth `depth` (the
 * breadcrumb's own rule; Shop has no session frames, so depth counts
 * entries only). */
export function shopSetPathDepth(entries: readonly NodeRef[], depth: number): NodeRef[] {
  return entries.slice(0, Math.max(0, depth))
}
