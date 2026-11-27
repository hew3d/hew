/**
 * Lock — the scene calls behind Lock, Unlock and the toast's buttons.
 *
 * Each is one undo step (the kernel records "Lock" / "Unlock" itself, and a
 * call that changes nothing records nothing), run through the same history
 * helper the other node edits use. The kernel refuses `ExplodeSessionScope`
 * while a component is open and refuses a lock that would freeze what is
 * being edited; those refusals reach the caller as thrown kernel errors.
 */

import { t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { structuralSelection } from '../panels/treeModel'
import { inHistoryBracket } from '../tools/transformSelection'
import { allLockableNodes, isLockableKind, isLocked, type LockScene } from './lockModel'

/** Lock or unlock `nodes` (a placement stands for its shape: every copy
 *  follows). Nodes of a kind that cannot be locked are skipped. */
export function setNodesLocked(scene: WasmScene, nodes: readonly NodeRef[], locked: boolean): void {
  const sel = structuralSelection(nodes.filter((n) => isLockableKind(n.kind)))
  if (sel === null || sel.ids.length === 0) return
  // A single kernel call is one atomic undo entry already; the helper keeps
  // the call inside the app's one bracket convention.
  inHistoryBracket(scene, locked ? t`Lock` : t`Unlock`, () => scene.set_nodes_locked(sel.kinds, sel.ids, locked), false)
}

/** The locked nodes among `nodes`, one per shape (a placement's copies share
 *  one lock, so naming each would only repeat it). */
export function lockedAmong(scene: LockScene, nodes: readonly NodeRef[]): NodeRef[] {
  return nodes.filter((n) => isLocked(scene, n))
}

/** The unlocked lockable nodes among `nodes`. */
export function unlockedAmong(scene: LockScene, nodes: readonly NodeRef[]): NodeRef[] {
  return nodes.filter((n) => isLockableKind(n.kind) && !isLocked(scene, n))
}

/** Unlock everything in the document. One undo step. Returns whether anything changed. */
export function unlockAll(scene: WasmScene): boolean {
  const locked = allLockableNodes(scene).filter((n) => isLocked(scene, n))
  if (locked.length === 0) return false
  setNodesLocked(scene, locked, false)
  return true
}
