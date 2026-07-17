/**
 * MoveTool logic tests: the durable Alt copy toggle and the ×N / /N array
 * refinement, driven through the tool's public event surface against a
 * mocked WasmScene (no three.js meshes — objectsGroup stays null, matching
 * RotateTool.test.ts's approach).
 */

import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { MoveTool } from './MoveTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

/** A ray straight down through world (x, y) — MoveTool ignores it. */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(x = 0, y = 0, z = 0): Snap {
  return { x, y, z, kind: 'ground' }
}

/** Minimal KeyboardEvent-shaped fake — onKey reads .key/.repeat/.preventDefault. */
function makeKeyEvent(key: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  return {
    key,
    repeat: opts.repeat ?? false,
    preventDefault: () => { /* no-op */ },
  } as unknown as KeyboardEvent
}

/** Type a string one key at a time, then Enter. */
function typeKeys(tool: MoveTool, text: string): void {
  for (const ch of text) tool.onKey(makeKeyEvent(ch))
}

/**
 * Minimal WasmScene stub — only the members MoveTool calls. The stub hands
 * out fresh object handles per clone and models the kernel's two identity
 * tokens separately: `hash` (content) and `gen` (history generation — bumps
 * on every commit/undo/redo, never on view-state edits). Tests mutate them
 * independently to simulate the two classes of external change.
 */
function makeWasmScene() {
  let nextId = 100n
  const state = { hash: 1n, gen: 1n }
  const scene = {
    duplicate_selection_array: vi.fn(
      (_kinds: Uint8Array, ids: BigUint64Array, _affine: Float64Array, count: number) => {
        const out: { kind: string; id: bigint }[] = []
        for (let k = 0; k < count; k++) {
          for (let i = 0; i < ids.length; i++) {
            out.push({ kind: 'object', id: nextId++ })
          }
        }
        // Every commit moves both tokens, as the kernel's would.
        state.hash++
        state.gen++
        return out
      },
    ),
    transform_selection: vi.fn(() => { state.hash++; state.gen++ }),
    state_hash: vi.fn(() => state.hash),
    history_generation: vi.fn(() => state.gen),
    max_array_count: vi.fn(() => 1000),
    scene_undo: vi.fn(() => { state.hash++; state.gen++; return { free: () => { /* no-op */ } } }),
    scene_redo: vi.fn(() => { state.hash++; state.gen++; return { free: () => { /* no-op */ } } }),
  }
  return { scene, state }
}

function makeTool(selection?: NodeRef[]) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onArrayCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const onCopyModeChange = vi.fn()
  const { scene, state } = makeWasmScene()
  const tool = new MoveTool(
    scene as never,
    preview,
    null, // objectsGroup — no ghost mesh in logic tests
    selection ?? [{ kind: 'object', id: 1n }],
    onCommit,
    onToast,
    onMeasurement,
    null,
    onCopyModeChange,
    onArrayCommit,
  )
  return { tool, scene, state, onCommit, onArrayCommit, onToast, onMeasurement, onCopyModeChange }
}

/** Start a gesture at the origin and lock the X axis. */
function beginGestureLockedX(tool: MoveTool): void {
  tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
  tool.onKey(makeKeyEvent('ArrowRight'))
}

/** The translation column [tx, ty, tz] of a row-major 3×4 affine. */
function translationOf(affine: Float64Array): [number, number, number] {
  return [affine[3], affine[7], affine[11]]
}

