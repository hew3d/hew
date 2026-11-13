/**
 * MoveTool logic tests: the durable copy toggle (Option on macOS, Ctrl on
 * Windows/Linux — `platform.ts`'s `COPY_MODIFIER_KEY`) and the ×N / /N array
 * refinement, driven through the tool's public event surface against a
 * mocked WasmScene (no three.js meshes — objectsGroup stays null, matching
 * RotateTool.test.ts's approach).
 *
 * `isMac` is a load-time constant read from `navigator.platform`, so this
 * whole file mocks `../platform` to `isMac: true` (macOS) rather than relying
 * on the CI host's actual OS — every test below except the "Windows/Linux"
 * describe near the copy-toggle tests assumes that mock. That one test needs
 * the OTHER platform instead, so it overrides the mock with a scoped
 * `vi.doMock` + `vi.resetModules()` + dynamic re-import (same pattern
 * `trayLayout.test.ts` uses for its module-load-time state) — the
 * statically-imported `MoveTool` above is a different module instance and is
 * unaffected by that reset.
 */

import { describe, it, expect, vi, beforeAll, afterAll, beforeEach } from 'vitest'
import * as THREE from 'three'
import { setLengthUnit, getLengthUnit, setPrecision, DEFAULT_PRECISION } from '../settings/units'
import { MoveTool } from './MoveTool'
import { setCopiesLinked } from '../settings/editing'
import { CleanModifierTap } from '../viewport/cleanModifierTap'
import { DragCommitGate } from '../viewport/dragCommit'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { ctxGround, ctxOfPlane, draftRef } from './planeUnits'
import { applyAffineToPoint } from './transformMath'
import { SiblingPreviewMirror } from '../viewport/siblingPreview'
import { isSessionGhost } from './transformPreview'

// The tool tests below were written when every copy was independent; with
// "Copies are linked" off the primary copy is that same independent copy.
// The linked-copy tests turn the preference back on for themselves.
beforeEach(() => setCopiesLinked(false))

const CTX = ctxGround()
/** Kernel keys are 16-hex strings. */
const hex = (n: bigint): string => n.toString(16).padStart(16, '0')
/** A `draft-island` ref of the ground plane's drawing. */
const di = (key: bigint): NodeRef => draftRef('draft-island', CTX, hex(key))

type Seg = { key: bigint; a: number[]; b: number[] }
type Unit = { key: bigint; edges: Seg[] }

vi.mock('../platform', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../platform')>()
  return { ...actual, isMac: true, COPY_MODIFIER_KEY: 'Alt', COPY_MODIFIER_LABEL: 'Option' }
})

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
/** World-identity drawing axes (tool-parity §4) — the default frame every
 *  test exercises unless it explicitly overrides `frame`. */
const WORLD_FRAME_FLAT = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]

function makeWasmScene(frame: number[] = WORLD_FRAME_FLAT) {
  let nextId = 100n
  let nextEdge = 500n
  const state = { hash: 1n, gen: 1n }
  // --- drawing surface (drafting engine). The ground plane's listing holds
  // two single-edge units: 40 (edge 10) and 41 (edge 20). A draft's
  // `draft_transform_edges` moves the listed edges; `draft_copy_edges` lands
  // a fresh unit 77 holding the copies. Mutate `drawing.units` (and bump
  // `state.gen`) to simulate an arranging landing or an undo.
  const drawing: { units: Unit[] } = {
    units: [
      { key: 40n, edges: [{ key: 10n, a: [0, 0, 0], b: [1, 0, 0] }] },
      { key: 41n, edges: [{ key: 20n, a: [5, 5, 0], b: [6, 5, 0] }] },
    ],
  }
  const undoCalls = (): number => scene.scene_undo.mock.calls.length
  // History bracket: open snapshots the identity tokens and the drawing;
  // abort restores them exactly (the kernel restores the whole document,
  // history included); commit folds everything recorded since open into ONE
  // entry (the generation moves by one).
  const brackets = { open: null as null | { gen: bigint; hash: bigint; units: Unit[] }, labels: [] as string[], aborted: 0 }
  const scene = {
    history_bracket_open: vi.fn(() => {
      if (brackets.open !== null) throw new Error('StaleReference: a history bracket is already open')
      brackets.open = { gen: state.gen, hash: state.hash, units: structuredClone(drawing.units) }
    }),
    history_bracket_commit: vi.fn((label: string) => {
      const b = brackets.open
      brackets.open = null
      if (b === null) throw new Error('StaleReference: no history bracket')
      if (state.gen === b.gen) return false
      state.gen = b.gen + 1n
      state.hash += 1n
      brackets.labels.push(label)
      return true
    }),
    history_bracket_abort: vi.fn(() => {
      const b = brackets.open
      brackets.open = null
      if (b === null) return
      brackets.aborted += 1
      state.gen = b.gen
      state.hash = b.hash
      drawing.units = b.units
    }),
    axes: vi.fn(() => new Float64Array(frame)),
    plane_units_json: vi.fn((_ctx: string) =>
      JSON.stringify({
        units: drawing.units.map((u) => ({
          key: hex(u.key),
          edges: u.edges.map((e) => ({
            key: hex(e.key), a: e.a, b: e.b, store: { kind: 'free', sketch: 3 }, curve: null,
          })),
        })),
        regions: [],
      }),
    ),
    draft_begin_ctx: vi.fn((_ctx: string, _label: string) => 1n),
    draft_transform_edges: vi.fn((_d: bigint, keys: BigUint64Array, affine: Float64Array) => {
      const set = new Set([...keys].map(hex))
      for (const u of drawing.units) {
        for (const e of u.edges) {
          if (!set.has(hex(e.key))) continue
          e.a = applyAffineToPoint(affine, e.a as [number, number, number])
          e.b = applyAffineToPoint(affine, e.b as [number, number, number])
        }
      }
    }),
    draft_copy_edges: vi.fn((_d: bigint, keys: BigUint64Array, affine: Float64Array) => {
      const set = new Set([...keys].map(hex))
      const edges: Seg[] = []
      for (const u of drawing.units) {
        for (const e of u.edges) {
          if (!set.has(hex(e.key))) continue
          edges.push({
            key: nextEdge++,
            a: applyAffineToPoint(affine, e.a as [number, number, number]),
            b: applyAffineToPoint(affine, e.b as [number, number, number]),
          })
        }
      }
      drawing.units.push({ key: 77n, edges })
    }),
    // Ending a draft is a commit: both identity tokens move, as the kernel's would.
    draft_end: vi.fn((_d: bigint) => {
      state.hash++
      state.gen++
      return '{"objects":[],"sketches":[],"kept_as_sketch":[]}'
    }),
    draft_cancel: vi.fn(),
    // --- sketch replay surface (whole-sketch Move+Alt copies). One sketch with
    // two single-edge islands: 40 (edge 10) and 41 (edge 20); replayed
    // edges get fresh ids >= 500 and land in island 77.
    sketch_island_ids: vi.fn(() => [40n, 41n]),
    sketch_island_edges: vi.fn((_s: bigint, island: bigint) =>
      island === 40n ? [10n] : island === 41n ? [20n] : [],
    ),
    sketch_edge_island: vi.fn((_s: bigint, edge: bigint) =>
      edge >= 500n ? 77n : edge === 10n ? 40n : 41n,
    ),
    sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) =>
      e === 10n ? [0, 0, 0, 1, 0, 0] : e === 20n ? [5, 5, 0, 6, 5, 0] : undefined,
    ),
    sketch_edge_curve: vi.fn(() => undefined),
    sketch_curve_geom: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => [0, 0, 0, 0, 0, 1]),
    sketch_island_lines: vi.fn(() => new Float32Array(0)),
    sketch_lines: vi.fn(() => new Float32Array(0)),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(() => { state.hash++; state.gen++ }),
    sketch_cancel_gesture: vi.fn(),
    sketch_begin_curve: vi.fn(() => 8n),
    sketch_begin_curve_with: vi.fn(() => 8n),
    sketch_end_curve: vi.fn(),
    sketch_add_segment: vi.fn(() => {
      const id = nextEdge++
      return { new_edges: () => [id], free: () => { /* no-op */ } }
    }),
    transform_sketch: vi.fn(() => { state.hash++; state.gen++ }),
    transform_sketch_island: vi.fn(() => { state.hash++; state.gen++ }),
    transform_sketch_islands: vi.fn(() => {
      state.hash++
      state.gen++
      return '{"objects":[],"sketches":[],"kept_as_sketch":[],"faces_closed":[],"cells":[]}'
    }),
    duplicate_selection_array: vi.fn(
      (_kinds: Uint8Array, ids: BigUint64Array, _affine: Float64Array, count: number, _linked?: boolean) => {
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
    // A linked copy: every loose source is wrapped as its own set, and each
    // copy is a placement (an instance) of it.
    duplicate_selection_array_linked: vi.fn(
      (kinds: Uint8Array, ids: BigUint64Array, _affine: Float64Array, count: number) => {
        const nodes: { kind: string; id: string }[] = []
        for (let k = 0; k < count; k++) {
          for (let i = 0; i < ids.length; i++) nodes.push({ kind: 'instance', id: String(nextId++) })
        }
        const linked = Array.from(ids).map((id, i) => ({
          source: { kind: kinds[i] === 1 ? 'group' : kinds[i] === 2 ? 'instance' : 'object', id: String(id) },
          instance: String(nextId++),
          definition: String(nextId++),
        })).filter((w) => w.source.kind !== 'instance')
        state.gen++
        return JSON.stringify({ nodes, linked })
      },
    ),
    transform_selection: vi.fn(
      (_kinds: Uint8Array, _ids: BigUint64Array, _sketches: BigUint64Array, _affine: Float64Array) => {
        state.hash++
        state.gen++
      },
    ),
    state_hash: vi.fn(() => state.hash),
    history_generation: vi.fn(() => state.gen),
    // The undo stack's size: records minus undos plus redos = gen - 2 * undos.
    undo_depth: vi.fn(() => Number(state.gen) - 1 - 2 * undoCalls()),
    max_array_count: vi.fn(() => 1000),
    scene_undo: vi.fn(() => { state.hash++; state.gen++; return { free: () => { /* no-op */ } } }),
    scene_redo: vi.fn(() => { state.hash++; state.gen++; return { free: () => { /* no-op */ } } }),
  }
  return { scene, state, drawing, brackets }
}

