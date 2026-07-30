/**
 * CAMERA_HANDOFF_TOOL_NAMES — the Camera ▸ … tools whose real activity can
 * change from something OTHER than an explicit user re-selection: Position
 * Camera auto-hands off to Look Around on placement, and every camera/
 * walkthrough tool returns to Select on Escape or a completed one-shot
 * gesture (Zoom Window).
 *
 * `App.tsx`'s `activateTool` uses this set to decide when an explicit
 * reselect of an already-`activeTool`-valued entry (e.g. re-choosing Orbit
 * from the Camera menu while Orbit is nominally already active) needs to be
 * FORCED through to Viewport rather than trusted as a no-op — see
 * `toolActivationSeq`'s doc comment there.
 *
 * This module used to also host `deriveMenuActiveTool`, a workaround for
 * `activeTool` going stale on exactly these transitions (Viewport reported
 * them only through the status-bar signal, never through `setActiveTool`).
 * That gap is closed at the source now: Viewport's `onInternalToolChange`
 * callback fires from switchToolRef's single entry point on every internal
 * transition, camera or not, keeping `activeTool` truthful throughout. The
 * Camera/Draw/Modify menus, the tool rail, and the contextual dock all read
 * `activeTool` directly as a result — no derivation needed to keep them in
 * agreement.
 */
export const CAMERA_HANDOFF_TOOL_NAMES: ReadonlySet<string> = new Set([
  'Orbit', 'Pan', 'Zoom', 'Zoom Window', 'Position Camera', 'Look Around', 'Walk',
])
