import { describe, expect, it } from 'vitest'
import { shouldSkipToolSwitch } from './toolSwitchGuard'
import { CameraRig } from './cameraRig'

describe('shouldSkipToolSwitch — pure predicate', () => {
  it('skips when the requested name matches the last applied name and nothing forced a reapply', () => {
    expect(shouldSkipToolSwitch('Look Around', 'Look Around', false)).toBe(true)
  })

  it('does not skip when the requested name differs from the last applied name', () => {
    expect(shouldSkipToolSwitch('Look Around', 'Position Camera', false)).toBe(false)
  })

  it('does not skip when nothing has ever been applied yet (pre-mount)', () => {
    expect(shouldSkipToolSwitch('Select', undefined, false)).toBe(false)
  })

  it('does not skip when a seq bump forced this run, even if already active — the explicit reselect path', () => {
    expect(shouldSkipToolSwitch('Look Around', 'Look Around', true)).toBe(false)
  })

  it('CRITICAL: a request for Select is not skipped just because the controller (post resetToSelect) also reads Select', () => {
    // The pre-fix guard's second argument was `toolController.activeToolName`.
    // Orbit/Pan/Zoom/Zoom Window/default all call `resetToSelect()`, so the
    // controller reads 'Select' while Orbit is the tool actually active.
    // The fixed guard's second argument is the last APPLIED REQUESTED name
    // ('Orbit'), which correctly never equals a later request for 'Select'.
    expect(shouldSkipToolSwitch('Select', 'Orbit', false)).toBe(false)
  })
})

/**
 * Harness replicating the real mechanism end to end: App.tsx's functional
 * `setActiveTool` (bails on an unchanged value) + `toolActivationSeq`
 * bump-on-explicit-activation, Viewport's `useLayoutEffect` on
 * `[activeToolProp, activeToolSeqProp]` guarded by `shouldSkipToolSwitch`,
 * and `switchToolRef`'s body: the walkthrough-exit reseed (keyed on the
 * OUTGOING tool, using the real `CameraRig.effectiveDistance` so the k/k²
 * math is the production math, not a re-derivation of it), AND the
 * camera-mode side effects (`cameraModeRef`, `controls.mouseButtons.LEFT`,
 * `zoomWindowActive`) together with the `toolController.resetToSelect()`
 * call that Orbit/Pan/Zoom/Zoom Window/default all make.
 *
 * That last piece is what the ORIGINAL version of this harness got wrong
 * (camera delta review, "mechanism-replica test models the wrong
 * semantics"): it modeled `controllerActiveToolName = name` for every tool,
 * including Orbit/Pan/Zoom/Zoom Window — i.e. it pretended the controller
 * tracks the camera tool by name, when the real `switchToolRef` calls
 * `toolController.resetToSelect()` for those and the controller genuinely
 * reads 'Select' while Orbit is active. That wrong replica was exactly
 * faithful enough to exercise the echo-suppression fix (which never
 * touches Select) while masking the critical: a real prop-switch to
 * 'Select' while Orbit is active got silently swallowed by the guard, and
 * the old harness had no way to observe it because it never modeled
 * `cameraModeRef` / `mouseButtons.LEFT` / `zoomWindowActive` at all.
 */