function makeTool(selection?: NodeRef[], frame?: number[]) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onArrayCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const onCopyModeChange = vi.fn()
  const { scene, state, drawing, brackets } = makeWasmScene(frame)
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
  return { tool, scene, state, drawing, brackets, onCommit, onArrayCommit, onToast, onMeasurement, onCopyModeChange }
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

describe('MoveTool — movable drawing axes (arrow-key lock)', () => {
  it('ArrowRight locks to the CURRENT frame\'s red axis, not literal world X, under a moved frame', () => {
    // Frame with red/green swapped relative to world: x=[0,1,0], y=[-1,0,0],
    // z=[0,0,1] — an orthonormal, right-handed frame (tool-parity §4).
    const frame = [0, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1]
    const { tool, scene } = makeTool(undefined, frame)
    beginGestureLockedX(tool)
    typeKeys(tool, '2')
    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.transform_selection).toHaveBeenCalledTimes(1)
    const [, , , affine] = scene.transform_selection.mock.calls[0]
    // 2 units along the frame's red axis [0,1,0] — NOT world X [1,0,0].
    expect(translationOf(affine as Float64Array)).toEqual([0, 2, 0])
  })
})

describe('MoveTool — typed distance before the cursor has a direction', () => {
  it('moves nothing and says to move the cursor or lock an axis', () => {
    const { tool, scene, onToast } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0)) // base, no move, no lock
    typeKeys(tool, '2')
    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.transform_selection).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledWith(expect.stringContaining('lock an axis'))
  })
})

describe('MoveTool — locked typed-entry direction (signed by the cursor\'s side of the base)', () => {
  // The typed commit's direction under a lock is `sign((dest - base) ·
  // lockDir) * lockDir`, with the sign defaulting POSITIVE when dest sits
  // exactly on the base. These three pin the whole contract the inference
  // engine's locked resolve feeds: hovering the base resolves the base
  // EXACTLY (never a noise-signed station — the Move+Alt donut e2e's
  // wrong-way copy), so the zero-displacement default must be +lock; and a
  // genuine dest on either side must sign the typed distance toward it.

  /** Begin at the origin, lock Z, and (optionally) track a locked dest. */
  function lockZAt(tool: MoveTool, dest?: [number, number, number]): void {
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('ArrowUp'))
    if (dest !== undefined) {
      tool.onPointerMove(makeSnap(...dest), rayThrough(dest[0], dest[1]))
    }
  }

  it('dest exactly on the base (hovering the grab point): typed distance goes +lock', () => {
    const { tool, scene } = makeTool()
    lockZAt(tool) // no pointer move: dest === base, the on-anchor resolve
    typeKeys(tool, '0.5')
    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.transform_selection).toHaveBeenCalledTimes(1)
    const [, , , affine] = scene.transform_selection.mock.calls[0]
    expect(translationOf(affine as Float64Array)).toEqual([0, 0, 0.5])
  })

  it('dest on the NEGATIVE side (real geometry below the base): typed distance follows it down', () => {
    const { tool, scene } = makeTool()
    lockZAt(tool, [0, 0, -2]) // e.g. a hovered edge 2 m below, Z-locked
    typeKeys(tool, '0.5')
    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.transform_selection).toHaveBeenCalledTimes(1)
    const [, , , affine] = scene.transform_selection.mock.calls[0]
    expect(translationOf(affine as Float64Array)).toEqual([0, 0, -0.5])
  })

  it('dest on the POSITIVE side: typed distance follows it up', () => {
    const { tool, scene } = makeTool()
    lockZAt(tool, [0, 0, 2])
    typeKeys(tool, '0.5')
    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.transform_selection).toHaveBeenCalledTimes(1)
    const [, , , affine] = scene.transform_selection.mock.calls[0]
    expect(translationOf(affine as Float64Array)).toEqual([0, 0, 0.5])
  })
})