describe('MoveTool — durable Alt copy toggle', () => {
  it('tapping Alt toggles copy mode on and off (not hold-to-copy)', () => {
    const { tool, onCopyModeChange } = makeTool()
    expect(tool.statusHint()).toContain('start the move')

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith(true)
    expect(tool.statusHint()).toContain('Copy is on')

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith(false)
    expect(tool.statusHint()).toContain('start the move')
  })

  it('ignores Alt autorepeat (a held Alt toggles exactly once)', () => {
    const { tool, onCopyModeChange } = makeTool()
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt', { repeat: true }))
    tool.onKey(makeKeyEvent('Alt', { repeat: true }))
    expect(onCopyModeChange).toHaveBeenCalledTimes(1)
    expect(onCopyModeChange).toHaveBeenLastCalledWith(true)
  })

  it('typed exact distance commits a COPY while toggled on — Alt long released', () => {
    const { tool, scene, onCommit } = makeTool()
    tool.onKey(makeKeyEvent('Alt')) // tap, release — durable
    beginGestureLockedX(tool)
    typeKeys(tool, '2')
    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    const [kinds, ids, affine, count] = scene.duplicate_selection_array.mock.calls[0]
    expect(Array.from(kinds as Uint8Array)).toEqual([0])
    expect(Array.from(ids as BigUint64Array)).toEqual([1n])
    expect(translationOf(affine as Float64Array)).toEqual([2, 0, 0])
    expect(count).toBe(1)
    expect(scene.transform_selection).not.toHaveBeenCalled()
    // The fresh clone becomes the committed selection.
    expect(onCommit).toHaveBeenCalledWith([{ kind: 'object', id: 100n }])
  })

  it('typed exact distance commits a plain MOVE after toggling back off', () => {
    const { tool, scene } = makeTool()
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt')) // back off
    beginGestureLockedX(tool)
    typeKeys(tool, '2')
    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.transform_selection).toHaveBeenCalledTimes(1)
    expect(scene.duplicate_selection_array).not.toHaveBeenCalled()
  })

  it('prefixes the readout with "Copy ·" while toggled on', () => {
    const { tool, onMeasurement } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('Alt'))
    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last.startsWith('Copy ·')).toBe(true)
  })
})

