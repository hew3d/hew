/**
 * Direct unit tests for `computeEditContext`/`applyEditContext` — the A1
 * breadcrumb-to-EditContext reduction and the tool-push helper. Both are
 * exported, pure(-ish) functions, but every prior exercise of them was
 * indirect: every tool test hand-constructs an `EditContext` literal and
 * injects it via `tool.setEditContext(...)`, bypassing the reduction logic
 * entirely, and the only e2e coverage drives just the single instance-
 * context path (dblclick in, Escape out) — never the object/group branches
 * or the stale-instance-degrades-to-top fallback this file's own doc
 * comment documents.
 */
import { describe, it, expect, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { SaveToLibraryPopover } from '../panels/SaveToLibraryPopover'
import {
  computeEditContext,
  computeLitInstances,
  applyEditContext,
  isToolSwitchAllowedUnderReadOnly,
  mapUndoRedoError,
  keyboardBelongsToFieldOrDialog,
  cursorFor,
  createIsolationController,
} from './Viewport'
import type { IsolationScene } from './isolation'
import { setShowToolPointer } from '../settings/view'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import type { EditContext, Tool } from '../tools/types'

function makeScene(defs: Map<bigint, bigint>): WasmScene {
  return {
    instance_def: vi.fn((id: bigint) => defs.get(id)),
  } as unknown as WasmScene
}

/** A scene whose only wired-up query is `group_members`, keyed by group id. */
function makeGroupScene(members: Map<bigint, { kind: string; id: bigint }[]>): WasmScene {
  return {
    group_members: vi.fn((gid: bigint) => members.get(gid) ?? []),
  } as unknown as WasmScene
}

describe('computeEditContext', () => {
  it('an empty path is the top-level context', () => {
    const scene = makeScene(new Map())
    expect(computeEditContext(scene, [])).toEqual({ kind: 'top' })
  })

  it('the deepest OBJECT entry becomes an object context', () => {
    const scene = makeScene(new Map())
    const path: NodeRef[] = [{ kind: 'object', id: 7n }]
    expect(computeEditContext(scene, path)).toEqual({ kind: 'object', id: 7n })
  })

  it('the deepest GROUP entry becomes a group context', () => {
    const scene = makeScene(new Map())
    const path: NodeRef[] = [{ kind: 'group', id: 9n }]
    expect(computeEditContext(scene, path)).toEqual({ kind: 'group', id: 9n })
  })

  it('the deepest INSTANCE entry resolves its definition via instance_def', () => {
    const scene = makeScene(new Map([[42n, 5n]]))
    const path: NodeRef[] = [{ kind: 'instance', id: 42n }]
    expect(computeEditContext(scene, path)).toEqual({ kind: 'instance', id: 42n, component: 5n })
  })

  it('only the DEEPEST entry matters — an object nested under an instance path resolves to an object context', () => {
    const scene = makeScene(new Map([[42n, 5n]]))
    const path: NodeRef[] = [{ kind: 'instance', id: 42n }, { kind: 'object', id: 7n }]
    expect(computeEditContext(scene, path)).toEqual({ kind: 'object', id: 7n })
  })

  it('a stale/hidden deepest instance (instance_def misses) degrades to top rather than throwing', () => {
    const scene = makeScene(new Map()) // 42n resolves to undefined
    const path: NodeRef[] = [{ kind: 'instance', id: 42n }]
    expect(computeEditContext(scene, path)).toEqual({ kind: 'top' })
  })

  it('a deepest entry of an unexpected kind (defensive fallback) degrades to top', () => {
    const scene = makeScene(new Map())
    const path: NodeRef[] = [{ kind: 'draft-edge', id: 1n, ctx: '{"plane":[0,0,0,0,0,1]}', key: '0000000000000001' }]
    expect(computeEditContext(scene, path)).toEqual({ kind: 'top' })
  })
})

describe('computeLitInstances (delta-review Findings 2 & 3 on component-edit-parity.md)', () => {
  it('an empty path lights no instances', () => {
    const scene = makeGroupScene(new Map())
    expect(computeLitInstances(scene, [])).toBeNull()
  })

  it('the deepest INSTANCE entry lights that instance', () => {
    const scene = makeGroupScene(new Map())
    const path: NodeRef[] = [{ kind: 'instance', id: 10n }]
    expect(computeLitInstances(scene, path)).toEqual(new Set([10n]))
  })

  it('Finding 2: [instance, object] keeps the ENCLOSING instance lit even though the deepest node is the object', () => {
    // Double-clicking a definition member while already inside its instance's
    // editing context: the deepest node is the member object, not the
    // instance — the fix must walk the whole chain, not just the tail.
    const scene = makeGroupScene(new Map())
    const path: NodeRef[] = [{ kind: 'instance', id: 10n }, { kind: 'object', id: 2n }]
    expect(computeLitInstances(scene, path)).toEqual(new Set([10n]))
  })

  it('Finding 3: a GROUP context also lights its own member instances (not just leaf objects)', () => {
    const scene = makeGroupScene(
      new Map([[9n, [{ kind: 'object', id: 1n }, { kind: 'instance', id: 20n }]]]),
    )
    const path: NodeRef[] = [{ kind: 'group', id: 9n }]
    expect(computeLitInstances(scene, path)).toEqual(new Set([20n]))
  })

  it('Finding 3: a nested sub-group\'s instances light up too (recursive)', () => {
    const scene = makeGroupScene(
      new Map([
        [9n, [{ kind: 'group', id: 8n }]],
        [8n, [{ kind: 'instance', id: 21n }]],
      ]),
    )
    const path: NodeRef[] = [{ kind: 'group', id: 9n }]
    expect(computeLitInstances(scene, path)).toEqual(new Set([21n]))
  })

  it('an OBJECT context with no enclosing instance lights nothing (no instance to distinguish)', () => {
    const scene = makeGroupScene(new Map())
    const path: NodeRef[] = [{ kind: 'object', id: 7n }]
    expect(computeLitInstances(scene, path)).toBeNull()
  })
})

describe('applyEditContext', () => {
  function makeTool(withHook: boolean): { tool: Tool; setEditContext: ReturnType<typeof vi.fn> | undefined } {
    const setEditContext = withHook ? vi.fn() : undefined
    const tool = {
      onPointerMove: () => { /* no-op */ },
      onPointerDown: () => { /* no-op */ },
      onKey: () => { /* no-op */ },
      cancel: () => { /* no-op */ },
      name: 'Fake',
      ...(withHook ? { setEditContext } : {}),
    } as Tool
    return { tool, setEditContext }
  }

  it('pushes the context to a tool that implements setEditContext', () => {
    const { tool, setEditContext } = makeTool(true)
    const ctx: EditContext = { kind: 'instance', id: 9n, component: 90n }
    applyEditContext(tool, ctx)
    expect(setEditContext).toHaveBeenCalledTimes(1)
    expect(setEditContext).toHaveBeenCalledWith(ctx)
  })

  it('is a no-op for a tool without setEditContext — never throws', () => {
    const { tool } = makeTool(false)
    expect(() => applyEditContext(tool, { kind: 'top' })).not.toThrow()
  })
})

// ---------------------------------------------------------------------------
// Ctrl-tap-clean disarm surviving a swallowed Escape.
//
// Viewport's real `onCtrlKeyDown`/`onCtrlKeyUp` are closures over local state
// inside the giant setup `useEffect` (three.js scene, WASM scene, tool
// controller, ResizeObserver, …) and are not exported — mounting the real
// component in a unit test to drive them would need a disproportionate WebGL/
// wasm harness with no payoff beyond what's checked here. This instead proves
// the actual mechanism the fix relies on directly: a window-level listener
// registered in CAPTURE phase still observes a keydown that a document-level
// bubble-phase listener (standing in for a dialog's own Escape handler)
// stops from propagating further. `ctrlDisarmModel` below mirrors the real
// `ctrlTapClean` state machine bit-for-bit so the assertion tracks the actual
// bug: with a BUBBLE-phase listener the clean-tap flag survives the swallowed
// Escape and the following Control-up would wrongly fire
// `toggleCenterAnchor`; with CAPTURE it does not.
// ---------------------------------------------------------------------------
describe('ctrl-tap-clean disarm vs. a dialog stopPropagation-ing Escape', () => {
  function ctrlDisarmModel() {
    let ctrlTapClean = false
    return {
      onCtrlKeyDown(ev: KeyboardEvent): void {
        if (ev.key === 'Control') {
          if (!ev.repeat) ctrlTapClean = true
          return
        }
        ctrlTapClean = false
      },
      isArmed: () => ctrlTapClean,
    }
  }

  it('a CAPTURE-phase window listener still disarms on Escape even when a dialog stops the keydown at the bubble phase', () => {
    const model = ctrlDisarmModel()
    window.addEventListener('keydown', model.onCtrlKeyDown, true)
    // Stand-in for a dialog's own `document.addEventListener('keydown', ...)`
    // Escape handler, which stopPropagation()s at the bubble phase (see
    // dialogs.test.tsx's expectEscapeStopsPropagationToWindow).
    const dialogEscapeHandler = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') ev.stopPropagation()
    }
    document.addEventListener('keydown', dialogEscapeHandler)
    try {
      fireEvent.keyDown(document, { key: 'Control' })
      expect(model.isArmed()).toBe(true)

      fireEvent.keyDown(document, { key: 'Escape' })
      // Capture fires window → document, strictly before the dialog's
      // bubble-phase stopPropagation runs — so the model still sees Escape
      // and disarms, exactly as intended by registering onCtrlKeyDown with
      // the capture flag.
      expect(model.isArmed()).toBe(false)
    } finally {
      document.removeEventListener('keydown', dialogEscapeHandler)
      window.removeEventListener('keydown', model.onCtrlKeyDown, true)
    }
  })

  it('control case: a BUBBLE-phase listener is starved by the same stopPropagation, leaving the tap armed', () => {
    const model = ctrlDisarmModel()
    window.addEventListener('keydown', model.onCtrlKeyDown) // bubble phase — the pre-fix registration
    const dialogEscapeHandler = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') ev.stopPropagation()
    }
    document.addEventListener('keydown', dialogEscapeHandler)
    try {
      fireEvent.keyDown(document, { key: 'Control' })
      expect(model.isArmed()).toBe(true)

      fireEvent.keyDown(document, { key: 'Escape' })
      // The dialog's stopPropagation() at the bubble phase swallows the
      // Escape before it would reach the bubble-phase window listener, so
      // the flag never disarms — reproducing the bug the finding describes.
      expect(model.isArmed()).toBe(true)
    } finally {
      document.removeEventListener('keydown', dialogEscapeHandler)
      window.removeEventListener('keydown', model.onCtrlKeyDown)
    }
  })
})