function makeHarness() {
  const WALKTHROUGH_TOOL_NAMES = new Set(['Position Camera', 'Look Around', 'Walk'])
  const CAMERA_HANDOFF_TOOL_NAMES = new Set([
    'Orbit', 'Pan', 'Zoom', 'Zoom Window', 'Position Camera', 'Look Around', 'Walk',
  ])

  const rig = new CameraRig(1)
  rig.setFov(60)

  // switchToolRef's mutable state — mirrors Viewport.tsx's closures over
  // `toolController`, `cameraModeRef`, `controls.mouseButtons.LEFT`, and
  // `zoomWindowActive`.
  let controllerActiveToolName = 'Select' // ToolController's constructor default
  let cameraModeRef = false
  let mouseButtonsLeft: string | null = null
  let zoomWindowActive = false
  let distance = 10 // stand-in for controls.getDistance()
  let switchExecutionCount = 0
  let reseedCount = 0

  // App.tsx state
  let activeTool = 'Select'
  let toolActivationSeq = 0

  // What the layout effect last saw (React's dependency-array bookkeeping —
  // distinct from the guard's own `lastAppliedToolSeqRef`).
  let effectSeenName: string | undefined
  let effectSeenSeq: number | undefined
  // The guard's own last-applied seq (mirrors `lastAppliedToolSeqRef` in Viewport.tsx).
  let lastAppliedSeq: number | undefined
  // The guard's own last-applied NAME (mirrors `lastAppliedToolNameRef` in
  // Viewport.tsx) — the fixed guard's comparison source, set inside
  // `switchToolRef` on EVERY invocation, before the resetToSelect-mapping
  // switch below runs.
  let lastAppliedToolName: string | undefined

  function switchToolRef(name: string): void {
    switchExecutionCount++
    if (WALKTHROUGH_TOOL_NAMES.has(controllerActiveToolName)) {
      distance = rig.effectiveDistance(distance)
      reseedCount++
    }
    // Mirrors Viewport.tsx: `lastAppliedToolNameRef.current = toolName` is
    // the very first thing `switchToolRef.current` does, before anything
    // below (including the camera-mode cases) can overwrite the
    // controller's own name via `resetToSelect()`.
    lastAppliedToolName = name
    zoomWindowActive = false
    switch (name) {
      case 'Orbit':
        cameraModeRef = true
        mouseButtonsLeft = 'ROTATE'
        controllerActiveToolName = 'Select' // toolController.resetToSelect()
        break
      case 'Pan':
        cameraModeRef = true
        mouseButtonsLeft = 'PAN'
        controllerActiveToolName = 'Select'
        break
      case 'Zoom':
        cameraModeRef = true
        mouseButtonsLeft = 'DOLLY'
        controllerActiveToolName = 'Select'
        break
      case 'Zoom Window':
        // NOT cameraModeRef — this mode owns left-drag itself.
        cameraModeRef = false
        zoomWindowActive = true
        mouseButtonsLeft = null
        controllerActiveToolName = 'Select'
        break
      case 'Position Camera':
      case 'Look Around':
      case 'Walk':
        cameraModeRef = false
        mouseButtonsLeft = null
        controllerActiveToolName = name
        break
      default:
        // 'Select' and every other named (non-camera) tool: the real
        // switch has no dedicated 'Select' case, so it falls through to
        // `default`, which also calls `resetToSelect()`.
        cameraModeRef = false
        mouseButtonsLeft = null
        controllerActiveToolName = 'Select'
    }
    // onInternalToolChange — App.tsx's handleInternalToolChange: bails on an unchanged value.
    if (activeTool !== name) {
      activeTool = name
      maybeRunEffect()
    }
  }

  function maybeRunEffect(): void {
    if (activeTool === effectSeenName && toolActivationSeq === effectSeenSeq) return
    effectSeenName = activeTool
    effectSeenSeq = toolActivationSeq
    const seqChanged = toolActivationSeq !== lastAppliedSeq
    if (!shouldSkipToolSwitch(activeTool, lastAppliedToolName, seqChanged)) {
      switchToolRef(activeTool)
    }
    lastAppliedSeq = toolActivationSeq
  }

  function activateTool(name: string): void {
    activeTool = name
    if (CAMERA_HANDOFF_TOOL_NAMES.has(name)) toolActivationSeq += 1
    maybeRunEffect()
  }

  /** The internal auto-handoff Viewport makes on its own — Position Camera
   * placing the eye and calling `switchToolRef.current?.('Look Around')`
   * directly, NOT through the prop-driven effect. */
  function internalHandoff(name: string): void {
    switchToolRef(name)
  }

  return {
    rig,
    activateTool,
    internalHandoff,
    getSwitchExecutionCount: () => switchExecutionCount,
    getReseedCount: () => reseedCount,
    getDistance: () => distance,
    getControllerActiveToolName: () => controllerActiveToolName,
    getCameraModeRef: () => cameraModeRef,
    getMouseButtonsLeft: () => mouseButtonsLeft,
    getZoomWindowActive: () => zoomWindowActive,
  }
}

