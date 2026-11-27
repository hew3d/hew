/**
 * drawPlanePips — what a click on one of the view die's four plane pips
 * does (view-die design §3.3), as a pure decision the Viewport's `pipClick`
 * carries out. A pip click is the mouse form of the keys that already
 * exist:
 *
 * | Pip | Idle draw tool | During a gesture |
 * |---|---|---|
 * | Red, green, blue | The matching arrow key (→ red, ← green, ↑ blue): lock that plane, or release it if it is the one locked; under a Shift pin another axis pip releases the pin and locks its plane, the pinned pip releases the pin. | The arrow key at that stage of that tool. |
 * | Custom | Enabled only when a custom plane is pinned, is the tracked hover plane, or is the live custom candidate (the last custom plane hovered, for 2 s, and while the pointer is off the canvas on its way here): pin that plane, or release the pin. | Nothing. |
 *
 * With a tool that draws on no plane every pip is disabled and a click does
 * nothing.
 */
import type { DrawingAxes } from '../tools/drawingAxes'
import { pipArrowKey, planePip, type DrawPlane, type DrawPlaneState, type PlanePip } from '../tools/drawPlane'

/** What the active tool offers a pip click right now. */
export interface PipContext {
  /** The tool's plane state, or null for a tool that draws on no plane. */
  state: DrawPlaneState | null
  /** The plane a Shift press would act on (`Tool.pinnablePlane`): the pin,
   *  else the tracked hover plane; null mid-gesture or with none. */
  pinnable: DrawPlane | null
  /** The plane the custom pip acts on (`Tool.customPipPlane`): a pinned
   *  custom plane, a custom tracked hover, or the live custom candidate;
   *  null mid-gesture or with none. */
  customPlane: DrawPlane | null
  /** Whether the key router would hand an arrow key to the tool's
   *  capturing branch (a gesture under way). */
  capturing: boolean
  frame: DrawingAxes
  /** Shop Mode's read-only viewer: no pip does anything. */
  readOnly: boolean
  /** Whether the pip's arrow key at the tool's current stage would release
   *  the held plane lock (`Tool.planeKeyReleases`); false for the custom
   *  pip. */
  arrowReleases: boolean
}

/** The action `pipClick` performs. */
export type PipAction =
  | { kind: 'none' }
  | { kind: 'togglePin' }
  | { kind: 'pinCustom' }
  | { kind: 'key'; key: 'ArrowRight' | 'ArrowLeft' | 'ArrowUp' }

/** Whether a click on `pip` does anything right now. */
export function pipEnabledFor(pip: PlanePip, ctx: PipContext): boolean {
  if (ctx.readOnly || ctx.state === null) return false
  if (pip !== 'custom') return true
  return ctx.customPlane !== null && planePip(ctx.customPlane.normal, ctx.frame) === 'custom'
}

/** What a click on `pip` does (the table in the module doc). */
export function pipClickAction(pip: PlanePip, ctx: PipContext): PipAction {
  if (!pipEnabledFor(pip, ctx)) return { kind: 'none' }
  const key = pipArrowKey(pip)
  if (key === null) return { kind: 'pinCustom' }
  // The held pip of a Shift pin, while the tool can still release it
  // (`pinnable` is null mid-gesture): release the pin. Any other axis pip is
  // its arrow, which displaces a pin and locks that plane.
  if (isPinnedPip(pip, ctx)) return { kind: 'togglePin' }
  return { kind: 'key', key }
}

function isPinnedPip(pip: PlanePip, ctx: PipContext): boolean {
  const state = ctx.state
  return ctx.pinnable !== null && state !== null && state.hold === 'pinned' && state.pip === pip
}

/**
 * Whether a click on `pip` would RELEASE the held plane (the tooltip then
 * starts with "Release"): the pinned pip of a Shift pin, the custom pip of a
 * pinned custom plane, or an axis pip whose arrow at this stage releases the
 * lock. Mid-chain in Line an arrow locks the segment's axis, so the lit pip
 * there does not release.
 */
export function pipWillReleaseFor(pip: PlanePip, ctx: PipContext): boolean {
  if (!pipEnabledFor(pip, ctx)) return false
  if (isPinnedPip(pip, ctx)) return true
  if (pip === 'custom') return false
  return ctx.arrowReleases
}

/** The pips in their row order: red, green, blue, custom. */
export const PLANE_PIPS: readonly PlanePip[] = ['red', 'green', 'blue', 'custom']

/**
 * The drawing plane the snap dot's ring and the chip's plane word name
 * (view-die design §4), or undefined for none: no draw tool, or no hover the
 * next click would act on — off the canvas, or inside an entered object off
 * that object's faces, where a click does nothing (the pips keep showing
 * the last plane; the cursor does not claim it).
 */
export function cursorPlaneOf(state: DrawPlaneState | null): { pip: PlanePip; hold: DrawPlaneState['hold'] } | undefined {
  if (state === null || state.hover === null) return undefined
  return { pip: state.pip, hold: state.hold }
}