// Shop-mode adversarial review, CRITICAL finding 1: `switchToolRef`'s own
// top-of-function refusal under `readOnly`. Extracted as a standalone pure
// predicate specifically so this contract is unit-testable without mounting
// the whole component/WASM stack — see the E2E "CONTRACT" specs in
// e2e/shop-mode.spec.ts for the end-to-end proof (a keyboard shortcut can't
// arm a real drag-to-move; a double-tap can't open a session).
describe('isToolSwitchAllowedUnderReadOnly (shop-mode adversarial review, CRITICAL finding 1)', () => {
  it('allows exactly Shop Mode\'s 3-tool registry', () => {
    expect(isToolSwitchAllowedUnderReadOnly('Select')).toBe(true)
    expect(isToolSwitchAllowedUnderReadOnly('Orbit')).toBe(true)
    expect(isToolSwitchAllowedUnderReadOnly('Tape Measure')).toBe(true)
  })

  it('refuses every editor-only tool, including the ones keyboard shortcuts reach directly', () => {
    // Rectangle/Push-Pull/Move/Rotate/Scale/Offset/Line/Circle/Arc all have
    // a bare-letter or number-key shortcut in Viewport.tsx's own onKeyDown
    // (r/p/m/q/s/f/l/c/a, 1-6) — exactly the paths this allowlist exists to
    // close off. Pan/Zoom/Zoom Window/Walk/Position Camera/Look Around are
    // camera tools Shop Mode's own gesture chrome (pinch/drag/two-finger
    // pan) already covers without a tool switch at all.
    for (const name of [
      'Rectangle', 'Circle', 'Polygon', 'Arc', 'Line', 'Push/Pull', 'Follow Me',
      'Offset', 'Paint', 'Position Texture', 'Move', 'Rotate', 'Scale', 'Dimension',
      'Text', 'Protractor', 'Slice', 'Section Plane', 'Edit Vertex', 'Drawing Axes',
      'Position Camera', 'Look Around', 'Walk', 'Pan', 'Zoom', 'Zoom Window',
      '', 'select', 'SELECT', 'Orbitt',
    ]) {
      expect(isToolSwitchAllowedUnderReadOnly(name)).toBe(false)
    }
  })
})

