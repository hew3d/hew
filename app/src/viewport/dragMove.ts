/**
 * dragMove — pure decision logic for the Select tool's drag-to-move gesture
 * ("dragging an object moves it, like a standard OS drag").
 *
 * A press on a movable node arms a drag-move instead of a marquee; past a
 * small pixel threshold the Viewport hands the gesture to a one-shot Move
 * tool (full inference/snapping/VCB), and the tool springs back to Select on
 * release. A plain release under the threshold is still an ordinary click
 * (click ≠ drag). Pure data in, pure data out — unit-testable directly.
 */

import type { NodeRef } from '../panels/treeModel'
import { nodeKey } from '../panels/treeModel'

/**
 * Pixels of pointer travel that turn a press into a drag. Matches the
 * marquee's own threshold so "click" means the same thing on an object and
 * on empty space.
 */
export const DRAG_MOVE_THRESHOLD_PX = 5

/** Has the pointer traveled far enough from the press to count as a drag? */
export function exceedsDragThreshold(
  startX: number,
  startY: number,
  x: number,
  y: number,
): boolean {
  return Math.hypot(x - startX, y - startY) >= DRAG_MOVE_THRESHOLD_PX
}

/**
 * What a drag starting on `pressed` should move. OS convention: dragging a
 * member of the current multi-selection drags the WHOLE selection; dragging
 * anything else drags just that node (the drag will also select it).
 */
export function dragMoveTargets(
  pressed: NodeRef,
  selection: readonly NodeRef[],
): NodeRef[] {
  const pressedKey = nodeKey(pressed)
  if (selection.some((n) => nodeKey(n) === pressedKey)) {
    return [...selection]
  }
  return [pressed]
}
