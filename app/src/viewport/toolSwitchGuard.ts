/**
 * toolSwitchGuard — the tool-switch layout effect's last-applied guard
 * (camera-fov-fixes delta review, "onInternalToolChange echo double-runs
 * the walkthrough-exit reseed"; camera delta review, "guard wedges the
 * viewport in camera mode when the controller reads 'Select'").
 *
 * Viewport's tool-switch `useLayoutEffect` (deps `[activeToolProp,
 * activeToolSeqProp]`) re-invokes `switchToolRef` whenever either prop
 * changes. `activeToolProp` changes for two different reasons that look
 * identical from the effect's point of view:
 *
 *   1. An explicit parent-driven switch (menu click, toolbar, keyboard).
 *   2. The ECHO of an internal transition Viewport itself just made
 *      (`onInternalToolChange`, see its doc comment on the Props interface)
 *      — e.g. Position Camera's auto-handoff to Look Around. By the time
 *      this echo reaches the effect, the tool controller has ALREADY been
 *      switched to the new tool; re-running `switchToolRef` a second time
 *      re-executes the whole switch body for a transition that already
 *      happened, including the walkthrough-exit reseed branch (keyed only
 *      on the OUTGOING tool being a walkthrough tool) and
 *      `toolController.setTool(...)` — tearing down and replacing the
 *      tool instance the first invocation just created, and, if the
 *      now-active tool is ITSELF a walkthrough tool, reseeding
 *      `controls.target` a second time against a target the first reseed
 *      already moved (compounding the reseed distance to k² instead of k).
 *
 * `shouldSkipToolSwitch` tells the effect when it's looking at case 2: the
 * requested name matches the name `switchToolRef` was LAST INVOKED WITH,
 * and nothing forced a reapply. That comparison MUST be against the last
 * applied tool name — not against `toolController.activeToolName` — because
 * the switch body's own Orbit/Pan/Zoom/Zoom Window/default cases
 * deliberately call `toolController.resetToSelect()`, leaving the
 * controller reporting 'Select' while one of those camera tools is
 * genuinely active. Comparing against the controller's name made the guard
 * fire for an entirely unrelated later switch TO 'Select' (e.g. clicking
 * Select in the rail while Orbit is active): requested name 'Select'
 * equalled the controller's 'Select', so the guard skipped the ENTIRE
 * switch body, leaving `cameraModeRef`/`mouseButtons.LEFT`/
 * `zoomWindowActive` armed for the camera tool the rail no longer showed —
 * the viewport stayed wedged in camera-navigation mode. Keying the
 * comparison on the last-applied REQUESTED name instead of the
 * controller's ACTUAL name fixes this: Orbit's requested name is 'Orbit',
 * so a later request for 'Select' never matches it.
 *
 * "Nothing forced a reapply" must still let an EXPLICIT re-selection of an
 * already-active camera tool through — App.tsx's `toolActivationSeq` exists
 * for exactly that (see its doc comment) — so the guard only fires when the
 * activation sequence number is unchanged from the last time this effect
 * actually ran.
 */

/**
 * Pure predicate: should the tool-switch effect skip re-invoking
 * `switchToolRef` this run?
 *
 * @param requestedName - `activeToolProp` for this run.
 * @param lastAppliedToolName - the `toolName` argument `switchToolRef` was
 *   last invoked with (`lastAppliedToolNameRef.current` in Viewport.tsx),
 *   set on EVERY invocation regardless of call site (prop-driven switch,
 *   internal handoff, Escape revert, one-shot). Deliberately NOT
 *   `toolController.activeToolName`: several switch-body branches
 *   overwrite that with 'Select' via `resetToSelect()` while a DIFFERENT
 *   named tool (Orbit/Pan/Zoom/Zoom Window) is actually active — see the
 *   file doc comment. `undefined` before the first switch has ever been
 *   applied (first paint, before the mount effect has run).
 * @param seqChanged - whether `activeToolSeqProp` differs from the value
 *   it held the last time this effect ran a real switch. A seq bump is an
 *   explicit forced-reapply request (App.tsx `activateTool`) and must
 *   never be swallowed by this guard, even when the requested name is
 *   already active.
 */
export function shouldSkipToolSwitch(
  requestedName: string,
  lastAppliedToolName: string | undefined,
  seqChanged: boolean,
): boolean {
  return !seqChanged && lastAppliedToolName === requestedName
}