// runUndo/runRedo's error-mapping, extracted as a standalone pure function
// (mapUndoRedoError) specifically so this contract — a refused undo/redo's
// kernel copy reaching the user, but a poisoned instance's code-less throw
// left alone for the crash screen — is unit-testable without mounting the
// whole component/WASM stack.
describe('mapUndoRedoError', () => {
  it('maps a typed kernel refusal to its plain-language copy', () => {
    expect(mapUndoRedoError(new Error('RestoreConflicts: newer drawing is in the way'))).toEqual({
      code: 'RestoreConflicts',
      message:
        'Undo needs to put the original outline back, but newer drawing is in its way. Erase the overlapping lines and undo again.',
    })
  })

  it('maps every undo/redo-specific refusal code (InverseFailed, InverseDiverged)', () => {
    expect(mapUndoRedoError(new Error('InverseFailed: could not invert')))
      .toEqual({ code: 'InverseFailed', message: expect.stringContaining('Report Bug') })
    expect(mapUndoRedoError(new Error('InverseDiverged: digest mismatch')))
      .toEqual({ code: 'InverseDiverged', message: expect.stringContaining('Report Bug') })
  })

  it('returns null for a code-less error (a poisoned instance) — left to the crash screen', () => {
    expect(
      mapUndoRedoError(
        new Error('recursive use of an object detected which would lead to unsafe aliasing in rust'),
      ),
    ).toBeNull()
  })

  it('returns null for a non-Error throw with no CODE: prefix', () => {
    expect(mapUndoRedoError('boom')).toBeNull()
  })
})

