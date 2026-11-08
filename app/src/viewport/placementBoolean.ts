/**
 * Booleans on a placement (a copy of a linked set, or a component).
 *
 * Subtract cuts through the shared shape, so every copy shows the cut; the
 * scope strip's "Only this one" unlinks that copy first. Union and Intersect
 * cannot propagate (the result would belong to two shapes or to none), so
 * the app unlinks the placement and joins plain solids, and says so.
 */

import { t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { inHistoryBracket } from '../tools/transformSelection'
import { unlinkedLabel } from '../linkedCopyCopy'

export const OP_UNION = 0
export const OP_SUBTRACT = 1
export const OP_INTERSECT = 2

/** How a boolean on these operands runs. */
export type BooleanRoute =
  /** Both plain: the kernel's world boolean. */
  | 'plain'
  /** Subtract a loose solid from a placement: through the shared shape. */
  | 'through-placement'
  /** A placement cut by a group or another placement: one solid at a time. */
  | 'refuse-cutter'
  /** A placement joins (or is the cutter): unlink it, then the world boolean. */
  | 'unlink-and-join'

export function routeBoolean(
  op: number,
  a: NodeRef,
  b: NodeRef,
  isLinked: (instance: bigint) => boolean,
): BooleanRoute {
  // Only a copy of a LINKED set takes these routes. An ordinary component
  // keeps the boolean it always had (the kernel's refusal, then the
  // explode-and-retry), whatever its instance count.
  const linkedHost = op === OP_SUBTRACT && a.kind === 'instance' && isLinked(a.id)
  if (linkedHost) return b.kind === 'object' ? 'through-placement' : 'refuse-cutter'
  const linked = (n: NodeRef): boolean => n.kind === 'instance' && isLinked(n.id)
  if (linked(a) || linked(b)) return 'unlink-and-join'
  return 'plain'
}

/** The label of an operation, for the undo step. */
export function opLabel(op: number): string {
  if (op === OP_UNION) return t`Union`
  if (op === OP_INTERSECT) return t`Intersect`
  return t`Subtract`
}

/**
 * Subtract the loose solid `tool` from the placement `host`. With `scope`
 * `'one'` the host is unlinked first, in the same undo step, so only it is
 * cut. Throws the kernel's typed error on a refusal, leaving the document
 * untouched.
 */
export function subtractThroughPlacement(
  scene: WasmScene,
  host: bigint,
  tool: bigint,
  scope: 'all' | 'one',
): void {
  const path = BigUint64Array.of(host)
  const cut = (): void => {
    // `host` undefined: the definition's only part (else AmbiguousHost).
    scene.boolean_through_placement_json(path, undefined, tool, OP_SUBTRACT)
  }
  if (scope === 'all') {
    cut()
    return
  }
  inHistoryBracket(scene, unlinkedLabel(opLabel(OP_SUBTRACT)), () => {
    scene.make_unique_by_path_json(path, true, false)
    cut()
  })
}