describe('tool-switch echo — full mechanism replica (Position Camera → Look Around)', () => {
  it('runs the switch exactly once for the internal handoff and reseeds exactly once, at distance k·d0 (not k²·d0)', () => {
    const h = makeHarness()
    h.activateTool('Position Camera')
    const executionsBeforeHandoff = h.getSwitchExecutionCount()
    const d0 = h.getDistance()

    h.internalHandoff('Look Around')

    // The pre-fix replica saw TWO switch executions here (the direct call,
    // then the unguarded echo through the layout effect); the guard leaves
    // exactly one.
    expect(h.getSwitchExecutionCount() - executionsBeforeHandoff).toBe(1)
    expect(h.getControllerActiveToolName()).toBe('Look Around')

    // Exactly one reseed, at k·d0 — not k²·d0 (the compounding the finding
    // describes: 1.39x at 60° if it happened twice would instead read ~1.94x).
    const k = h.getDistance() / d0
    expect(k).toBeGreaterThan(1) // fov 60 > the 45-degree reference, so k > 1
    expect(h.getDistance()).toBeCloseTo(d0 * k, 10)
    expect(h.getDistance()).not.toBeCloseTo(d0 * k * k, 5)
  })

  it('still forces a real reapply (and a fresh reseed) when the same tool is explicitly reselected — the seq-bump path', () => {
    const h = makeHarness()
    h.activateTool('Position Camera')
    h.internalHandoff('Look Around')
    const executionsAfterHandoff = h.getSwitchExecutionCount()
    const distanceAfterHandoff = h.getDistance()

    // Explicit re-choice of the already-active 'Look Around' entry (Camera
    // menu / rail) — App.tsx's `activateTool` bumps `toolActivationSeq`
    // for exactly this case.
    h.activateTool('Look Around')

    expect(h.getSwitchExecutionCount()).toBe(executionsAfterHandoff + 1)
    // Look Around was still the outgoing (walkthrough) tool at reapply
    // time, so this forced reapply reseeds again — a genuine explicit
    // reactivation, not an echo.
    expect(h.getDistance()).not.toBe(distanceAfterHandoff)
  })
})

describe('CRITICAL regression — prop-switch to Select while a camera tool is active must not be swallowed', () => {
  it('Orbit active, rail click on Select (no seq bump): the switch body runs and camera-mode state is cleared', () => {
    const h = makeHarness()
    h.activateTool('Orbit')

    // Orbit's own switch body ran: cameraModeRef armed, LEFT remapped to
    // ROTATE, and `toolController.resetToSelect()` means the controller
    // itself already reads 'Select' — even though Orbit is the tool
    // actually active from the rail's point of view. This is exactly the
    // controller state the pre-fix guard mistook for "Select already
    // active, skip".
    const executionsAfterOrbit = h.getSwitchExecutionCount()
    expect(h.getCameraModeRef()).toBe(true)
    expect(h.getMouseButtonsLeft()).toBe('ROTATE')
    expect(h.getControllerActiveToolName()).toBe('Select')

    // 'Select' is not a camera-handoff tool name, so App.tsx's
    // `activateTool` does NOT bump `toolActivationSeq` for this click —
    // exactly the repro from the finding ("no seq bump — 'Select' is not a
    // handoff tool").
    h.activateTool('Select')

    // The switch body MUST have run for this transition. Under the 50bd639
    // guard (keyed on `toolController.activeToolName`, which already read
    // 'Select' before this click) this whole assertion block fails: the
    // guard swallows the switch, and cameraModeRef/mouseButtons.LEFT stay
    // armed for Orbit while the rail shows Select — the viewport is wedged
    // in camera-navigation mode.
    expect(h.getSwitchExecutionCount()).toBe(executionsAfterOrbit + 1)
    expect(h.getCameraModeRef()).toBe(false)
    expect(h.getMouseButtonsLeft()).toBe(null)
    expect(h.getControllerActiveToolName()).toBe('Select')
  })
})