describe('MoveTool — durable Alt copy toggle', () => {
  it('tapping_the_modifier_cycles_move_copy_independent_copy', () => {
    // With "Copies are linked" on: Move, then Copy (linked), then
    // Independent copy, then Move again — a durable cycle, not hold-to-copy.
    setCopiesLinked(true)
    const { tool, onCopyModeChange } = makeTool()
    expect(tool.statusHint()).toContain('start the move')

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith('primary', true)
    expect(tool.statusHint()).toContain('Copy is on')

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith('alternate', false)
    expect(tool.statusHint()).toContain('Independent copy is on')

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith('off', false)
    expect(tool.statusHint()).toContain('start the move')
  })

  it('with_the_preference_off_the_default_copy_is_independent_and_the_alternate_is_linked', () => {
    setCopiesLinked(false)
    const { tool, onCopyModeChange } = makeTool()
    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith('primary', false)
    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith('alternate', true)
    expect(tool.statusHint()).toContain('Linked copy is on')
  })

  it('ignores Alt autorepeat (a held Alt toggles exactly once)', () => {
    const { tool, onCopyModeChange } = makeTool()
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt', { repeat: true }))
    tool.onKey(makeKeyEvent('Alt', { repeat: true }))
    expect(onCopyModeChange).toHaveBeenCalledTimes(1)
    expect(onCopyModeChange).toHaveBeenLastCalledWith('primary', false)
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
    tool.onKey(makeKeyEvent('Alt')) // the other kind of copy
    tool.onKey(makeKeyEvent('Alt')) // back to plain Move
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

describe('MoveTool — copy modifier is platform-specific (Option on macOS, Ctrl elsewhere)', () => {
  it('toggleCopyMode() flips copy mode directly — the method a Windows/Linux Control clean tap calls', () => {
    const { tool, onCopyModeChange } = makeTool()
    tool.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith('primary', false)
    expect(tool.statusHint()).toContain('Copy is on')
    tool.toggleCopyMode()
    tool.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith('off', false)
  })

  it('a bare Control keydown does NOT toggle copy mode on macOS — Option owns it here', () => {
    // Bare Control never reaches onKey via the real generic dispatch either
    // (Viewport gates it behind `!isMod`) — this pins that even a direct
    // call can't mistake Control for the mac copy modifier.
    const { tool, onCopyModeChange } = makeTool()
    tool.onKey(makeKeyEvent('Control'))
    expect(onCopyModeChange).not.toHaveBeenCalled()
  })

  it('a Windows/Linux Control clean tap toggles copy mode via toggleCopyMode, exactly as the Viewport calls it', () => {
    // Mirrors Viewport.tsx's onCtrlKeyUp branch: CleanModifierTap arms on a
    // bare Control keydown and fires toggleCopyMode on a clean keyup.
    const { tool, onCopyModeChange } = makeTool()
    const tap = new CleanModifierTap<{ toggleCopyMode(): void }>((key) => key === 'Control')

    tap.onKeyDown({ key: 'Control', repeat: false }, tool)
    const armed = tap.onKeyUp({ key: 'Control' }, tool)
    expect(armed).not.toBeNull()
    armed!.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith('primary', false)
  })

  it('a Ctrl+Z chord does NOT toggle copy mode (the clean-tap combo guard)', () => {
    const { tool, onCopyModeChange } = makeTool()
    const tap = new CleanModifierTap<{ toggleCopyMode(): void }>((key) => key === 'Control')

    tap.onKeyDown({ key: 'Control', repeat: false }, tool)
    tap.onKeyDown({ key: 'z', repeat: false }, tool) // Ctrl+Z joins the press
    const armed = tap.onKeyUp({ key: 'Control' }, tool)
    expect(armed).toBeNull()
    expect(onCopyModeChange).not.toHaveBeenCalled()
  })

  it('a Ctrl+C chord does NOT toggle copy mode either', () => {
    const { tool, onCopyModeChange } = makeTool()
    const tap = new CleanModifierTap<{ toggleCopyMode(): void }>((key) => key === 'Control')

    tap.onKeyDown({ key: 'Control', repeat: false }, tool)
    tap.onKeyDown({ key: 'c', repeat: false }, tool) // Ctrl+C joins the press
    expect(tap.onKeyUp({ key: 'Control' }, tool)).toBeNull()
    expect(onCopyModeChange).not.toHaveBeenCalled()
  })
})

describe('MoveTool — on Windows/Linux, Alt is NOT the copy modifier (Ctrl is)', () => {
  it('a bare Alt keydown does nothing when isMac is false — toggleCopyMode is still reachable directly', async () => {
    vi.resetModules()
    vi.doMock('../platform', () => ({
      isMac: false,
      isLinux: false,
      isWindows: true,
      modLabel: 'Ctrl+',
      COPY_MODIFIER_KEY: 'Control',
      COPY_MODIFIER_LABEL: 'Ctrl',
      isCoarsePointer: () => false,
      prefersReducedMotion: () => false,
    }))
    const { MoveTool: MoveToolOnWindows } = await import('./MoveTool')
    const { scene } = makeWasmScene()
    const onCopyModeChange = vi.fn()
    const tool = new MoveToolOnWindows(
      scene as never,
      new THREE.Group(),
      null,
      [{ kind: 'object', id: 1n }],
      vi.fn(),
      vi.fn(),
      vi.fn(),
      null,
      onCopyModeChange,
      vi.fn(),
    )

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).not.toHaveBeenCalled()
    expect(tool.statusHint()).not.toContain('Copy is on')

    // The Viewport's Control clean tap calls this directly on this platform.
    tool.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith('primary', false)
    expect(tool.statusHint()).toContain('Copy is on')
    expect(tool.statusHint()).toContain('Ctrl')
  })
})

describe('MoveTool — drawn-shape copy (playtest: "you can\'t Copy a Sketch")', () => {
  it('Alt copy of a drawn unit COPIES it through one draft at the offset instead of moving it', () => {
    const t = makeTool([di(40n)])
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))

    // One draft on the shape's plane, the unit's edge copied translated by +2 X.
    expect(t.scene.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(t.scene.draft_begin_ctx).toHaveBeenCalledWith(CTX, 'Copy')
    expect(t.scene.draft_copy_edges).toHaveBeenCalledTimes(1)
    const [, keys, affine] = t.scene.draft_copy_edges.mock.calls[0]
    expect([...keys]).toEqual([10n])
    expect(translationOf(affine as Float64Array)).toEqual([2, 0, 0])
    expect(t.scene.draft_end).toHaveBeenCalledTimes(1)
    expect(t.scene.sketch_begin_gesture).not.toHaveBeenCalled()
    // A COPY, not a move — and no object-duplicate call for a pure drawing
    // selection.
    expect(t.scene.draft_transform_edges).not.toHaveBeenCalled()
    expect(t.scene.transform_sketch_island).not.toHaveBeenCalled()
    expect(t.scene.transform_sketch_islands).not.toHaveBeenCalled()
    expect(t.scene.transform_sketch).not.toHaveBeenCalled()
    expect(t.scene.duplicate_selection_array).not.toHaveBeenCalled()
    // The new unit becomes the committed selection (found by geometry).
    expect(t.onCommit).toHaveBeenCalledWith([di(77n)])
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('Alt copy of a WHOLE sketch still REPLAYS it at the offset (one gesture bracket)', () => {
    const t = makeTool([{ kind: 'sketch', id: 3n }])
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.scene.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(t.scene.sketch_add_segment).toHaveBeenCalledWith(3n, 2, 0, 0, 3, 0, 0)
    expect(t.scene.sketch_add_segment).toHaveBeenCalledWith(3n, 7, 5, 0, 8, 5, 0)
    expect(t.scene.sketch_end_gesture).toHaveBeenCalledTimes(1)
    expect(t.scene.draft_begin_ctx).not.toHaveBeenCalled()
    expect(t.scene.transform_sketch).not.toHaveBeenCalled()
    expect(t.scene.duplicate_selection_array).not.toHaveBeenCalled()
  })

  it('a drawn-shape copy arms the ×N window: 3x retracts the copy and lays 3 in ONE draft', () => {
    const t = makeTool([di(40n)])
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.draft_end).toHaveBeenCalledTimes(1)

    typeKeys(t.tool, '3x')
    t.tool.onKey(makeKeyEvent('Enter'))
    // The single copy's one history entry is retracted, then ONE new draft
    // lays all three reps at 1x, 2x, 3x the step.
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.draft_begin_ctx).toHaveBeenCalledTimes(2)
    expect(t.scene.draft_end).toHaveBeenCalledTimes(2)
    const calls = t.scene.draft_copy_edges.mock.calls.slice(1)
    expect(calls).toHaveLength(3)
    expect(calls.map((c) => translationOf(c[2] as Float64Array))).toEqual([[2, 0, 0], [4, 0, 0], [6, 0, 0]])
    for (const c of calls) expect([...c[1]]).toEqual([10n])
    expect(t.scene.duplicate_selection_array).not.toHaveBeenCalled()
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('/3 on a drawn-shape copy divides the committed distance into 3 steps', () => {
    const t = makeTool([di(40n)])
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))

    typeKeys(t.tool, '/3')
    t.tool.onKey(makeKeyEvent('Enter'))
    const calls = t.scene.draft_copy_edges.mock.calls.slice(1)
    expect(calls.map((c) => translationOf(c[2] as Float64Array))).toEqual([[1, 0, 0], [2, 0, 0], [3, 0, 0]])
  })

  it('a mixed selection arrays objects AND drawn shapes together', () => {
    const t = makeTool([
      { kind: 'object', id: 1n },
      di(40n),
    ])
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.scene.draft_end).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenCalledWith([
      di(77n),
      { kind: 'object', id: 100n },
    ])
    // The drawn and object halves are ONE undo step (a history bracket).
    expect(t.scene.undo_depth()).toBe(1)
    expect(t.brackets.labels).toEqual(['Copy'])

    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))
    // One entry for the whole gesture (was one per half): one undo retracts it
    // and the array is reissued as one entry again.
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.undo_depth()).toBe(1)
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    expect(t.scene.duplicate_selection_array.mock.calls[1][3]).toBe(3)
    expect(t.scene.draft_copy_edges.mock.calls.slice(1)).toHaveLength(3)
  })
})

