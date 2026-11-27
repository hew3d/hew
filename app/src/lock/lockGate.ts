/**
 * Lock — how a command that meets locked things answers.
 *
 * Three shapes, one per kind of command (OW-1, docs/design D1):
 *
 * - Commands on the SELECTION that run on what they can (Delete, Move,
 *   Rotate, Scale, Flip, Group, tag and paint applied to a selection):
 *   `leaveBehind`. Locked items are left behind, the command runs on the
 *   rest, and ONE toast says how many were left alone. If everything is
 *   locked nothing runs and the toast is the "Locked." line with Unlock.
 * - Commands that need ALL their operands (Union, Make Component, Explode,
 *   Ungroup, link): `refuseIfLocked`. Refused whole, with the "Locked."
 *   toast.
 * - Tools on the thing under the cursor: `lockHintFor` (lockModel.ts) is the
 *   line beside the cursor, and `toastLocked` the click's toast.
 *
 * The toast's buttons come from its code (`lockedHintCode`); the app builds
 * them. Under all of it the kernel stays strict: an API call naming a locked
 * node is refused whole. "Left behind" is the app choosing what to send.
 */

import type { NodeRef } from '../panels/treeModel'
import {
  blockForCause,
  containerFreezeCause,
  copyAllowed,
  gateAllOperands,
  gateSelection,
  leftBehindMessage,
  lockBlockFor,
  lockedHintCode,
  lockedMessage,
  type LockBlock,
  type LockScene,
} from './lockModel'

export type LockToast = (message: string, code?: string) => void

/** The "Locked." toast for a block: the line, and the code that makes the
 *  app add Unlock (and Make Unique) buttons for the right node. */
export function toastLocked(toast: LockToast, node: NodeRef, block: LockBlock): void {
  const target = block.indirect ? (block.uniqueTarget ?? node) : (block.cause ?? node)
  toast(lockedMessage(block), lockedHintCode(target, block.indirect))
}

/**
 * Splits `nodes` for a command that runs on what it can. Returns what to run
 * on (possibly all of `nodes`), or null when nothing should run (everything
 * is locked, and the toast said so). Says "N locked items were left alone."
 * once when some were skipped.
 */
export function leaveBehind(
  scene: LockScene,
  nodes: readonly NodeRef[],
  toast: LockToast,
  opts: { descendants?: boolean } = {},
): NodeRef[] | null {
  const outcome = gateSelection(scene, nodes, opts)
  switch (outcome.kind) {
    case 'clear':
      return outcome.go
    case 'left-behind':
      toast(leftBehindMessage(scene, outcome.left))
      return outcome.go
    case 'refused':
      toastLocked(toast, outcome.node, outcome.block)
      return null
  }
}

/**
 * The gate for COPYING a selection (Move with the copy modifier): a copy of a
 * locked node is allowed, the original stays put. Only a node frozen by what
 * holds it (a locked group above, a frozen shape) is refused, whole, with the
 * "Locked." toast. Returns the nodes to copy, or null after the toast.
 */
export function gateCopyOperands(
  scene: LockScene,
  nodes: readonly NodeRef[],
  toast: LockToast,
): NodeRef[] | null {
  for (const n of nodes) {
    if (!copyAllowed(scene, n)) {
      // A locked node inside a locked container: name the container, the
      // thing a person has to unlock first.
      const container = containerFreezeCause(scene, n)
      const block = container !== null ? blockForCause(scene, container) : lockBlockFor(scene, n)
      if (block !== null) {
        toastLocked(toast, n, block)
        return null
      }
    }
  }
  return [...nodes]
}

/** For a command that needs every operand: true (after toasting) when one is
 *  locked, so the caller stops. */
export function refuseIfLocked(
  scene: LockScene,
  nodes: readonly NodeRef[],
  toast: LockToast,
  opts: { descendants?: boolean } = {},
): boolean {
  const blocked = gateAllOperands(scene, nodes, opts)
  if (blocked === null) return false
  toastLocked(toast, blocked.node, blocked.block)
  return true
}
