import { describe, expect, it } from 'vitest'
import { CAMERA_HANDOFF_TOOL_NAMES } from './cameraHandoffTools'

// This module used to also host `deriveMenuActiveTool`, a workaround for
// `activeTool` (App.tsx state) going stale across a camera-tool auto-handoff
// or Escape-to-Select — the tests that pinned ITS behavior (Position Camera
// → Look Around reads as checked, Escape leaves nothing in the Camera group
// checked, a drag-to-move's transient 'Move' never leaks into the menu, …)
// now live at the level the fix actually operates: App.test.tsx's "walkthrough
// handoff" and "internal tool change" describe blocks assert the SAME
// end-to-end behavior against `activeTool` directly (via Viewport's
// `onInternalToolChange`), since there is no more derivation step to unit
// test in isolation. What remains here is just the lookup table
// `activateTool`'s `toolActivationSeq` forcing depends on.
describe('CAMERA_HANDOFF_TOOL_NAMES — App.tsx toolActivationSeq scoping', () => {
  it('covers every camera tool whose real activity can change without an explicit re-selection', () => {
    for (const name of ['Orbit', 'Pan', 'Zoom', 'Zoom Window', 'Position Camera', 'Look Around', 'Walk']) {
      expect(CAMERA_HANDOFF_TOOL_NAMES.has(name)).toBe(true)
    }
  })

  it('excludes non-camera tools', () => {
    for (const name of ['Select', 'Rectangle', 'Move', 'Push/Pull']) {
      expect(CAMERA_HANDOFF_TOOL_NAMES.has(name)).toBe(false)
    }
  })
})