describe('MoveTool — linked copies', () => {
  function copyThenArray(t: ReturnType<typeof makeTool>, taps: number, spec: string): void {
    for (let i = 0; i < taps; i += 1) t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    if (spec !== '') {
      typeKeys(t.tool, spec)
      t.tool.onKey(makeKeyEvent('Enter'))
    }
  }

  it('a_linked_copy_passes_linked_true_to_duplicate_selection_array', () => {
    setCopiesLinked(true)
    const t = makeTool()
    copyThenArray(t, 1, '')
    // The linked call is the one that reports the wraps; the independent
    // call is never made.
    expect(t.scene.duplicate_selection_array_linked).toHaveBeenCalledTimes(1)
    expect(t.scene.duplicate_selection_array).not.toHaveBeenCalled()
    const [kinds, ids, , count] = t.scene.duplicate_selection_array_linked.mock.calls[0]
    expect(Array.from(kinds as Uint8Array)).toEqual([0])
    expect(Array.from(ids as BigUint64Array)).toEqual([1n])
    expect(count).toBe(1)
    // The copies are placements: that is what the host selects.
    expect(t.onCommit.mock.calls.at(-1)![0][0].kind).toBe('instance')
    // The host learns a loose node was wrapped (it needs a full refresh).
    expect(t.tool.takeLinkedWraps()).toHaveLength(1)
    expect(t.tool.takeLinkedWraps()).toHaveLength(0)
  })

  it('retyping_a_linked_copy_stays_linked_after_the_preference_flips_and_the_readout_says_so', () => {
    setCopiesLinked(true)
    const t = makeTool()
    copyThenArray(t, 1, '')
    expect(t.scene.duplicate_selection_array_linked).toHaveBeenCalledTimes(1)
    t.tool.takeLinkedWraps()
    // The preference flips while the retype window is open.
    setCopiesLinked(false)
    typeKeys(t.tool, '3')
    // The readout names the kind the redo will make, not what a new copy would be.
    expect(t.onMeasurement.mock.calls.at(-1)![0]).toMatch(/^Copy · /)
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.duplicate_selection_array_linked).toHaveBeenCalledTimes(2)
    expect(t.scene.duplicate_selection_array).not.toHaveBeenCalled()
  })

  it('a_redo_that_makes_no_wraps_does_not_report_the_previous_ones', () => {
    setCopiesLinked(true)
    const t = makeTool()
    // A linked copy of an instance wraps nothing; an earlier wrap must not linger.
    copyThenArray(t, 1, '')
    expect(t.tool.takeLinkedWraps()).toHaveLength(1)
    t.scene.duplicate_selection_array_linked.mockImplementationOnce(() =>
      JSON.stringify({ nodes: [{ kind: 'instance', id: '900' }], linked: [] }),
    )
    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.tool.takeLinkedWraps()).toHaveLength(0)
  })

  it('reports_its_copy_state_for_the_cursor_badge', () => {
    setCopiesLinked(true)
    const t = makeTool()
    expect(t.tool.currentCopyState()).toBe('off')
    t.tool.onKey(makeKeyEvent('Alt'))
    expect(t.tool.currentCopyState()).toBe('primary')
  })

  it('an_array_inherits_the_mode_of_the_copy_it_follows', () => {
    setCopiesLinked(true)
    const linked = makeTool()
    copyThenArray(linked, 1, 'x3')
    expect(linked.scene.duplicate_selection_array_linked).toHaveBeenCalledTimes(2)
    expect(linked.scene.duplicate_selection_array_linked.mock.calls[1][3]).toBe(3)
    expect(linked.scene.duplicate_selection_array).not.toHaveBeenCalled()

    // The alternate (independent) copy stays independent through its array.
    const independent = makeTool()
    copyThenArray(independent, 2, 'x3')
    expect(independent.scene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    expect(independent.scene.duplicate_selection_array.mock.calls[1][4]).toBe(false)
    expect(independent.scene.duplicate_selection_array_linked).not.toHaveBeenCalled()
  })

  it('x4_after_a_linked_copy_undoes_the_wrap_and_rewraps_to_one_set_of_five', () => {
    setCopiesLinked(true)
    const t = makeTool()
    copyThenArray(t, 1, 'x4')
    // One undo retracts the first copy together with its wrap (one history
    // entry), then the re-resolve copies the ORIGINAL loose source again.
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.duplicate_selection_array_linked).toHaveBeenCalledTimes(2)
    const [, ids, , count] = t.scene.duplicate_selection_array_linked.mock.calls[1]
    expect(Array.from(ids as BigUint64Array)).toEqual([1n])
    expect(count).toBe(4)
    // Four copies plus the wrapped original are the five placements.
    expect((t.onArrayCommit.mock.calls.at(-1)![0] as NodeRef[]).length).toBe(4)
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
    history_generation: vi.fn(() => 1n),
    // World drawing axes (the dashed base→destination line is colored against them).
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
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

describe('MoveTool — setEditContext aborts an armed gesture on a genuine change (component-edit-parity.md phase A2)', () => {
  it('a genuine context change cancels an armed drag instead of silently retargeting its eventual commit', () => {
    const t = makeTool()
    t.tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })
    beginGestureLockedX(t.tool)
    expect(t.tool.capturingInput()).toBe(true)

    t.tool.setEditContext({ kind: 'top' })

    expect(t.tool.capturingInput()).toBe(false)
  })

  it('re-pushing the SAME context is a no-op — an armed drag survives it untouched', () => {
    const t = makeTool()
    const ctx = { kind: 'instance' as const, id: 9n, component: 90n }
    t.tool.setEditContext(ctx)
    beginGestureLockedX(t.tool)
    expect(t.tool.capturingInput()).toBe(true)

    t.tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })

    expect(t.tool.capturingInput()).toBe(true)
  })
})