describe('MoveTool — ×N / /N array copy', () => {
  /** Tap Alt, move 1 selected object 2 m along X via the VCB — the copy
   * commit that arms the array refinement. */
  function commitOneCopy(t: ReturnType<typeof makeTool>): void {
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
  }

  it('teaches the refinement in the status hint after a copy commits (SketchUp 3x form first)', () => {
    const t = makeTool()
    commitOneCopy(t)
    expect(t.tool.statusHint()).toContain('3x')
    expect(t.tool.statusHint()).toContain('3/')
  })

  it('the SketchUp trailing form 3x + Enter resolves exactly like x3', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, '3x')
    // The trailing form's leading digit is buffer input, and it reads back
    // with the display glyph: "3×".
    expect(t.onMeasurement).toHaveBeenLastCalledWith('3×')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const [, ids, affine, count] = t.scene.duplicate_selection_array.mock.calls[1]
    expect(Array.from(ids as BigUint64Array)).toEqual([1n])
    expect(translationOf(affine as Float64Array)).toEqual([2, 0, 0])
    expect(count).toBe(3)
  })

  it('the trailing divide form 4/ + Enter divides the committed distance', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, '4/')
    t.tool.onKey(makeKeyEvent('Enter'))
    const call = t.scene.duplicate_selection_array.mock.calls.at(-1)!
    expect(translationOf(call[2] as Float64Array)).toEqual([0.5, 0, 0])
    expect(call[3]).toBe(4)
  })

  it('x3 + Enter re-resolves into 3 copies at the SAME spacing (one undo retracts the single copy first)', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, 'x3')
    expect(t.tool.capturingInput()).toBe(true) // digits must not switch tools
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    const [kinds, ids, affine, count] = t.scene.duplicate_selection_array.mock.calls[1]
    expect(Array.from(kinds as Uint8Array)).toEqual([0])
    expect(Array.from(ids as BigUint64Array)).toEqual([1n]) // the ORIGINAL source
    expect(translationOf(affine as Float64Array)).toEqual([2, 0, 0])
    expect(count).toBe(3)
    // All three clones become the selection via the full-refresh commit path.
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
    expect((t.onArrayCommit.mock.calls[0][0] as NodeRef[]).length).toBe(3)
  })

  it('*N works like xN; /N divides the committed distance', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, '*4')
    t.tool.onKey(makeKeyEvent('Enter'))
    let call = t.scene.duplicate_selection_array.mock.calls.at(-1)!
    expect(translationOf(call[2] as Float64Array)).toEqual([2, 0, 0])
    expect(call[3]).toBe(4)

    // Still hot — refine again, this time dividing: step = 2 m / 4.
    typeKeys(t.tool, '/4')
    t.tool.onKey(makeKeyEvent('Enter'))
    call = t.scene.duplicate_selection_array.mock.calls.at(-1)!
    expect(translationOf(call[2] as Float64Array)).toEqual([0.5, 0, 0])
    expect(call[3]).toBe(4)
    // Each refinement retracted the previous commit with ONE undo.
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(2)
  })

  it('refuses the refinement when the HISTORY moved even though the content hash did not (net-zero edit pair)', () => {
    // The adversarial-review reproduction: a tag added then removed leaves
    // state_hash identical while pushing two real undo actions. A hash
    // guard passes here — and its undo would silently eat the tag edit,
    // then stack a second array on the still-committed first.
    const t = makeTool()
    commitOneCopy(t)

    t.state.gen += 2n // two pushed actions, content restored (hash untouched)
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    // Closing the window is announced, not a silent no-op Enter.
    expect(t.onToast).toHaveBeenCalledWith(
      expect.stringContaining('the model changed'),
    )
    // The window is over: further array input is inert.
    typeKeys(t.tool, 'x3')
    expect(t.tool.capturingInput()).toBe(false)
  })

  it('survives view-state changes that move the hash but not the history (hide/eye toggles)', () => {
    // The inverse failure: set_tag_hidden / set_node_user_hidden change the
    // content hash but are deliberately not undoable — the undo stack (and
    // generation) are untouched, so an innocent declutter-hide must NOT
    // kill the refinement window.
    const t = makeTool()
    commitOneCopy(t)

    t.state.hash += 100n // view-state toggle: hash moves, generation doesn't
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const call = t.scene.duplicate_selection_array.mock.calls.at(-1)!
    expect(call[3]).toBe(3)
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
  })

  it('the armed window captures input, so Delete/Backspace cannot destroy the copies (App defers to capturingInput)', () => {
    const t = makeTool()
    commitOneCopy(t)

    // Armed but nothing typed yet — the status bar is inviting "Type ×3…";
    // App.tsx's Delete/Backspace handler checks exactly this flag before
    // firing edit-delete on the selection (which IS the just-made copies).
    expect(t.tool.capturingInput()).toBe(true)

    // Backspace routes to the tool and edits the (empty) buffer harmlessly.
    t.tool.onKey(makeKeyEvent('Backspace'))
    expect(t.tool.capturingInput()).toBe(true)

    // Esc remains the way out of the window.
    t.tool.onKey(makeKeyEvent('Escape'))
    expect(t.tool.capturingInput()).toBe(false)
  })

  it('the armed window captures only its buffer keys — Space and letters fall through (per-key capture)', () => {
    const t = makeTool()
    commitOneCopy(t)

    // Armed: the buffer needs digits, mode tokens, Backspace, Enter — plus
    // the bare Delete keystroke guard over the just-made copies.
    for (const key of ['0', '9', 'x', 'X', '*', '/', 'Backspace', 'Delete', 'Enter']) {
      expect(t.tool.capturesKey(key), `armed must capture ${JSON.stringify(key)}`).toBe(true)
    }
    // Space must NEVER be captured (it always resets to Select — the
    // Viewport's fall-through does the switch and the switch cancels the
    // tool, quietly ending the window). Tab and letter shortcuts fall
    // through to their global meanings too.
    for (const key of [' ', 'Tab', 'm', 'q', 'r', 'Escape']) {
      expect(t.tool.capturesKey(key), `armed must not capture ${JSON.stringify(key)}`).toBe(false)
    }
  })

  it('a mid-gesture VCB still captures the whole keyboard (Space is length grammar)', () => {
    const t = makeTool()
    beginGestureLockedX(t.tool)
    // "5' 3" needs the space; unit suffixes need letters.
    for (const key of [' ', '5', 'm', 'c', 'Backspace', 'Enter']) {
      expect(t.tool.capturesKey(key)).toBe(true)
    }
  })

  it('Space exit is quiet: the tool-switch cancel disarms without undoing the copies', () => {
    const t = makeTool()
    commitOneCopy(t)
    typeKeys(t.tool, '5') // even with a partial buffer typed
    expect(t.tool.capturesKey(' ')).toBe(false)

    // What the Viewport does on the fall-through: switch tools, which
    // cancels the outgoing MoveTool.
    t.tool.cancel()

    expect(t.tool.capturingInput()).toBe(false)
    // The committed copy is untouched — no retraction, no toast.
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.onToast).not.toHaveBeenCalled()
    // A stray Enter afterwards is inert.
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
  })

  it('garbage input resolves to nothing and a new gesture ends the window', () => {
    const t = makeTool()
    commitOneCopy(t)

    // A bare mode token is not a valid spec.
    typeKeys(t.tool, 'x')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).not.toHaveBeenCalled()

    // Starting another gesture closes the refinement window entirely.
    t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Escape')) // cancel the gesture
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
  })

  it('a refused refinement restores the retracted copies with redo and keeps the window hot', () => {
    const t = makeTool()
    commitOneCopy(t)

    t.scene.duplicate_selection_array.mockImplementationOnce(() => {
      throw new Error('Transform: refused')
    })
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.scene_redo).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalled()

    // The recovery undo+redo moved the history generation; the window
    // re-stamped its token, so a fresh count still resolves.
    typeKeys(t.tool, 'x2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(2)
    const call = t.scene.duplicate_selection_array.mock.calls.at(-1)!
    expect(call[3]).toBe(2)
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
  })

  it('disarmArray (explicit delete/undo/redo commands) closes the window cleanly: capture released, later Enter quietly inert', () => {
    const t = makeTool()
    commitOneCopy(t)
    t.onToast.mockClear()
    expect(t.tool.capturingInput()).toBe(true)

    // The Viewport calls this from runDelete AND runUndo/runRedo — every
    // explicit document command ends the window before executing, so the
    // keyboard capture releases and tool shortcuts route normally again.
    t.tool.disarmArray()

    // Window gone: the keyboard guard releases (Delete works again) ...
    expect(t.tool.capturingInput()).toBe(false)
    // ... and a later x3 + Enter does nothing — no wrong-action undo, no
    // second array, and no toast spam (the user asked for the delete).
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('refuses a count above the kernel cap with a toast, before any undo fires', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, 'x1001')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).toHaveBeenCalledWith(expect.stringContaining('1000'))
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    // The cap comes from the scene — the single source of truth.
    expect(t.scene.max_array_count).toHaveBeenCalled()
  })
})

