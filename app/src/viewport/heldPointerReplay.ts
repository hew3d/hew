/**
 * heldPointerReplay — whether, and how, the Viewport re-runs the snap at the
 * pointer's last position without a pointer event (`reResolveAtHeldPointer`:
 * a precision key held or released, a render-mode switch, and for a tool that
 * follows the view an idle key, Shift or the camera settling).
 *
 * Pure decision logic beside dragMove.ts, so the guards are unit-tested
 * directly rather than only through a browser.
 */

export interface HeldPointerState {
  /** A ray is cached from the last pointer move over the canvas. */
  hasHeldRay: boolean
  /** The snap path is live: no camera navigation, marquee, deferred tap or
   *  drag-move owns the pointer. */
  snapPathLive: boolean
  /** The pointer is within the canvas's bounds: set by its last pointer move
   *  or enter, cleared when it leaves them (`leftCanvasBounds`). A cached
   *  ray outlives the pointer's leaving (to the menu bar, a panel), and a
   *  replay then would put a cue where the user no longer is. */
  pointerInside: boolean
  /** What the layout puts at the pointer's last client position now
   *  (`document.elementFromPoint`): an overlay that has come up over a
   *  parked pointer (the palette, a dialog) covers the canvas without a
   *  pointer event to say so. */
  elementAtPoint: Element | null
  /** The canvas itself: the point must still show it, not a dock, panel or
   *  overlay over it. */
  container: { contains(node: Node | null): boolean }
}

/** Replay only for a held ray, on a live snap path, with the pointer inside
 *  the canvas and nothing covering the point it rests on. */
export function shouldReplayHeldPointer(s: HeldPointerState): boolean {
  return (
    s.hasHeldRay &&
    s.snapPathLive &&
    s.pointerInside &&
    s.elementAtPoint !== null &&
    s.container.contains(s.elementAtPoint)
  )
}

/** Whether the held-pointer hover replay after an idle key, Shift or the
 *  camera settling applies to `tool`: only a tool whose idle hover follows
 *  the view (`Tool.followsViewPlane`) has an answer those events change
 *  under a still cursor. Replaying any other tool's `onPointerMove` there
 *  would re-hover where the cursor rests now in a state that never asked
 *  for it, e.g. Follow Me backing up to profile-pick on Escape mid-drag,
 *  whose hint must keep the verdict of the profile the drag was armed on. */
export function followsViewPlane(tool: object): boolean {
  return (tool as { followsViewPlane?: unknown }).followsViewPlane === true
}

/** A screen-space drag tool (Look Around, Walk) has no world pick of its
 *  own: the replay hands it a null snap instead of resolving one, as the
 *  pointer-move path does. */
export function replaysWithoutSnap(tool: object): boolean {
  return 'onPointerRawMove' in tool
}

/**
 * Whether a `pointerleave` on the canvas means the pointer really went
 * elsewhere: its position is outside the canvas's bounds. An overlay coming
 * up over a pointer resting on the canvas (the command palette, a dialog)
 * also fires a leave, at the very same position — that pointer has not
 * moved, and when the overlay goes it is over the canvas again without any
 * pointer event to say so. `shouldReplayHeldPointer`'s point probe covers
 * the overlay while it is up.
 */
export function leftCanvasBounds(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; right: number; bottom: number },
): boolean {
  return clientX < rect.left || clientX >= rect.right || clientY < rect.top || clientY >= rect.bottom
}