// Post-commit distance retype (retypeWindow.ts): type a distance after a
// move or copy commits and it is redone at that distance along the same
// direction. Shares the idle buffer with the ×N / /N array window.
describe('MoveTool — retype the distance after the commit', () => {
  const lastTranslation = (t: ReturnType<typeof makeTool>) => {
    const calls = t.scene.transform_selection.mock.calls
    return translationOf(calls[calls.length - 1][3] as Float64Array)
  }

  it('a plain move redone at a typed distance: one undo, the same nodes moved the new distance along X', () => {
    const t = makeTool()
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(1)
    expect(lastTranslation(t)).toEqual([2, 0, 0])
    expect(t.tool.statusHint()).toContain('redo the move')
    expect(t.tool.capturesKey('3')).toBe(true)
    expect(t.tool.capturesKey('m')).toBe(false) // shortcuts still work with an empty buffer
    expect(t.tool.capturesKey(' ')).toBe(false)

    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(2)
    expect(lastTranslation(t)).toEqual([3, 0, 0])
    expect(t.onCommit).toHaveBeenCalledTimes(2)

    // Negative flips; Space is never captured even with a buffer open.
    typeKeys(t.tool, '-1')
    expect(t.tool.capturesKey(' ')).toBe(false)
    t.tool.onKey(makeKeyEvent('Enter'))
    const tr = lastTranslation(t)
    expect(tr[0]).toBeCloseTo(-1, 9)
    expect(tr[1]).toBeCloseTo(0, 9)
    expect(tr[2]).toBeCloseTo(0, 9)
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(2)
  })

  it('a copy redone at a typed distance re-duplicates from the originals; 3x afterwards still arrays', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt')) // copy on
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.tool.statusHint()).toContain('redo the copy')

    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    const dupCalls = t.scene.duplicate_selection_array.mock.calls
    const affine = dupCalls[1][2] as Float64Array
    expect(translationOf(affine)).toEqual([3, 0, 0])

    // The array window rides along: 3x now makes three copies at 3 m spacing.
    typeKeys(t.tool, '3x')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(3)
    expect(dupCalls[2][3]).toBe(3)
  })

  it('Escape closes the window; a stale generation is reported', () => {
    const t = makeTool()
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    typeKeys(t.tool, '4')
    t.tool.onKey(makeKeyEvent('Escape'))
    expect(t.tool.capturesKey('4')).toBe(false)

    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    t.state.gen++ // something else recorded
    typeKeys(t.tool, '4')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.onToast).toHaveBeenCalledWith(expect.stringContaining('move'))
  })
})

describe('MoveTool — retype after an arranging drawn-shape move', () => {
  it('re-commits with the PRE-commit shape refs (the undo restores the original keys), while the visible selection is the re-resolved unit', () => {
    const sel: NodeRef[] = [di(40n)]
    const t = makeTool(sel)
    const scene = t.scene as unknown as Record<string, ReturnType<typeof vi.fn>>
    const original = JSON.parse(JSON.stringify(t.drawing.units, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v)))
    const restore = () => {
      t.drawing.units = original.map((u: { key: string; edges: { key: string; a: number[]; b: number[] }[] }) => ({
        key: BigInt(u.key.slice(0, -1)),
        edges: u.edges.map((e) => ({ key: BigInt(e.key.slice(0, -1)), a: [...e.a], b: [...e.b] })),
      }))
    }
    // The first landing arranges (unit 40 merges into fresh unit 99 holding
    // the moved edge under a new key); the undo the retype window runs
    // restores the original keys.
    const applyImpl = scene.draft_transform_edges.getMockImplementation() as (...a: unknown[]) => void
    scene.draft_transform_edges.mockImplementation((...args: unknown[]) => {
      applyImpl(...args)
      // EVERY landing arranges: the moved edge re-keys into fresh unit 99.
      t.drawing.units = [
        { key: 99n, edges: [{ key: 60n, a: t.drawing.units[0].edges[0].a, b: t.drawing.units[0].edges[0].b }] },
        t.drawing.units[1],
      ]
    })
    const undo = scene.scene_undo.getMockImplementation() as () => unknown
    scene.scene_undo.mockImplementation(() => {
      restore()
      return undo()
    })

    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onCommit).toHaveBeenLastCalledWith([di(99n)])

    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(scene.draft_transform_edges).toHaveBeenCalledTimes(2)
    // Both commits name the ORIGINAL unit's edge, never the dead merged key 60.
    expect([...(scene.draft_transform_edges.mock.calls[0][1] as BigUint64Array)]).toEqual([10n])
    expect([...(scene.draft_transform_edges.mock.calls[1][1] as BigUint64Array)]).toEqual([10n])
    expect(translationOf(scene.draft_transform_edges.mock.calls[1][2] as Float64Array)).toEqual([3, 0, 0])
    // The visible selection after the retype is the re-resolved unit again.
    expect(t.onCommit).toHaveBeenLastCalledWith([di(99n)])
  })
})

describe('MoveTool — drawing plane constraint for drawn selections', () => {
  const withStore = (t: ReturnType<typeof makeTool>, store: object) => {
    ;(t.scene.plane_units_json as ReturnType<typeof vi.fn>).mockImplementation(() =>
      JSON.stringify({
        units: [{
          key: hex(40n),
          edges: [{ key: hex(10n), a: [0, 0, 0], b: [1, 0, 0], store, curve: null }],
        }],
        regions: [],
      }),
    )
  }

  it('a drawn shape with a face edge slides ON its plane (constraintPlane from its context)', () => {
    const t = makeTool([di(40n)])
    withStore(t, { kind: 'topology', object: 1 })
    t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(t.tool.snapConstraint()?.constraintPlane).toEqual({ point: [0, 0, 0], normal: [0, 0, 1] })
  })

  it('a shape drawn wholly over the void is NOT constrained (it may detach out of its plane)', () => {
    const t = makeTool([di(40n)])
    withStore(t, { kind: 'free', sketch: 3 })
    t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(t.tool.snapConstraint()?.constraintPlane).toBeUndefined()
  })

  it('a mixed object + drawn selection is not constrained', () => {
    const t = makeTool([{ kind: 'object', id: 1n }, di(40n)])
    withStore(t, { kind: 'topology', object: 1 })
    t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(t.tool.snapConstraint()?.constraintPlane).toBeUndefined()
  })
})