// #36 follow-up: digits typed into Save to Library's name field reached the
// last rectangle's typed-measurement buffer. The viewport's key handler now
// asks this predicate before it routes anything to a tool.
describe('keyboardBelongsToFieldOrDialog', () => {
  it('is true for text fields and contenteditable, false for the page body', () => {
    const input = document.createElement('input')
    const area = document.createElement('textarea')
    const editable = document.createElement('div')
    Object.defineProperty(editable, 'isContentEditable', { value: true })
    expect(keyboardBelongsToFieldOrDialog(input)).toBe(true)
    expect(keyboardBelongsToFieldOrDialog(area)).toBe(true)
    expect(keyboardBelongsToFieldOrDialog(editable)).toBe(true)
    expect(keyboardBelongsToFieldOrDialog(document.body)).toBe(false)
    expect(keyboardBelongsToFieldOrDialog(null)).toBe(false)
  })

  it('a role=dialog without aria-modal owns the keyboard; aria-modal=false is exempt', () => {
    const sheet = document.createElement('div')
    sheet.setAttribute('role', 'dialog')
    document.body.appendChild(sheet)
    try {
      expect(keyboardBelongsToFieldOrDialog(document.body)).toBe(true)
      sheet.setAttribute('aria-modal', 'true')
      expect(keyboardBelongsToFieldOrDialog(document.body)).toBe(true)
      sheet.setAttribute('aria-modal', 'false')
      expect(keyboardBelongsToFieldOrDialog(document.body)).toBe(false)
    } finally {
      sheet.remove()
    }
  })

  it('typing "box 2" in the Save to Library name field never reaches the viewport capture', () => {
    const routedToTool: string[] = []
    // Stand-in for the viewport's window keydown handler: it routes a key to
    // the tool only when the predicate says the viewport owns the keyboard.
    const onWindowKeyDown = (ev: KeyboardEvent): void => {
      if (!keyboardBelongsToFieldOrDialog(ev.target)) routedToTool.push(ev.key)
    }
    window.addEventListener('keydown', onWindowKeyDown)
    try {
      render(<SaveToLibraryPopover defaultName="" onSave={vi.fn()} onClose={vi.fn()} />)
      const input = screen.getByLabelText('Item name')
      expect(input).toHaveFocus()
      let value = ''
      for (const key of 'box 2') {
        fireEvent.keyDown(input, { key })
        value += key
        fireEvent.change(input, { target: { value } })
      }
      expect(input).toHaveValue('box 2')
      expect(routedToTool).toEqual([])
      // Focus lost to the body while the modal is still up: still not the tool's.
      ;(document.activeElement as HTMLElement).blur()
      fireEvent.keyDown(document.body, { key: '2' })
      expect(routedToTool).toEqual([])
    } finally {
      window.removeEventListener('keydown', onWindowKeyDown)
    }
  })
})