// ---- select-and-transform acquirer coverage (select-ux branch) ----

/** A ray straight down (−Z) through world (x, y). */
function rayThroughSel(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnapSel(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** Minimal WasmScene stub — only the members MoveTool calls in these paths. */
function makeWasmSceneSel() {
  return {
    transform_selection: vi.fn(),
  }
}

function makeToolSel(selection: NodeRef[] = []) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const wasmScene = makeWasmSceneSel()
  const tool = new MoveTool(
    wasmScene as never,
    preview,
    null, // objectsGroup — null means no ghost mesh is cloned (fine for logic tests)
    selection,
    onCommit,
    onToast,
    onMeasurement,
    null,
  )
  return { tool, preview, onCommit, onToast, onMeasurement, wasmScene }
}

describe('MoveTool — auto-select on click', () => {
  // Deliberate contract change (selection-UX overhaul): moving an object no
  // longer requires a two-step Select-then-Move — an empty-selection click
  // acquires the node under the cursor and starts the move on it.
  it('empty selection: the first click acquires the node under the cursor and sets the base point', () => {
    const { tool, onToast } = makeToolSel([])
    const acquire = vi.fn(() => [{ kind: 'object', id: 7n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnapSel({ x: 1, y: 1, z: 0 }), rayThroughSel(1, 1))

    expect(acquire).toHaveBeenCalledTimes(1)
    expect(acquire).toHaveBeenCalledWith(rayThroughSel(1, 1))
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true) // in 'base' stage — the move began
  })

  it('the acquired node is what the second click commits (one fluid select-and-move)', () => {
    const { tool, wasmScene, onCommit } = makeToolSel([])
    tool.setSelectionAcquirer(() => [{ kind: 'object', id: 7n }])

    tool.onPointerDown(makeSnapSel({ x: 0, y: 0, z: 0 }), rayThroughSel(0, 0)) // base (auto-select)
    tool.onPointerMove(makeSnapSel({ x: 2, y: 0, z: 0 }), rayThroughSel(2, 0))
    tool.onPointerDown(makeSnapSel({ x: 2, y: 0, z: 0 }), rayThroughSel(2, 0)) // destination

    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    const [kinds, ids, , affine] = (wasmScene.transform_selection as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(Array.from(kinds as Uint8Array)).toEqual([0]) // one object
    expect(Array.from(ids as BigUint64Array)).toEqual([7n])
    expect((affine as Float64Array)[3]).toBeCloseTo(2) // tx = 2
    expect(onCommit).toHaveBeenCalledWith([{ kind: 'object', id: 7n }])
    expect(tool.capturingInput()).toBe(false) // reset to idle after commit
  })

  it('a genuine miss (acquirer returns null) toasts and stays idle', () => {
    const { tool, onToast } = makeToolSel([])
    tool.setSelectionAcquirer(() => null)
    tool.onPointerDown(makeSnapSel(), rayThroughSel(0, 0))
    expect(onToast).toHaveBeenCalledWith('Click an object to move it')
    expect(tool.capturingInput()).toBe(false)
  })

  it('an existing selection is never re-acquired — the click is the base point as before', () => {
    const { tool, onToast } = makeToolSel([{ kind: 'object', id: 3n }])
    const acquire = vi.fn(() => [{ kind: 'object', id: 7n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnapSel({ x: 1, y: 1, z: 0 }), rayThroughSel(1, 1))
    expect(acquire).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)
  })
})

describe('MoveTool — live selection sync (setSelection)', () => {
  it('setSelection replaces the cached targets so the next gesture commits against live handles', () => {
    // The maintainer's repro tail: after Undo killed the two array copies,
    // MoveTool's cached targets still pointed at the dead clones and the
    // next 8cm copy failed with UnknownObject. The Viewport pushes every
    // app-selection change (undo pruning included) into the active tool.
    const t = makeTool([
      { kind: 'object', id: 101n },
      { kind: 'object', id: 102n },
      { kind: 'object', id: 103n },
    ])
    t.tool.setSelection([{ kind: 'object', id: 1n }])

    t.tool.onKey(makeKeyEvent('Alt')) // copy mode
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    const [, ids] = t.scene.duplicate_selection_array.mock.calls[0]
    expect(Array.from(ids as BigUint64Array)).toEqual([1n])
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('an emptied selection falls back to the auto-acquire path instead of a dead-handle commit', () => {
    const t = makeTool([{ kind: 'object', id: 101n }])
    t.tool.setSelection([])
    t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    // No acquirer injected: the tool hints and stays idle — no kernel call.
    expect(t.onToast).toHaveBeenCalledWith('Click an object to move it')
    expect(t.tool.capturingInput()).toBe(false)
  })
})