describe('MoveTool — retype window edge cases', () => {
  it('a half-typed post-commit value does not leak into the next gesture', () => {
    const t = makeTool()
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    typeKeys(t.tool, '7') // starts a retype, never finished
    beginGestureLockedX(t.tool) // new gesture
    typeKeys(t.tool, '1')
    t.tool.onKey(makeKeyEvent('Enter'))
    const calls = t.scene.transform_selection.mock.calls
    expect(translationOf(calls[calls.length - 1][3] as Float64Array)[0]).toBeCloseTo(1, 9) // not 71
  })

  it('after a PLAIN move, a slash is an imperial fraction bar, not an array token', () => {
    const t = makeTool()
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    typeKeys(t.tool, '1/2"')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const calls = t.scene.transform_selection.mock.calls
    expect(translationOf(calls[calls.length - 1][3] as Float64Array)[0]).toBeCloseTo(0.0127, 6)
  })
})

describe('MoveTool — a typed distance after an array re-spaces the array', () => {
  it('copy 2, then 3x, then 3 → three copies at 3 m; then 5x → five at 3 m; then 1.5 → five at 1.5 m', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2'); t.tool.onKey(makeKeyEvent('Enter'))
    const dup = t.scene.duplicate_selection_array.mock.calls
    typeKeys(t.tool, '3x'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(dup.length).toBe(2)
    expect(dup[1][3]).toBe(3)
    expect(translationOf(dup[1][2] as Float64Array)[0]).toBeCloseTo(2, 9)

    typeKeys(t.tool, '3'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(dup.length).toBe(3)
    expect(dup[2][3]).toBe(3)
    expect(translationOf(dup[2][2] as Float64Array)[0]).toBeCloseTo(3, 9)

    typeKeys(t.tool, '5x'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(dup.length).toBe(4)
    expect(dup[3][3]).toBe(5)
    expect(translationOf(dup[3][2] as Float64Array)[0]).toBeCloseTo(3, 9)

    typeKeys(t.tool, '1.5'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(dup.length).toBe(5)
    expect(dup[4][3]).toBe(5)
    expect(translationOf(dup[4][2] as Float64Array)[0]).toBeCloseTo(1.5, 9)
    // Every re-space retracted exactly the previous array (one step each).
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(4)
  })

  it('a divide array re-spaced keeps dividing the NEW distance', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2'); t.tool.onKey(makeKeyEvent('Enter'))
    typeKeys(t.tool, '4/'); t.tool.onKey(makeKeyEvent('Enter'))
    typeKeys(t.tool, '8'); t.tool.onKey(makeKeyEvent('Enter'))
    const dup = t.scene.duplicate_selection_array.mock.calls
    expect(dup[dup.length - 1][3]).toBe(4)
    expect(translationOf(dup[dup.length - 1][2] as Float64Array)[0]).toBeCloseTo(2, 9) // 8 / 4
  })
})


describe('MoveTool — a clicked distance is exactly the displayed distance', () => {
  // These pin the Exact digits (9.399 mm); the default mm rounding is coarser.
  beforeAll(() => setPrecision('exact', 'mm'))
  afterAll(() => setPrecision(DEFAULT_PRECISION.mm, 'mm'))
  const prev = getLengthUnit()
  const target: [number, number] = [0.0093987, 0.0037873] // |.| = 10.133 mm, shows "10.133 mm"

  function drag(kind: string, typed: string | null, locked = false) {
    const { tool, scene, onMeasurement } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    if (locked) tool.onKey(makeKeyEvent('ArrowRight'))
    const s = { ...makeSnap(locked ? target[0] : target[0], locked ? 0 : target[1], 0), kind }
    tool.onPointerMove(s, rayThrough(s.x, s.y))
    if (typed === null) tool.onPointerDown(s, rayThrough(s.x, s.y))
    else {
      typeKeys(tool, typed)
      tool.onKey(makeKeyEvent('Enter'))
    }
    expect(scene.transform_selection).toHaveBeenCalledTimes(1)
    return { t: translationOf(scene.transform_selection.mock.calls[0][3] as Float64Array), onMeasurement }
  }

  it('a free click moves by the same vector as typing the shown distance', () => {
    setLengthUnit('mm')
    try {
      const clicked = drag('ground', null)
      expect(clicked.onMeasurement).toHaveBeenCalledWith('10.133 mm')
      expect(clicked.t).toEqual(drag('ground', '10.133').t)
      expect(Math.hypot(...clicked.t)).toBeCloseTo(10.133 * 0.001, 15)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('a free click under an axis lock moves by the shown signed distance along the axis', () => {
    setLengthUnit('mm')
    try {
      const clicked = drag('on-axis', null, true)
      expect(clicked.t).toEqual(drag('on-axis', '9.399', true).t)
      expect(clicked.t[0]).toBe(9.399 * 0.001)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('a hard-snapped (endpoint) click moves exactly to the snapped point', () => {
    setLengthUnit('mm')
    try {
      expect(drag('endpoint', null).t).toEqual([target[0], target[1], 0])
    } finally {
      setLengthUnit(prev)
    }
  })
})

describe('MoveTool — a refused mixed array resolve is atomic', () => {
  it('aborts the attempt (drawn half included) and redoes the previous copy', () => {
    const t = makeTool([{ kind: 'object', id: 1n }, di(40n)])
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.draft_end).toHaveBeenCalledTimes(1)
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)

    t.scene.duplicate_selection_array.mockImplementationOnce(() => {
      throw new Error('Transform: refused')
    })
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))

    // One undo retracted the previous commit (its single entry); the refused
    // attempt's bracket aborted — restoring the document and the redo stack
    // exactly — so ONE redo brings the previous copy back. Nothing is re-laid.
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.scene_redo).toHaveBeenCalledTimes(1)
    expect(t.brackets.aborted).toBe(1)
    expect(t.scene.draft_end).toHaveBeenCalledTimes(2) // first copy + the aborted attempt's draft
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    expect(t.scene.undo_depth()).toBe(1)
    expect(t.onToast).toHaveBeenCalled()
    expect(t.onArrayCommit).toHaveBeenCalledTimes(0)

    // The window is still hot and re-stamped: another count resolves.
    typeKeys(t.tool, 'x2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.duplicate_selection_array.mock.calls.at(-1)![3]).toBe(2)
  })
})

describe('MoveTool — array history edge cases', () => {
  const STALE = /changed|again|stale|Retype|retype/i

  it('a refused xN that recorded nothing keeps the typed-distance window armed', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2'); t.tool.onKey(makeKeyEvent('Enter'))
    t.scene.duplicate_selection_array.mockImplementationOnce(() => { throw new Error('Transform: refused') })
    typeKeys(t.tool, 'x3'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_redo).toHaveBeenCalledTimes(1) // nothing recorded: redo, not a re-lay
    t.onToast.mockClear()
    typeKeys(t.tool, '5'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast.mock.calls.map((c) => c[0] as string).filter((m) => STALE.test(m))).toEqual([])
    const dup = t.scene.duplicate_selection_array.mock.calls.at(-1)!
    expect(translationOf(dup[2] as Float64Array)[0]).toBeCloseTo(5, 9)
  })

  it('a refused relay that recorded nothing re-stamps the array window so the next xN is accepted', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2'); t.tool.onKey(makeKeyEvent('Enter'))
    typeKeys(t.tool, '3x'); t.tool.onKey(makeKeyEvent('Enter'))
    t.scene.duplicate_selection_array.mockImplementationOnce(() => { throw new Error('Transform: refused') })
    typeKeys(t.tool, '4'); t.tool.onKey(makeKeyEvent('Enter')) // relay refused
    t.onToast.mockClear()
    typeKeys(t.tool, '5x'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).not.toHaveBeenCalledWith(expect.stringContaining('Array entry ended'), undefined)
    expect(t.scene.duplicate_selection_array.mock.calls.at(-1)![3]).toBe(5)
  })

  describe('a relay whose second drawn context refuses (the bracket aborts: history and stack are put back exactly)', () => {
    const CTX2 = ctxOfPlane([0, 5, 0], [0, 1, 0]) as string
    const di2 = (key: bigint): NodeRef => draftRef('draft-island', CTX2, hex(key))

    it('does not over-undo unrelated history on a refused retype', () => {
      const t = makeTool([{ kind: 'object', id: 1n }, di(40n), di2(41n)])
      const ctxOf = new Map<bigint, string>()
      let nextD = 1n
      let failsLeft = 0
      t.scene.draft_begin_ctx.mockImplementation(((ctx: string) => {
        const d = nextD++
        ctxOf.set(d, ctx)
        return d
      }) as never)
      t.scene.draft_end.mockImplementation(((d: bigint) => {
        if (ctxOf.get(d) === CTX2 && failsLeft > 0) {
          failsLeft -= 1
          throw new Error('WouldRetopologize: refused')
        }
        t.state.hash++
        t.state.gen++
        return '{"objects":[],"sketches":[],"kept_as_sketch":[]}'
      }) as never)
      t.tool.onKey(makeKeyEvent('Alt'))
      beginGestureLockedX(t.tool)
      typeKeys(t.tool, '2'); t.tool.onKey(makeKeyEvent('Enter'))
      typeKeys(t.tool, '2x'); t.tool.onKey(makeKeyEvent('Enter'))
      const depth = t.scene.undo_depth()
      expect(depth).toBe(1) // two drawn contexts + the object array = ONE step (was 3)
      // The relay: context 1 records, context 2 refuses (batch + per-rep
      // fallback), and the bracket aborts, taking context 1's draft with it.
      failsLeft = 2
      typeKeys(t.tool, '4'); t.tool.onKey(makeKeyEvent('Enter'))
      // The refused retype put back exactly what was there: nothing popped
      // beyond the window's own steps.
      expect(t.scene.undo_depth()).toBe(depth)
    })

    it('the drawn half succeeding and the object half refusing during a relay also nets out', () => {
      const t = makeTool([{ kind: 'object', id: 1n }, di(40n)])
      t.tool.onKey(makeKeyEvent('Alt'))
      beginGestureLockedX(t.tool)
      typeKeys(t.tool, '2'); t.tool.onKey(makeKeyEvent('Enter'))
      typeKeys(t.tool, '2x'); t.tool.onKey(makeKeyEvent('Enter'))
      const depth = t.scene.undo_depth()
      t.scene.duplicate_selection_array.mockImplementationOnce(() => { throw new Error('Transform: refused') })
      typeKeys(t.tool, '4'); t.tool.onKey(makeKeyEvent('Enter'))
      expect(t.scene.undo_depth()).toBe(depth)
    })
  })
})

describe('MoveTool — one gesture, one undo step (history bracket)', () => {
  it('Select All + Move (an object and drawing together) records exactly ONE undo step labeled Move', () => {
    const t = makeTool([{ kind: 'object', id: 1n }, di(40n), di(41n)])
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.draft_end).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(1)
    expect(t.scene.undo_depth()).toBe(1) // draft + object move folded into one entry
    expect(t.brackets.labels).toEqual(['Move'])
    expect(t.brackets.aborted).toBe(0)
    // And one typed re-distance retracts exactly that one step.
    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.undo_depth()).toBe(1)
  })

  it('a refusal mid-gesture (object move after the drawing landed) aborts: nothing moved, nothing recorded', () => {
    const t = makeTool([{ kind: 'object', id: 1n }, di(40n)])
    const before = structuredClone(t.drawing.units)
    t.scene.transform_selection.mockImplementationOnce(() => { throw new Error('NothingToMove: refused') })
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).toHaveBeenCalled()
    expect(t.brackets.aborted).toBe(1)
    expect(t.scene.undo_depth()).toBe(0)
    expect(t.drawing.units).toEqual(before) // the drawing moved by the landed draft came back
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
  })
})