describe('cursorFor honors Settings ▸ View ▸ Show tool pointer', () => {
  it('gives the tool-shaped cursor by default and the plain arrow when off', () => {
    expect(cursorFor('Move')).toContain('data:image/svg+xml')
    setShowToolPointer(false)
    try {
      expect(cursorFor('Move')).toBe('default')
    } finally {
      setShowToolPointer(true)
    }
  })
})

describe('createIsolationController', () => {
  /** Two world objects, nothing else. */
  function twoObjects(): IsolationScene {
    return {
      object_ids: () => BigUint64Array.from([1n, 2n]),
      instance_ids: () => new BigUint64Array(),
      group_ids: () => new BigUint64Array(),
      group_members: () => [],
      node_parent: () => undefined,
      instance_expanded_members: () => new BigUint64Array(),
    }
  }

  function controller() {
    const onTargetsChange = vi.fn()
    const onCountChange = vi.fn()
    const applyHidden = vi.fn()
    const frame = vi.fn()
    const ctl = createIsolationController({
      scene: twoObjects(),
      baseHidden: () => ({ objects: [], instances: [] }),
      applyHidden,
      frame,
      onCountChange,
      onTargetsChange,
    })
    return { ctl, onTargetsChange, onCountChange, applyHidden, frame }
  }

  it('reports the targets at every change: isolating, then clearing, is two calls', () => {
    const { ctl, onTargetsChange, onCountChange, frame } = controller()
    expect(ctl.isolate([{ kind: 'object', id: 1n }])).toBe(1)
    ctl.clear()
    expect(onTargetsChange).toHaveBeenCalledTimes(2)
    expect(onTargetsChange).toHaveBeenNthCalledWith(1, [{ kind: 'object', id: 1n }])
    expect(onTargetsChange).toHaveBeenNthCalledWith(2, null)
    expect(onCountChange.mock.calls).toEqual([[1], [null]])
    expect(frame).toHaveBeenCalledTimes(1)
    expect(ctl.state()).toBeNull()
    // Clearing again changes nothing, so reports nothing.
    ctl.clear()
    expect(onTargetsChange).toHaveBeenCalledTimes(2)
  })

  it('reports a re-resolution that drops every target, and an isolation of nothing', () => {
    const { ctl, onTargetsChange } = controller()
    // An id that does not exist resolves to no isolation at all.
    expect(ctl.isolate([{ kind: 'object', id: 9n }])).toBe(0)
    expect(onTargetsChange).not.toHaveBeenCalled()
    ctl.isolate([{ kind: 'object', id: 2n }], { frame: false })
    ctl.rebase()
    expect(onTargetsChange).toHaveBeenCalledTimes(2)
    expect(onTargetsChange).toHaveBeenLastCalledWith([{ kind: 'object', id: 2n }])
  })
})
