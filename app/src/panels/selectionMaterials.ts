/**
 * selectionMaterials — every distinct material on a selection, for Object
 * Info's Materials row.
 *
 * Per leaf Object the kernel's `object_materials` answers its base material
 * plus every per-face override (front and back). The walk makes that a union
 * over a selection:
 *   - an Object contributes its own;
 *   - a Group contributes the union over its members, recursively;
 *   - a component instance contributes the materials its placements pass
 *     down (its own, and a nested placement's own — what unpainted member
 *     faces show, `instance_expanded_inherited`), then the union over its
 *     definition's expanded leaf Objects (nested member instances and
 *     member groups already descended by `instance_expanded_members`).
 * First-seen order, each material once. A Sketch or other kind adds nothing.
 *
 * Hidden contents count: Object Info describes the object, not what is
 * currently shown, so a hidden member's material is listed too. A group that
 * contains itself (or is reached twice) is walked once.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import { nodeRefFromJs, type NodeRef } from './treeModel'

/** `u64::MAX`, the kernel's "no material" sentinel. */
const NO_MATERIAL = 0xffff_ffff_ffff_ffffn

export function selectionMaterials(scene: WasmScene, nodes: readonly NodeRef[]): bigint[] {
  const seen = new Set<bigint>()
  const out: bigint[] = []
  const visitedGroups = new Set<bigint>()
  const add = (m: bigint) => {
    if (!seen.has(m)) {
      seen.add(m)
      out.push(m)
    }
  }
  const addObject = (id: bigint) => {
    for (const m of scene.object_materials(id)) add(m)
  }
  const walk = (node: NodeRef) => {
    if (node.kind === 'object') {
      addObject(node.id)
    } else if (node.kind === 'group') {
      if (visitedGroups.has(node.id)) return
      visitedGroups.add(node.id)
      for (const child of scene.group_members(node.id)) walk(nodeRefFromJs(child))
    } else if (node.kind === 'instance') {
      for (const m of scene.instance_expanded_inherited(node.id)) {
        if (m !== NO_MATERIAL) add(m)
      }
      for (const o of scene.instance_expanded_members(node.id)) addObject(o)
    }
  }
  for (const n of nodes) walk(n)
  return out
}