// Click-drag-release (#52b, viewport/dragCommit.ts): the ×N array window keeps
// `capturingInput()` true at idle, so the gate reads `gestureInFlight()`.
describe('MoveTool — click-drag-release gate', () => {
  it('a press during the ×N window arms a drag; the drag release moves; a mid-gesture press never arms', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    beginGestureLockedX(t.tool)
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.tool.capturingInput()).toBe(true) // the array window
    expect(t.tool.gestureInFlight()).toBe(false)

    const gate = new DragCommitGate()
    const press = () => t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(gate.press(t.tool, 1, 100, 100, press)).toBe(true)
    expect(t.tool.gestureInFlight()).toBe(true)
    // A press now (mid-gesture) is a plain click, never armed.
    expect(new DragCommitGate().press(t.tool, 2, 0, 0, () => { /* not delivered */ })).toBe(false)
    gate.move(1, 160, 100)
    const commits = t.scene.duplicate_selection_array.mock.calls.length + t.onCommit.mock.calls.length
    expect(gate.release(t.tool, 1, 160, 100, () => t.tool.onPointerDown(makeSnap(3, 0, 0), rayThrough(3, 0)))).toBe(true)
    expect(t.scene.duplicate_selection_array.mock.calls.length + t.onCommit.mock.calls.length).toBeGreaterThan(commits)
    expect(t.tool.gestureInFlight()).toBe(false)
  })
})

describe('MoveTool — the drag ghost is session geometry, mirrored onto sibling placements', () => {
  it('arming a move puts a marked ghost in the preview; the mirror follows it and clears with Esc', () => {
    const objectsGroup = new THREE.Group()
    const sub = new THREE.Group()
    sub.name = 'Object_1'
    sub.add(new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial()))
    objectsGroup.add(sub)
    const preview = new THREE.Group()
    const { scene } = makeWasmScene()
    const tool = new MoveTool(scene as never, preview, objectsGroup, [{ kind: 'object', id: 1n }], vi.fn(), vi.fn(), vi.fn(), null, vi.fn(), vi.fn())
    const root = new THREE.Group()
    const mirror = new SiblingPreviewMirror({ source: preview, root, frames: () => [1, 0, 0, 4, 0, 1, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 7, 0, 0, 1, 0] })

    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    const ghosts = preview.children.filter(isSessionGhost)
    expect(ghosts).toHaveLength(1)
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)

    // The ghost rides the cursor; so do its mirrors, in place.
    tool.onPointerMove(makeSnap(1, 0, 0), rayThrough(1, 0))
    mirror.sync()
    expect(mirror.lastWork).toBe('retarget')
    const clone = (root.children[0] as THREE.Group).children[0]
    expect(clone.position.x).toBeCloseTo(1)

    tool.onKey(makeKeyEvent('Escape'))
    mirror.sync()
    expect(preview.children.filter(isSessionGhost)).toHaveLength(0)
    expect(mirror.mirrorCount).toBe(0)
  })
})

describe('MoveTool — a click-taken selection lasts only for the gesture (SketchUp pre-highlight rule)', () => {
  const NODE: NodeRef = { kind: 'object', id: 7n }

  /** A tool with nothing selected whose click takes NODE. */
  function makeAcquiring() {
    const t = makeTool([])
    const acquire = vi.fn(() => [NODE])
    const release = vi.fn()
    t.tool.setSelectionAcquirer(acquire, release)
    return { ...t, acquire, release }
  }

  /** Click NODE at the origin, then click 2 m along X. */
  function moveTwoAlongX(tool: MoveTool): void {
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerMove(makeSnap(2, 0, 0), rayThrough(2, 0))
    tool.onPointerDown(makeSnap(2, 0, 0), rayThrough(2, 0))
  }

  it('a landed move empties the selection again', () => {
    const t = makeAcquiring()
    moveTwoAlongX(t.tool)
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(1)
    expect(t.release).toHaveBeenCalledTimes(1)
    // The tool's own cache is empty too: the next click takes again.
    t.tool.onPointerDown(makeSnap(5, 0, 0), rayThrough(5, 0))
    expect(t.acquire).toHaveBeenCalledTimes(2)
  })

  it('Esc mid-gesture, and a click in place, give the selection back', () => {
    const esc = makeAcquiring()
    esc.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    esc.tool.onKey(makeKeyEvent('Escape'))
    expect(esc.release).toHaveBeenCalledTimes(1)

    const inPlace = makeAcquiring()
    inPlace.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    inPlace.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(inPlace.scene.transform_selection).not.toHaveBeenCalled()
    expect(inPlace.release).toHaveBeenCalledTimes(1)
  })

  it('a refused move gives the selection back', () => {
    const t = makeAcquiring()
    t.scene.transform_selection.mockImplementationOnce(() => { throw new Error('InvalidTransform: refused') })
    moveTwoAlongX(t.tool)
    expect(t.onToast).toHaveBeenCalled()
    expect(t.release).toHaveBeenCalledTimes(1)
  })

  it('a distance typed before the cursor left the base point gives the selection back', () => {
    const t = makeAcquiring()
    t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    typeKeys(t.tool, '2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.transform_selection).not.toHaveBeenCalled()
    expect(t.tool.capturingInput()).toBe(false)
    expect(t.release).toHaveBeenCalledTimes(1)
  })

  it('a refused retype leaves the gesture\'s release to the retype that lands', () => {
    const t = makeAcquiring()
    moveTwoAlongX(t.tool)
    expect(t.release).toHaveBeenCalledTimes(1)

    t.scene.transform_selection.mockImplementationOnce(() => { throw new Error('InvalidTransform: refused') })
    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).toHaveBeenCalled()
    expect(t.release).toHaveBeenCalledTimes(1)

    typeKeys(t.tool, '4')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(3)
    expect(t.release).toHaveBeenCalledTimes(2)
  })

  it('a copy, and its ×N array, leave nothing selected either — the array still fans from the source', () => {
    const t = makeAcquiring()
    t.tool.onKey(makeKeyEvent('Alt'))
    moveTwoAlongX(t.tool)
    expect(t.scene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.release).toHaveBeenCalledTimes(1)

    typeKeys(t.tool, '3x')
    t.tool.onKey(makeKeyEvent('Enter'))
    const [, ids, , count] = t.scene.duplicate_selection_array.mock.calls.at(-1)!
    expect(Array.from(ids as BigUint64Array)).toEqual([7n])
    expect(count).toBe(3)
    expect(t.release).toHaveBeenCalledTimes(2)
  })

  it('a selection made beforehand is kept, and copies become it', () => {
    const t = makeTool([NODE])
    const release = vi.fn()
    t.tool.setSelectionAcquirer(() => [{ kind: 'object', id: 99n }], release)
    moveTwoAlongX(t.tool)
    expect(release).not.toHaveBeenCalled()

    t.tool.onKey(makeKeyEvent('Alt'))
    moveTwoAlongX(t.tool)
    expect(release).not.toHaveBeenCalled()
    expect(t.onCommit).toHaveBeenLastCalledWith([{ kind: 'object', id: expect.any(BigInt) }])
  })

  it('a retyped distance re-lands with nothing selected; once something else is selected, it is left alone', () => {
    const t = makeAcquiring()
    moveTwoAlongX(t.tool)
    expect(t.release).toHaveBeenCalledTimes(1)

    typeKeys(t.tool, '3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(2)
    expect(t.release).toHaveBeenCalledTimes(2)

    t.tool.setSelection([{ kind: 'object', id: 3n }]) // picked in the Outliner
    typeKeys(t.tool, '4')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(3)
    expect(t.release).toHaveBeenCalledTimes(2)
  })
})

describe('MoveTool — what a click would take (clickAcquire, the hover highlight)', () => {
  it('idle with nothing selected: a node under the cursor', () => {
    const t = makeTool([])
    t.tool.setSelectionAcquirer(() => null)
    expect(t.tool.clickAcquire()).toEqual({ guides: false, nodes: true })
  })

  it('a guide first when guides can be taken; with a guide selected, only another guide', () => {
    const t = makeTool([])
    t.tool.setSelectionAcquirer(() => null)
    t.tool.setGuideHooks(() => null, vi.fn())
    expect(t.tool.clickAcquire()).toEqual({ guides: true, nodes: true })
    t.tool.setGuideTarget(5n)
    expect(t.tool.clickAcquire()).toEqual({ guides: true, nodes: false })
  })

  it('nothing with a selection, or mid-gesture', () => {
    const selected = makeTool([{ kind: 'object', id: 1n }])
    selected.tool.setSelectionAcquirer(() => null)
    expect(selected.tool.clickAcquire()).toBeNull()

    const t = makeTool([])
    t.tool.setSelectionAcquirer(() => [{ kind: 'object', id: 7n }])
    t.tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(t.tool.clickAcquire()).toBeNull()
  })
})

describe('MoveTool — dashed line from the base point to the destination', () => {
  function dashedLines(preview: THREE.Object3D): THREE.LineSegments[] {
    const out: THREE.LineSegments[] = []
    preview.traverse((o) => {
      if (o instanceof THREE.LineSegments && o.material instanceof THREE.LineDashedMaterial) out.push(o)
    })
    return out
  }

  it('follows the destination while the move runs and goes with the commit', () => {
    const preview = new THREE.Group()
    const { scene } = makeWasmScene()
    const tool = new MoveTool(scene as never, preview, null, [{ kind: 'object', id: 1n }], vi.fn(), vi.fn(), vi.fn(), null)
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerMove(makeSnap(2, 3, 0), rayThrough(2, 3))

    const [line] = dashedLines(preview)
    expect(line).toBeDefined()
    const pos = line.geometry.getAttribute('position')
    expect([pos.getX(0), pos.getY(0), pos.getX(1), pos.getY(1)]).toEqual([0, 0, 2, 3])

    tool.onPointerDown(makeSnap(2, 3, 0), rayThrough(2, 3))
    expect(dashedLines(preview)).toHaveLength(0)
  })
})
