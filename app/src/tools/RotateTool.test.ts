import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RotateTool } from './RotateTool'
import { DragCommitGate } from '../viewport/dragCommit'
import { CleanModifierTap } from '../viewport/cleanModifierTap'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { ctxGround, draftRef } from './planeUnits'
import { applyAffineToPoint } from './transformMath'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { worldPerPixelPerspective, worldPerPixelOrtho } from '../viewport/math'
import type { DrawingAxes } from './drawingAxes'
import { SiblingPreviewMirror } from '../viewport/siblingPreview'
import { isSessionGhost } from './transformPreview'

// `isMac` is a load-time constant read from `navigator.platform`, so this
// whole file mocks `../platform` to `isMac: true` (macOS) rather than
// relying on the CI host's actual OS — matches MoveTool.test.ts's identical
// mock. The one "Windows/Linux" test below overrides it with a scoped
// `vi.doMock` + `vi.resetModules()` + dynamic re-import instead (see that
// test for why the statically-imported `RotateTool` above is unaffected).
vi.mock('../platform', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../platform')>()
  return { ...actual, isMac: true, COPY_MODIFIER_KEY: 'Alt', COPY_MODIFIER_LABEL: 'Option' }
})

const CTX = ctxGround()
/** Kernel keys are 16-hex strings. */
const hex = (n: bigint): string => n.toString(16).padStart(16, '0')
/** A `draft-island` ref of the ground plane's drawing. */
const di = (key: bigint): NodeRef => draftRef('draft-island', CTX, hex(key))

type V3 = [number, number, number]
interface DSeg { key: bigint; a: V3; b: V3 }
interface DUnit { key: bigint; sketch: number; edges: DSeg[]; curve?: { center: V3; radius: number } }

/** ~2° axis tolerance, matching RotateTool's own AXIS_SNAP_TOL_DOT. */
const TOL = Math.cos((2 * Math.PI) / 180)

/** A ray straight down (−Z) through world (x, y): hits the z=0 plane at (x,y,0),
 * so a sweep move at (x,y) lands the cursor there in the ground rotation plane. */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** Minimal KeyboardEvent-shaped fake — onKey only reads .key/.repeat and calls
 * .preventDefault(). */
function makeKeyEvent(key: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  let defaultPrevented = false
  return {
    key,
    repeat: opts.repeat ?? false,
    get defaultPrevented() { return defaultPrevented },
    preventDefault: () => { defaultPrevented = true },
  } as unknown as KeyboardEvent
}

/** Type a string one key at a time (no Enter — callers add that explicitly). */
function typeKeys(tool: RotateTool, text: string): void {
  for (const ch of text) tool.onKey(makeKeyEvent(ch))
}

/** The rotation angle (radians) of a row-major 3×4 affine that is a pure
 * rotation about world Z through the origin (pivot = (0,0,0) in every test
 * gesture below, so `rotateAboutPivotAxis` reduces to `rotationZAffine`). */
function thetaOfZAffine(affine: Float64Array): number {
  return Math.atan2(affine[4], affine[0])
}

/** World-identity drawing axes (tool-parity §4) — the default frame every
 *  test exercises unless it explicitly overrides `frame`. */
const WORLD_FRAME_FLAT = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]

/** Minimal WasmScene stub — only the members RotateTool calls. `frame` is
 *  the flat 12-float buffer `Scene::axes()` would return — defaults to
 *  world identity so existing tests are unaffected; a moved-frame test
 *  overrides it to prove `axisDir` reads the CURRENT frame, not a literal
 *  world constant. */
function makeWasmScene(faceNormal?: [number, number, number], frame: number[] = WORLD_FRAME_FLAT) {
  let nextObjId = 100n
  let nextSketchId = 900n
  let nextEdge = 500n
  let gen = 1n
  // --- drawing surface (drafting engine): the ground plane's listing. Units
  // 40 (edge 10), 41 (edge 20) in sketch 3 and 45 (edge 30, a facet of the
  // drawn circle center (2,0,0) r 1) in sketch 6 — the same shapes as the
  // sketch fixture below. `draft_transform_edges` moves the listed edges;
  // copies land in unit 78 (draft copies and sketch replays alike) and
  // out-of-plane detach copies in unit 77. Mutate `drawing.units` (with a
  // generation bump) to simulate an arranging landing.
  const drawing: { units: DUnit[] } = {
    units: [
      { key: 40n, sketch: 3, edges: [{ key: 10n, a: [0, 0, 0], b: [1, 0, 0] }] },
      { key: 41n, sketch: 3, edges: [{ key: 20n, a: [5, 5, 0], b: [6, 5, 0] }] },
      {
        key: 45n, sketch: 6, edges: [{ key: 30n, a: [3, 0, 0], b: [2, 1, 0] }],
        curve: { center: [2, 0, 0], radius: 1 },
      },
    ],
  }
  const origSegs = (): Map<bigint, DSeg> =>
    new Map(drawing.units.flatMap((u) => u.edges).map((e) => [e.key, e]))
  const added = new Map<bigint, number[]>() // island 78
  const copied = new Map<bigint, number[]>() // island 77
  const jsonUnit = (key: bigint, sketch: number, edges: { key: bigint; a: number[]; b: number[] }[], curve?: object) => ({
    key: hex(key),
    edges: edges.map((e) => ({
      key: hex(e.key), a: e.a, b: e.b, store: { kind: 'free', sketch }, curve: curve ?? null,
    })),
  })
  const segs = (m: Map<bigint, number[]>) =>
    [...m].map(([key, v]) => ({ key, a: v.slice(0, 3), b: v.slice(3, 6) }))
  const undoCalls = (): number => scene.scene_undo.mock.calls.length
  // History bracket: open snapshots the generation and every drawing store;
  // abort restores them exactly; commit folds everything recorded since open
  // into ONE entry (the generation moves by one).
  const brackets = {
    open: null as null | { gen: bigint; units: DUnit[]; added: Map<bigint, number[]>; copied: Map<bigint, number[]> },
    labels: [] as string[],
    aborted: 0,
  }
  const scene = {
    brackets,
    history_bracket_open: vi.fn(() => {
      if (brackets.open !== null) throw new Error('StaleReference: a history bracket is already open')
      brackets.open = { gen, units: structuredClone(drawing.units), added: new Map(added), copied: new Map(copied) }
    }),
    history_bracket_commit: vi.fn((label: string) => {
      const b = brackets.open
      brackets.open = null
      if (b === null) throw new Error('StaleReference: no history bracket')
      if (gen === b.gen) return false
      gen = b.gen + 1n
      brackets.labels.push(label)
      return true
    }),
    history_bracket_abort: vi.fn(() => {
      const b = brackets.open
      brackets.open = null
      if (b === null) return
      brackets.aborted += 1
      gen = b.gen
      drawing.units = b.units
      added.clear()
      for (const [k, v] of b.added) added.set(k, v)
      copied.clear()
      for (const [k, v] of b.copied) copied.set(k, v)
    }),
    axes: vi.fn(() => new Float64Array(frame)),
    plane_units_json: vi.fn((_ctx: string) =>
      JSON.stringify({
        units: [
          ...drawing.units.map((u) => jsonUnit(u.key, u.sketch, u.edges, u.curve)),
          ...(added.size > 0 ? [jsonUnit(78n, 3, segs(added))] : []),
          ...(copied.size > 0 ? [jsonUnit(77n, 900, segs(copied))] : []),
        ],
        regions: [],
      }),
    ),
    sketch_island_at_edge_positions: vi.fn((s: bigint, a: Float64Array, b: Float64Array) => {
      const near = (p: number[], q: ArrayLike<number>) => p.every((v, i) => Math.abs(v - q[i]) < 1e-9)
      for (const u of drawing.units) {
        if (BigInt(u.sketch) !== s) continue
        for (const e of u.edges) {
          if ((near(e.a, a) && near(e.b, b)) || (near(e.a, b) && near(e.b, a))) return u.key
        }
      }
      return undefined
    }),
    draft_begin_ctx: vi.fn((_ctx: string, _label: string) => 1n),
    draft_transform_edges: vi.fn((_d: bigint, keys: BigUint64Array, affine: Float64Array) => {
      const set = new Set([...keys])
      for (const u of drawing.units) {
        for (const e of u.edges) {
          if (!set.has(e.key)) continue
          e.a = applyAffineToPoint(affine, e.a)
          e.b = applyAffineToPoint(affine, e.b)
        }
      }
    }),
    draft_copy_edges: vi.fn((_d: bigint, keys: BigUint64Array, affine: Float64Array) => {
      const all = origSegs()
      for (const k of keys) {
        const e = all.get(k)
        if (e === undefined) continue
        added.set(nextEdge++, [...applyAffineToPoint(affine, e.a), ...applyAffineToPoint(affine, e.b)])
      }
    }),
    // Ending a draft is a commit: the history generation moves by one.
    draft_end: vi.fn((_d: bigint) => {
      gen++
      return '{"objects":[],"sketches":[],"kept_as_sketch":[]}'
    }),
    draft_cancel: vi.fn(),
    drawing,
    face_normal: vi.fn((..._args: unknown[]) => {
      if (faceNormal === undefined) throw new Error('not a live world-object face')
      return new Float64Array(faceNormal)
    }),
    // No face under the ray → RotateTool's fallback returns world +Z.
    pick_face: vi.fn(() => undefined),
    transform_selection: vi.fn(() => { gen++ }),
    duplicate_selection_array: vi.fn(
      (_kinds: Uint8Array, ids: BigUint64Array, _affine: Float64Array, count: number) => {
        const out: { kind: string; id: bigint }[] = []
        for (let k = 0; k < count; k++) {
          for (let i = 0; i < ids.length; i++) {
            out.push({ kind: 'object', id: nextObjId++ })
          }
        }
        gen++
        return out
      },
    ),
    max_array_count: vi.fn(() => 1000),
    scene_undo: vi.fn(() => { gen++; return { free: () => { /* no-op */ } } }),
    scene_redo: vi.fn(() => { gen++; return { free: () => { /* no-op */ } } }),
    history_generation: vi.fn(() => gen),
    // The undo stack's size: records minus undos plus redos = gen - 2 * undos.
    undo_depth: vi.fn(() => Number(gen) - 1 - 2 * undoCalls()),
    // --- sketch copy surface (Rotate+Alt sketch copies). Two sketches, both
    // on the ground plane (point origin, normal +Z):
    //   - sketch 3: two single-edge islands, 40 (edge 10, endpoints
    //     (0,0,0)-(1,0,0)) and 41 (edge 20, endpoints (5,5,0)-(6,5,0)) — no
    //     curve, plain lines, mirroring MoveTool.test.ts's fixture.
    //   - sketch 6: one island, 45 (edge 30, endpoints (3,0,0)-(2,1,0)),
    //     riding a drawn CIRCLE (curve 99, center (2,0,0), radius 1) — for
    //     the curve-identity-through-rotation test.
    // An IN-PLANE replay lands its new edges/island on the SAME sketch
    // (island 78); an OUT-OF-PLANE (or flipping) copy lands a new sketch id
    // whose sole island is 77.
    sketch_plane: vi.fn((sketch: bigint) =>
      sketch === 3n || sketch === 6n || sketch >= 900n ? new Float64Array([0, 0, 0, 0, 0, 1]) : undefined,
    ),
    sketch_island_ids: vi.fn((sketch: bigint) =>
      sketch === 3n ? [40n, 41n] : sketch === 6n ? [45n] : sketch >= 900n ? [77n] : [],
    ),
    sketch_island_edges: vi.fn((_sketch: bigint, island: bigint) =>
      island === 40n ? [10n] : island === 41n ? [20n] : island === 45n ? [30n]
      : island === 78n ? [...added.keys()] : island === 77n ? [...copied.keys()] : [],
    ),
    sketch_edge_island: vi.fn((_sketch: bigint, edge: bigint) =>
      edge >= 500n ? 78n : edge === 10n ? 40n : edge === 20n ? 41n : edge === 30n ? 45n : undefined,
    ),
    sketch_edge_endpoints: vi.fn((_sketch: bigint, edge: bigint) =>
      edge === 10n ? [0, 0, 0, 1, 0, 0]
      : edge === 20n ? [5, 5, 0, 6, 5, 0]
      : edge === 30n ? [3, 0, 0, 2, 1, 0]
      : added.get(edge) ?? copied.get(edge),
    ),
    sketch_edge_curve: vi.fn((_sketch: bigint, edge: bigint) => (edge === 30n ? 99n : undefined)),
    sketch_curve_geom: vi.fn((_sketch: bigint, curve: bigint) => (curve === 99n ? [2, 0, 0, 1] : undefined)),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(() => { gen++ }),
    sketch_begin_curve: vi.fn((_sketch: bigint) => 8n),
    sketch_begin_curve_with: vi.fn(
      (_sketch: bigint, _cx: number, _cy: number, _cz: number, _radius: number) => 8n,
    ),
    sketch_end_curve: vi.fn(),
    sketch_add_segment: vi.fn(
      (_sketch: bigint, _ax: number, _ay: number, _az: number, _bx: number, _by: number, _bz: number) => {
        const id = nextEdge++
        added.set(id, [_ax, _ay, _az, _bx, _by, _bz])
        return { new_edges: () => [id], free: () => { /* no-op */ } }
      },
    ),
    // The ghost preview of a whole sketch builds its lines regardless of
    // objectsGroup (null in these logic tests) — no geometry needed to
    // exercise the copy/array logic itself.
    sketch_lines: vi.fn(() => new Float32Array(0)),
    sketch_island_lines: vi.fn(() => new Float32Array(0)),
    copy_sketch_islands: vi.fn(
      (sketch: bigint, islands: BigUint64Array, affine: Float64Array) => {
        const id = nextSketchId++
        const all = origSegs()
        for (const u of drawing.units) {
          if (BigInt(u.sketch) !== sketch || !islands.includes(u.key)) continue
          for (const e of u.edges) {
            const o = all.get(e.key)!
            copied.set(nextEdge++, [...applyAffineToPoint(affine, o.a), ...applyAffineToPoint(affine, o.b)])
          }
        }
        gen++
        return JSON.stringify({ objects: [], sketches: [Number(id)], kept_as_sketch: [], faces_closed: [], cells: [] })
      },
    ),
  }
  return scene
}

function makeTool(opts: {
  faceNormal?: [number, number, number]
  selection?: NodeRef[]
  instance?: { id: bigint; group: THREE.Group }
  frame?: number[]
} = {}) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onArrayCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const onCopyModeChange = vi.fn()
  const wasmScene = makeWasmScene(opts.faceNormal, opts.frame)
  const selection: NodeRef[] = opts.selection ?? [{ kind: 'object', id: 1n }]
  const tool = new RotateTool(
    wasmScene as never,
    preview,
    null, // world objectsGroup
    selection,
    onCommit,
    onToast,
    opts.instance === undefined
      ? null
      : (id) => id === opts.instance!.id ? opts.instance!.group : null,
    onMeasurement,
    onCopyModeChange,
    onArrayCommit,
  )
  return { tool, preview, onCommit, onArrayCommit, onToast, onMeasurement, onCopyModeChange, wasmScene }
}

// ── disk inspection helpers ──────────────────────────────────────────────────

function diskGroup(preview: THREE.Group): THREE.Group {
  const g = preview.children.find((c) => c instanceof THREE.Group)
  expect(g, 'a protractor disk group should exist').toBeDefined()
  return g as THREE.Group
}

function ringColorHex(preview: THREE.Group): number {
  const ring = diskGroup(preview).children.find((c) => c instanceof THREE.LineLoop) as THREE.LineLoop
  expect(ring, 'the disk should have a ring (LineLoop)').toBeDefined()
  return (ring.material as THREE.LineBasicMaterial).color.getHex()
}

/** Count of LineSegments in the disk group: the lock tick and/or sweep arms. In
 * the idle phase (no arms) this is 1 exactly when the axis is locked. */
function lineSegmentCount(preview: THREE.Group): number {
  return diskGroup(preview).children.filter((c) => c instanceof THREE.LineSegments).length
}

/** The color RotateTool assigns a ring whose normal points along `dir`
 * (against `frame`, defaulting to world identity — tool-parity §4), run
 * through a LineBasicMaterial so color-management matches the tool's own path. */
function axisColorHex(dir: [number, number, number], frame?: DrawingAxes): number {
  const match = axisColorForDirection(dir, TOL, axisColorsForTheme(getResolvedTheme()), frame)
  expect(match, `${dir} should map to an axis color`).not.toBeNull()
  return new THREE.LineBasicMaterial({ color: match!.color }).color.getHex()
}

// ── tests ────────────────────────────────────────────────────────────────────

describe('RotateTool — protractor widget (idle/hover)', () => {
  it('shows a blue (Z) protractor on the ground by default, centered at the cursor', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))
    expect(diskGroup(preview).position.toArray()).toEqual([1, 2, 0])
    expect(ringColorHex(preview)).toBe(axisColorHex([0, 0, 1]))
    expect(lineSegmentCount(preview)).toBe(0) // unlocked → no tick
  })

  it('recolors the disk to the hovered face axis (side face → red X)', () => {
    const { tool, preview } = makeTool({ faceNormal: [1, 0, 0] })
    tool.onPointerMove(makeSnap({ kind: 'face', elementKind: 'face', object: 1n, element: 2n }), rayThrough(0, 0))
    expect(ringColorHex(preview)).toBe(axisColorHex([1, 0, 0]))
    expect(ringColorHex(preview)).not.toBe(axisColorHex([0, 0, 1]))
  })

  it('a snap on the line where two objects\' faces meet does not adopt the first face\'s axis', () => {
    // The kernel names the first of the two faces as the line's owner of
    // record (here a side face, +X); the line is on both and a pick of
    // neither, so the axis comes from what is under the ray (nothing here
    // → +Z), as for an edge.
    const { tool, preview, wasmScene } = makeTool({ faceNormal: [1, 0, 0] })
    const meetingLine: Snap['meetingLine'] = [
      [0, -1, 0],
      [0, 1, 0],
    ]
    for (const kind of ['on-intersection-line', 'midpoint']) {
      tool.onPointerMove(
        makeSnap({ kind, elementKind: 'meeting-line', object: 1n, element: 2n, meetingLine }),
        rayThrough(0, 0),
      )
      expect(ringColorHex(preview)).toBe(axisColorHex([0, 0, 1]))
    }
    expect(wasmScene.face_normal).not.toHaveBeenCalledWith(1n, 2n)
  })
})

describe('RotateTool — axis locking', () => {
  it('ArrowRight locks X: recolors to red and emphasizes the disk with a normal tick', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    expect(lineSegmentCount(preview)).toBe(0)

    const ev = makeKeyEvent('ArrowRight')
    tool.onKey(ev)
    expect(ev.defaultPrevented).toBe(true)
    expect(ringColorHex(preview)).toBe(axisColorHex([1, 0, 0]))
    expect(lineSegmentCount(preview)).toBe(1) // locked → normal tick added
  })

  it('ArrowRight locks the CURRENT frame\'s red axis, not literal world X, under a moved frame', () => {
    // Frame with red/green swapped relative to world: x=[0,1,0], y=[-1,0,0],
    // z=[0,0,1] — an orthonormal, right-handed frame (tool-parity §4).
    const frame = [0, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1]
    const frameObj: DrawingAxes = { origin: [0, 0, 0], x: [0, 1, 0], y: [-1, 0, 0], z: [0, 0, 1] }
    const { tool, preview } = makeTool({ frame })
    tool.onPointerDown(makeSnap(), rayThrough(0, 0)) // place a pivot so snapConstraint can resolve
    tool.onKey(makeKeyEvent('ArrowRight'))
    const constraint = tool.snapConstraint()
    expect(constraint).not.toBeNull()
    expect(constraint!.constraintPlane.normal).toEqual([0, 1, 0])
    // The disk tint + axis tag (axisColorForDirection's other named consumer,
    // finding 3) must ALSO read the moved frame: the locked normal [0,1,0]
    // is world Y, not world X — a primitive still comparing against literal
    // world axes would tint this red (matching [1,0,0]) instead of green.
    expect(ringColorHex(preview)).toBe(axisColorHex([0, 1, 0], frameObj))
    expect(ringColorHex(preview)).not.toBe(axisColorHex([0, 1, 0]))
  })

  it('ArrowDown clears the lock, returning to the inferred (ground Z) axis', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onKey(makeKeyEvent('ArrowDown'))
    expect(ringColorHex(preview)).toBe(axisColorHex([0, 0, 1]))
    expect(lineSegmentCount(preview)).toBe(0)
  })

  it('Shift toggles the lock on and off', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('Shift'))
    expect(lineSegmentCount(preview)).toBe(1)
    tool.onKey(makeKeyEvent('Shift'))
    expect(lineSegmentCount(preview)).toBe(0)
  })

  it('ignores Shift keydown autorepeat', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('Shift', { repeat: true }))
    expect(lineSegmentCount(preview)).toBe(0) // no lock, no tick
  })
})

describe('RotateTool — gesture', () => {
  // Deliberate contract change (selection-UX overhaul): an empty-selection
  // click no longer demands a prior Select step — the Viewport injects an
  // acquirer that picks the node under the cursor. The toast survives only
  // for a genuine miss (nothing under the cursor / no acquirer injected).
  it('empty selection with no acquirer (or a miss): toasts and stays idle', () => {
    const { tool, onToast } = makeTool({ selection: [] })
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(), rayThrough(0, 0))
    expect(onToast).toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false) // never advanced past idle

    const missTool = makeTool({ selection: [] })
    missTool.tool.setSelectionAcquirer(() => null)
    missTool.tool.onPointerDown(makeSnap(), rayThrough(0, 0))
    expect(missTool.onToast).toHaveBeenCalled()
    expect(missTool.tool.capturingInput()).toBe(false)
  })

  it('idle status hint matches the selection state (empty → "click the object")', () => {
    expect(makeTool({ selection: [] }).tool.statusHint())
      .toBe('Click the object you want to rotate.')
    expect(makeTool().tool.statusHint()).toContain('center of rotation')
  })

  it('empty selection auto-selects via the injected acquirer and starts the gesture in one click', () => {
    const { tool, onToast, wasmScene, onCommit } = makeTool({ selection: [] })
    const acquire = vi.fn(() => [{ kind: 'object', id: 5n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot + auto-select
    expect(acquire).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true) // advanced past idle in the same click

    // The rest of the gesture proceeds on the acquired node.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep 90°
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit
    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith([{ kind: 'object', id: 5n }])
  })

  it('commits a rotation from a full three-click gesture (pivot → reference → sweep)', () => {
    const { tool, wasmScene, onCommit } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, 0°)
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep to +Y (90°)
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit

    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false) // reset to idle after commit
  })

  it('ignores a reference point coincident with the pivot', () => {
    const { tool, onMeasurement, wasmScene } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // ref == pivot → ignored
    // Entering the sweep stage sets the '0.0°' readout; if the coincident click
    // was ignored we never entered it.
    expect(onMeasurement).not.toHaveBeenCalledWith('0.0°')

    // A usable reference then still works — exactly one rotation commits.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // real reference
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep 90°
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit
    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
  })
})

describe('RotateTool — snapConstraint (axis-lock plane)', () => {
  it('is absent while idle, and absent while a pivot/ref stage has no lock engaged', () => {
    const { tool } = makeTool()
    // Idle — no lock, no pivot.
    expect(tool.snapConstraint()).toBeNull()

    // Pivot placed, still unlocked: axis is merely inferred (ground Z), so
    // it must stay free — an inferred axis can change on the next hover.
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    expect(tool.snapConstraint()).toBeNull()

    // Ref stage, still unlocked.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    expect(tool.snapConstraint()).toBeNull()
  })

  it('is {pivot, lockedNormal} once an axis lock is engaged with a pivot placed', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 2, y: 3, z: 0 }), rayThrough(2, 3))
    tool.onPointerDown(makeSnap({ x: 2, y: 3, z: 0 }), rayThrough(2, 3)) // pivot at (2,3,0)
    tool.onKey(makeKeyEvent('ArrowRight')) // lock X

    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [2, 3, 0], normal: [1, 0, 0] },
    })

    // Still constrained once the reference is placed (ref stage).
    tool.onPointerDown(makeSnap({ x: 2, y: 3, z: 1 }), rayThrough(2, 3)) // reference (in the X-locked plane)
    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [2, 3, 0], normal: [1, 0, 0] },
    })
  })

  it('stores the reference point projected into the locked plane for an out-of-plane snap', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    tool.onKey(makeKeyEvent('ArrowRight')) // lock X — rotation plane is x=0 (Y-Z plane)

    // The resolved snap for the reference click carries a nonzero X (as if
    // an off-plane candidate slipped through despite the constraint) — the
    // tool must still store the reference point IN the x=0 plane, not at
    // this raw 3D point.
    tool.onPointerDown(makeSnap({ x: 5, y: 2, z: 3 }), rayThrough(2, 3))

    const stage = (tool as unknown as { stage: { kind: string; refPoint: [number, number, number] } }).stage
    expect(stage.kind).toBe('ref')
    expect(stage.refPoint).toEqual([0, 2, 3])
  })

  it('an arrow-lock change mid-gesture (ref stage) moves the constraint plane', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    tool.onKey(makeKeyEvent('ArrowRight')) // lock X
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(1, 0)) // reference, in-plane

    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [1, 0, 0] },
    })

    tool.onKey(makeKeyEvent('ArrowUp')) // relock to Z mid-sweep

    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [0, 0, 1] },
    })
  })
})

describe('RotateTool — the center-to-first-point dashed line', () => {
  /** The dashed line's [start, end] in the preview, or null. */
  function linkEnds(preview: THREE.Group): number[][] | null {
    let found: THREE.LineSegments | null = null
    preview.traverse((o) => {
      if (o instanceof THREE.LineSegments && o.material instanceof THREE.LineDashedMaterial) found = o
    })
    if (found === null) return null
    const pos = (found as THREE.LineSegments).geometry.getAttribute('position')
    return [[pos.getX(0), pos.getY(0), pos.getZ(0)], [pos.getX(1), pos.getY(1), pos.getZ(1)]]
  }

  it('locked, it ends where the click will put the first point: in the rotation plane', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    tool.onKey(makeKeyEvent('ArrowUp')) // lock Z
    // A snap off the rotation plane (z = 0.5): the click would project it.
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0.5, kind: 'endpoint' }), rayThrough(1, 0))
    const [start, end] = linkEnds(preview)!
    expect(start).toEqual([0, 0, 0])
    expect(end[0]).toBeCloseTo(1)
    expect(end[1]).toBeCloseTo(0)
    expect(end[2]).toBeCloseTo(0)
  })
})

describe('RotateTool — the center-to-first-point line with no snap', () => {
  it('is hidden while the pointer has no snap, and returns with the next one', () => {
    const { tool, preview } = makeTool()
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // center
    tool.onPointerMove(makeSnap({ x: 2, y: 1, z: 0 }), rayThrough(2, 1))
    let line: THREE.LineSegments | undefined
    preview.traverse((o) => { if (o instanceof THREE.LineSegments && o.material instanceof THREE.LineDashedMaterial) line = o })
    expect(line!.visible).toBe(true)
    tool.onPointerMove(null, rayThrough(5, 5)) // a ray with no snap (parallel to the plane)
    expect(line!.visible).toBe(false)
    tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))
    expect(line!.visible).toBe(true)
  })
})

describe('RotateTool — locked sweep consumes the constrained snap', () => {
  it('the live sweep target is the resolved snap point (not a raw ray∩plane recompute) once locked', () => {
    const { tool, onMeasurement, wasmScene } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    tool.onKey(makeKeyEvent('ArrowUp')) // lock Z — ground plane, matches the ray helper's plane
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, 0°)

    // Feed a snap whose (x, y, z) disagrees with what ray∩plane would compute
    // for this ray (the ray itself points straight through (0, 1, 0) on
    // z=0) — a snapped vertex sitting exactly at 90° but slightly off the
    // ray, as a sticky kernel candidate would be. The committed angle must
    // follow the SNAP, proving the tool consumed it rather than recomputing
    // ray∩plane and ignoring `snap`.
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0, kind: 'endpoint' }), rayThrough(0.3, 0.9))
    expect(onMeasurement).toHaveBeenCalledWith('Z 90.0°')

    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0, kind: 'endpoint' }), rayThrough(0.3, 0.9)) // commit
    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('unlocked rotation keeps pure ray∩plane intersection (snap.xyz is ignored)', () => {
    const { tool, onMeasurement } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot (unlocked, inferred Z)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, 0°)

    // The snap's own (x,y,z) says 45°, but the ray itself intersects the
    // ground plane at (0,1,0) → 90°. Unlocked must follow the ray, not the
    // snap payload.
    tool.onPointerMove(makeSnap({ x: 0.70710678, y: 0.70710678, z: 0 }), rayThrough(0, 1))
    // "Z " tags the readout because the default INFERRED axis (no face/edge
    // hit) is world +Z, same as the locked case below — the tag reflects
    // world-alignment, not lock state. The 90° (not 45°) is the assertion
    // that matters: it comes from ray∩plane, proving `snap.xyz` was ignored.
    expect(onMeasurement).toHaveBeenCalledWith('Z 90.0°')
  })
})

describe('RotateTool — screen-constant disk scaling', () => {
  // The disk's screen size at the app's reference fov/viewport (45°, 720px
  // tall) — carried over from the old DISK_SCREEN_K = 0.06 constant so the
  // migration doesn't change how big the protractor looks at that baseline.
  const REF_FOV_DEG = 45
  const REF_VIEWPORT_H = 720
  const tanHalf = (fovDeg: number) => Math.tan((fovDeg * Math.PI) / 360)
  const expectedScale = (dist: number, fovDeg: number, viewportH: number) => {
    const desiredPixels = (0.06 * REF_VIEWPORT_H) / tanHalf(REF_FOV_DEG)
    return (desiredPixels * dist * tanHalf(fovDeg)) / viewportH
  }
  /** `updateDiskScale` now takes a `worldPerPixel` callback (the CameraRig
   * form, docs/design/camera.md §1) instead of a raw `(camera, viewportH)`
   * pair — this is what a real Viewport would build from `rig.worldPerPixel`
   * for a perspective camera at the given fov/viewport. */
  const wppPerspective = (fovDeg: number, viewportH: number) => (dist: number) =>
    worldPerPixelPerspective(dist, fovDeg, viewportH)

  it('updateDiskScale matches the old DISK_SCREEN_K * dist size at the reference fov/viewport', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))
    const disk = diskGroup(preview)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(1, 2, 10) // 10 m straight up from the disk center

    tool.updateDiskScale(camera, wppPerspective(REF_FOV_DEG, REF_VIEWPORT_H))

    const dist = camera.position.distanceTo(disk.position)
    expect(dist).toBeCloseTo(10, 9)
    const expected = expectedScale(dist, REF_FOV_DEG, REF_VIEWPORT_H)
    expect(expected).toBeCloseTo(0.06 * dist, 9) // old K * dist, sanity cross-check
    expect(disk.scale.x).toBeCloseTo(expected, 9)
  })

  it('holds its on-screen size across a FOV change, unlike the old K * dist form', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)

    for (const fov of [20, 45, 70, 100]) {
      const camera = new THREE.PerspectiveCamera(fov)
      camera.position.set(0, 0, 10)
      tool.updateDiskScale(camera, wppPerspective(fov, REF_VIEWPORT_H))
      const dist = camera.position.distanceTo(disk.position)
      expect(disk.scale.x).toBeCloseTo(expectedScale(dist, fov, REF_VIEWPORT_H), 9)
    }
  })

  it('holds its on-screen size across a viewport resize, unlike the old K * dist form', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)
    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)

    for (const viewportH of [400, 720, 1200]) {
      tool.updateDiskScale(camera, wppPerspective(REF_FOV_DEG, viewportH))
      const dist = camera.position.distanceTo(disk.position)
      expect(disk.scale.x).toBeCloseTo(expectedScale(dist, REF_FOV_DEG, viewportH), 9)
    }
  })

  it('is a no-op only when no disk is shown — a degenerate worldPerPixel still yields a defined (zero) size rather than leaving the old scale untouched', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)
    // A degenerate (zero) viewport height's worldPerPixel is 0 (see
    // worldPerPixelPerspective) — the disk collapses to size 0 deterministically,
    // rather than the old guard's "leave the previous scale untouched" (which
    // could leave an arbitrary stale size on screen).
    tool.updateDiskScale(camera, wppPerspective(REF_FOV_DEG, 0))
    expect(disk.scale.x).toBe(0)
  })

  it('DESIGN CHANGE: works under an orthographic (parallel-projection) camera too — no longer silently hidden', () => {
    // Before Phase 1, an `instanceof PerspectiveCamera` guard made this a
    // no-op under ortho — the protractor disk would silently vanish the
    // moment the user toggled Parallel Projection. `worldPerPixel` is
    // supplied by the caller (CameraRig.worldPerPixel under the hood), so the
    // tool itself no longer has (or needs) any projection-specific branch.
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)

    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 10)
    ortho.position.set(0, 0, 10)
    const frustumHeight = 4 // top(1) - bottom(-1), pre-scaled for this test
    const wpp = (_dist: number) => worldPerPixelOrtho(frustumHeight, 1, REF_VIEWPORT_H)
    tool.updateDiskScale(ortho, wpp)

    const expected = (0.06 * REF_VIEWPORT_H) / tanHalf(REF_FOV_DEG) * wpp(0) / 2
    expect(disk.scale.x).toBeCloseTo(expected, 9)
    expect(disk.scale.x).toBeGreaterThan(0)
  })
})

describe('RotateTool — setEditContext aborts an armed gesture on a genuine change (component-edit-parity.md phase A2)', () => {
  it('previews and commits a selected definition member through its active instance', () => {
    const instance = 77n
    const member = 9n
    const group = new THREE.Group()
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1))
    mesh.name = `InstanceFace_${instance}_${member}_0`
    group.add(mesh)
    const { tool, preview, wasmScene } = makeTool({
      selection: [{ kind: 'object', id: member }],
      instance: { id: instance, group },
    })
    ;(wasmScene as unknown as { transform_def_selection: ReturnType<typeof vi.fn> })
      .transform_def_selection = vi.fn()
    tool.setEditContext({ kind: 'instance', id: instance, component: 90n })
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0))
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1))
    expect(preview.getObjectByName('InstanceMemberPreview')).toBeDefined()
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1))
    expect(
      (wasmScene as unknown as { transform_def_selection: ReturnType<typeof vi.fn> })
        .transform_def_selection,
    ).toHaveBeenCalledTimes(1)
  })

  it('a genuine context change cancels an armed pivot/reference gesture instead of silently retargeting its eventual commit', () => {
    const { tool } = makeTool()
    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'top' })

    expect(tool.capturingInput()).toBe(false)
  })

  it('re-pushing the SAME context is a no-op — an armed gesture survives it untouched', () => {
    const { tool } = makeTool()
    const ctx = { kind: 'instance' as const, id: 9n, component: 90n }
    tool.setEditContext(ctx)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })

    expect(tool.capturingInput()).toBe(true)
  })
})

/** Run pivot (origin) → reference (+X) → sweep to +Y (90°) → commit, the same
 * three-click shape as the "commits a rotation" test above. */
function runNinetyDegreeGesture(tool: RotateTool): void {
  tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
  tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
  tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, 0°)
  tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep to +Y (90°)
  tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit
}

describe('RotateTool — durable Alt copy toggle', () => {
  it('tapping Alt toggles copy mode on and off (not hold-to-copy)', () => {
    const { tool, onCopyModeChange } = makeTool()
    expect(tool.statusHint()).toContain('center of rotation')

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith(true)
    expect(tool.statusHint()).toContain('Copy is on')

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).toHaveBeenLastCalledWith(false)
    expect(tool.statusHint()).not.toContain('Copy is on')
  })

  it('ignores Alt autorepeat (a held Alt toggles exactly once)', () => {
    const { tool, onCopyModeChange } = makeTool()
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt', { repeat: true }))
    tool.onKey(makeKeyEvent('Alt', { repeat: true }))
    expect(onCopyModeChange).toHaveBeenCalledTimes(1)
    expect(onCopyModeChange).toHaveBeenLastCalledWith(true)
  })

  it('a full gesture commits a COPY (via duplicate_selection_array) while toggled on — Alt long released', () => {
    const { tool, wasmScene, onCommit } = makeTool()
    tool.onKey(makeKeyEvent('Alt')) // tap, release — durable
    runNinetyDegreeGesture(tool)

    expect(wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    const [kinds, ids, affine, count] = wasmScene.duplicate_selection_array.mock.calls[0]
    expect(Array.from(kinds as Uint8Array)).toEqual([0])
    expect(Array.from(ids as BigUint64Array)).toEqual([1n])
    expect(thetaOfZAffine(affine as Float64Array)).toBeCloseTo(Math.PI / 2)
    expect(count).toBe(1)
    expect(wasmScene.transform_selection).not.toHaveBeenCalled()
    // The fresh clone becomes the committed selection.
    expect(onCommit).toHaveBeenCalledWith([{ kind: 'object', id: 100n }])
  })

  it('a full gesture commits a plain ROTATE (transform_selection) after toggling back off', () => {
    const { tool, wasmScene } = makeTool()
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt')) // back off
    runNinetyDegreeGesture(tool)

    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(wasmScene.duplicate_selection_array).not.toHaveBeenCalled()
  })

  it('prefixes the readout with "Copy ·" while toggled on', () => {
    const { tool, onMeasurement } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    tool.onKey(makeKeyEvent('Alt'))
    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last.startsWith('Copy ·')).toBe(true)
  })
})

describe('RotateTool — copy modifier is platform-specific (Option on macOS, Ctrl elsewhere)', () => {
  it('toggleCopyMode() flips copy mode directly — the method a Windows/Linux Control clean tap calls', () => {
    const { tool, onCopyModeChange } = makeTool()
    tool.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith(true)
    expect(tool.statusHint()).toContain('Copy is on')
    tool.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith(false)
  })

  it('a bare Control keydown does NOT toggle copy mode on macOS — Option owns it here', () => {
    const { tool, onCopyModeChange } = makeTool()
    tool.onKey(makeKeyEvent('Control'))
    expect(onCopyModeChange).not.toHaveBeenCalled()
  })

  it('a Windows/Linux Control clean tap toggles copy mode via toggleCopyMode, exactly as the Viewport calls it', () => {
    const { tool, onCopyModeChange } = makeTool()
    const tap = new CleanModifierTap<{ toggleCopyMode(): void }>((key) => key === 'Control')

    tap.onKeyDown({ key: 'Control', repeat: false }, tool)
    const armed = tap.onKeyUp({ key: 'Control' }, tool)
    expect(armed).not.toBeNull()
    armed!.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith(true)
  })

  it('a Ctrl+Z chord does NOT toggle copy mode (the clean-tap combo guard)', () => {
    const { tool, onCopyModeChange } = makeTool()
    const tap = new CleanModifierTap<{ toggleCopyMode(): void }>((key) => key === 'Control')

    tap.onKeyDown({ key: 'Control', repeat: false }, tool)
    tap.onKeyDown({ key: 'z', repeat: false }, tool) // Ctrl+Z joins the press
    expect(tap.onKeyUp({ key: 'Control' }, tool)).toBeNull()
    expect(onCopyModeChange).not.toHaveBeenCalled()
  })
})

describe('RotateTool — on Windows/Linux, Alt is NOT the copy modifier (Ctrl is)', () => {
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
    const { RotateTool: RotateToolOnWindows } = await import('./RotateTool')
    const wasmScene = makeWasmScene()
    const onCopyModeChange = vi.fn()
    const tool = new RotateToolOnWindows(
      wasmScene as never,
      new THREE.Group(),
      null,
      [{ kind: 'object', id: 1n }],
      vi.fn(),
      vi.fn(),
      null,
      vi.fn(),
      onCopyModeChange,
      vi.fn(),
    )

    tool.onKey(makeKeyEvent('Alt'))
    expect(onCopyModeChange).not.toHaveBeenCalled()
    expect(tool.statusHint()).not.toContain('Copy is on')

    // The Viewport's Control clean tap calls this directly on this platform.
    tool.toggleCopyMode()
    expect(onCopyModeChange).toHaveBeenLastCalledWith(true)
    expect(tool.statusHint()).toContain('Copy is on')
    expect(tool.statusHint()).toContain('Ctrl')
  })
})

describe('RotateTool — drawn-shape copy (Move\'s "you can\'t Copy a Sketch" envelope)', () => {
  // `runNinetyDegreeGesture` rotates 90° about the origin on the inferred
  // (default) axis, world +Z — exactly normal to the ground plane the shapes
  // are drawn on, with the pivot ON that plane. That is the in-plane case
  // (spokes/petals/bolt-ring): the copy is ONE draft over the shape's edge
  // keys, never a detach onto a new sketch.
  it('an in-plane copy of a drawn unit goes through ONE draft (draft_copy_edges) with the 90° rotation affine', () => {
    const t = makeTool({ selection: [di(40n)] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    expect(t.wasmScene.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.draft_begin_ctx).toHaveBeenCalledWith(CTX, 'Copy')
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1)
    const [, keys, affine] = t.wasmScene.draft_copy_edges.mock.calls[0]
    expect([...(keys as BigUint64Array)]).toEqual([10n])
    expect(thetaOfZAffine(affine as Float64Array)).toBeCloseTo(Math.PI / 2, 9)
    expect(t.wasmScene.draft_end).toHaveBeenCalledTimes(1)
    // Never the sketch machinery.
    expect(t.wasmScene.sketch_begin_gesture).not.toHaveBeenCalled()
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled()
    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    // A COPY, not a rotate — and no object-duplicate call for a pure drawing
    // selection.
    expect(t.wasmScene.draft_transform_edges).not.toHaveBeenCalled()
    expect(t.wasmScene.transform_selection).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).not.toHaveBeenCalled()
    // The copy (found by geometry) becomes the committed selection.
    expect(t.onCommit).toHaveBeenCalledWith([di(78n)])
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('an in-plane copy of a WHOLE sketch REPLAYS into the SAME sketch id, with the geometry already rotated', () => {
    const t = makeTool({ selection: [{ kind: 'sketch', id: 3n }] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    // One gesture bracket on the SOURCE sketch — never a new one.
    expect(t.wasmScene.sketch_begin_gesture).toHaveBeenCalledWith(3n)
    expect(t.wasmScene.sketch_end_gesture).toHaveBeenCalledWith(3n)
    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    expect(t.wasmScene.draft_begin_ctx).not.toHaveBeenCalled()

    // The edge (0,0,0)-(1,0,0) rotated 90° about Z through the origin lands
    // at (0,0,0)-(0,1,0) — the geometry itself arrives already rotated, not
    // merely offset. (Sketch 3's other island replays after it.)
    expect(t.wasmScene.sketch_add_segment).toHaveBeenCalledTimes(2)
    const [sketch, ax, ay, az, bx, by, bz] = t.wasmScene.sketch_add_segment.mock.calls[0]
    expect(sketch).toBe(3n)
    expect(ax).toBeCloseTo(0, 9)
    expect(ay).toBeCloseTo(0, 9)
    expect(az).toBeCloseTo(0, 9)
    expect(bx).toBeCloseTo(0, 9)
    expect(by).toBeCloseTo(1, 9)
    expect(bz).toBeCloseTo(0, 9)

    expect(t.wasmScene.transform_selection).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).not.toHaveBeenCalled()
    // The replayed unit becomes the committed selection.
    expect(t.onCommit).toHaveBeenCalledWith([di(78n)])
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('an in-plane copy of a drawn-circle unit hands the kernel the circle\'s edge keys and the rigid rotation (curve identity is the draft\'s job)', () => {
    const t = makeTool({ selection: [di(45n)] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1)
    const [, keys, affine] = t.wasmScene.draft_copy_edges.mock.calls[0]
    expect([...(keys as BigUint64Array)]).toEqual([30n])
    // A pure rotation: Curve 99's center (2,0,0) goes to (0,2,0) under it and
    // the radius is untouched — the draft carries the analytic circle.
    const c = applyAffineToPoint(affine as Float64Array, [2, 0, 0])
    expect(c[0]).toBeCloseTo(0, 9)
    expect(c[1]).toBeCloseTo(2, 9)
    expect(t.wasmScene.sketch_begin_curve_with).not.toHaveBeenCalled()
    expect(t.onCommit).toHaveBeenCalledWith([di(78n)])
  })

  it('an in-plane copy of a WHOLE drawn-circle sketch keeps curve identity: rotated center, same radius', () => {
    const t = makeTool({ selection: [{ kind: 'sketch', id: 6n }] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    // Curve 99's center (2,0,0), radius 1, rotated 90° about Z through the
    // origin: the center moves to (0,2,0), the radius survives untouched —
    // a rotated circle stays a true circle, not just rotated facets.
    expect(t.wasmScene.sketch_begin_curve_with).toHaveBeenCalledTimes(1)
    const [sketch, cx, cy, cz, radius] = t.wasmScene.sketch_begin_curve_with.mock.calls[0]
    expect(sketch).toBe(6n)
    expect(cx).toBeCloseTo(0, 9)
    expect(cy).toBeCloseTo(2, 9)
    expect(cz).toBeCloseTo(0, 9)
    expect(radius).toBe(1) // untouched — not recomputed, just carried through
    expect(t.wasmScene.sketch_begin_curve).not.toHaveBeenCalled() // it has geom — not an identity-only chain
    expect(t.onCommit).toHaveBeenCalledWith([di(78n)])
  })

  /** Pivot at the origin, lock Y (an in-plane axis), sweep toward +Z. */
  function sweepAboutY(t: ReturnType<typeof makeTool>): void {
    t.tool.onKey(makeKeyEvent('Alt'))
    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    t.tool.onKey(makeKeyEvent('ArrowLeft')) // lock Y — in the sketch's own plane, NOT its normal
    t.tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, on the Y-locked plane)
    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0)) // sweep toward +Z (~90° about Y)
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0)) // commit
  }

  it('an out-of-plane rotation (axis not normal to the plane) of a drawn unit still lands a new sketch (detach copy)', () => {
    const t = makeTool({ selection: [di(40n)] })
    sweepAboutY(t)

    expect(t.wasmScene.draft_begin_ctx).not.toHaveBeenCalled()
    expect(t.wasmScene.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [sketch, islands] = t.wasmScene.copy_sketch_islands.mock.calls[0]
    expect(sketch).toBe(3n)
    expect(Array.from(islands as BigUint64Array)).toEqual([40n])
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled() // never a same-sketch replay
    expect(t.onCommit).toHaveBeenCalledWith([di(77n)])
  })

  it('an out-of-plane rotation of a WHOLE sketch still lands a new sketch', () => {
    const t = makeTool({ selection: [{ kind: 'sketch', id: 3n }] })
    sweepAboutY(t)

    expect(t.wasmScene.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [sketch, islands] = t.wasmScene.copy_sketch_islands.mock.calls[0]
    expect(sketch).toBe(3n)
    expect(Array.from(islands as BigUint64Array)).toEqual([40n, 41n])
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled()
    expect(t.onCommit).toHaveBeenCalledWith([di(77n)])
  })

  /** Pivot at the origin, lock X (in-plane), sweep +Y to −Y: 180°. */
  function flipAboutX(t: ReturnType<typeof makeTool>): void {
    t.tool.onKey(makeKeyEvent('Alt'))
    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    t.tool.onKey(makeKeyEvent('ArrowRight')) // lock X — in the sketch's own plane
    t.tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(1, 0)) // reference (+Y, on the X-locked plane)
    t.tool.onPointerMove(makeSnap({ x: 0, y: -1, z: 0 }), rayThrough(0, -1)) // sweep to −Y: 180°
    t.tool.onPointerDown(makeSnap({ x: 0, y: -1, z: 0 }), rayThrough(0, -1)) // commit
  }

  it('an orientation-flipping rotation (180° about an in-plane axis) of a WHOLE sketch routes to the new-sketch arm, never a reflective in-plane replay', () => {
    const t = makeTool({ selection: [{ kind: 'sketch', id: 3n }] })
    flipAboutX(t)

    expect(t.wasmScene.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [sketch] = t.wasmScene.copy_sketch_islands.mock.calls[0]
    expect(sketch).toBe(3n)
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled() // never a reflective in-plane replay
    // The flipped copy of edge 10 lands exactly back on the original (the
    // axis runs along it), so the fixture's by-geometry lookup may name
    // either listed unit: just one drawn-unit ref comes back.
    const [committed] = t.onCommit.mock.calls[0] as [NodeRef[]]
    expect(committed).toHaveLength(1)
    expect(committed[0].kind).toBe('draft-island')
  })

  it('a 180° flip about an in-plane axis of a DRAWN unit stays in the plane: ONE draft copy (the draft re-derives regions from edges), no detach', () => {
    // FLIPPED from the sketch-era expectation (new-sketch arm): a drawn unit
    // lands every endpoint back on its plane under this affine, and the
    // draft re-derives its regions from the copied edges, so the reflective
    // winding problem of the sketch replay does not exist for it.
    const t = makeTool({ selection: [di(40n)] })
    flipAboutX(t)

    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1)
    expect([...(t.wasmScene.draft_copy_edges.mock.calls[0][1] as BigUint64Array)]).toEqual([10n])
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled()
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('a WHOLE-sketch copy ARMS the ×N array window: typing 5x retracts the single-copy commit with ONE scene undo, then replays 5 cumulative copies into the SAME sketch inside a SECOND (still singular) gesture bracket', () => {
    const t = makeTool({ selection: [{ kind: 'sketch', id: 6n }] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    // The initial commit: one copy, one gesture bracket, one segment.
    expect(t.wasmScene.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.sketch_end_gesture).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.sketch_add_segment).toHaveBeenCalledTimes(1)

    typeKeys(t.tool, 'x5')
    t.tool.onKey(makeKeyEvent('Enter'))

    // Retracts the single-copy commit (it recorded exactly ONE history
    // entry) with exactly ONE scene_undo, then replays 5 cumulative copies
    // into the SOURCE sketch inside ONE further gesture bracket — never a
    // new sketch, never one bracket per rep. A single further scene_undo
    // (the WHOLE-array undo step) would retract every rep, matching the
    // object array's undo shape.
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.sketch_begin_gesture).toHaveBeenCalledTimes(2)
    expect(t.wasmScene.sketch_end_gesture).toHaveBeenCalledTimes(2)
    expect(t.wasmScene.sketch_add_segment).toHaveBeenCalledTimes(1 + 5)
    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).not.toHaveBeenCalled()
    for (const call of t.wasmScene.sketch_add_segment.mock.calls) {
      expect(call[0]).toBe(6n) // every rep lands on the SOURCE sketch
    }
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
  })

  it('a drawn-unit copy ARMS the ×N array window: typing 5x retracts the single-copy draft with ONE scene undo, then lands 5 cumulative copies inside ONE draft (one undo step for the whole array)', () => {
    const t = makeTool({ selection: [di(40n)] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    expect(t.wasmScene.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.draft_end).toHaveBeenCalledTimes(1)

    typeKeys(t.tool, 'x5')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1) // the one single-copy draft
    expect(t.wasmScene.draft_begin_ctx).toHaveBeenCalledTimes(1 + 1) // the single copy + ONE array draft
    expect(t.wasmScene.draft_end).toHaveBeenCalledTimes(1 + 1)
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1 + 5) // one copy per rep inside it
    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).not.toHaveBeenCalled()
    // Each rep carries its OWN cumulative angle: 90°, 180°, 270°, 360°, 450°.
    const angles = t.wasmScene.draft_copy_edges.mock.calls
      .slice(1)
      .map((c) => thetaOfZAffine(c[2] as Float64Array))
    expect(angles[0]).toBeCloseTo(Math.PI / 2, 6)
    expect(angles[1]).toBeCloseTo(Math.PI, 6)
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
  })

  it('a mixed selection duplicates objects AND copies drawn shapes, and BOTH array together when ×N is typed (ONE step each time)', () => {
    const t = makeTool({
      selection: [
        { kind: 'object', id: 1n },
        di(40n),
      ],
    })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    // The default gesture (Z axis, pivot at the origin) is in-plane for the
    // ground drawing — the drawn half copies through one draft, it does not
    // detach onto a new sketch.
    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenCalledWith([
      di(78n),
      { kind: 'object', id: 100n },
    ])

    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))

    // The initial commit is ONE history entry (draft copy + object array in a
    // bracket; was two) — one undo retracts it before the array reissues at
    // count 3, again as one entry.
    expect(t.wasmScene.brackets.labels).toEqual(['Copy', 'Copy'])
    expect(t.wasmScene.undo_depth()).toBe(1)
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1 + 3)
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
  })
})

describe('RotateTool — ×N / /N array copy', () => {
  /** Tap Alt, rotate one selected object 90° about the origin via a full
   * gesture — the copy commit that arms the array refinement. */
  function commitOneCopy(t: ReturnType<typeof makeTool>): void {
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
  }

  it('teaches the refinement in the status hint after a copy commits (SketchUp 3x form first)', () => {
    const t = makeTool()
    commitOneCopy(t)
    expect(t.tool.statusHint()).toContain('3x')
    expect(t.tool.statusHint()).toContain('3/')
  })

  it('the SketchUp trailing form 3x + Enter resolves exactly like x3, at the SAME angular step', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, '3x')
    // The trailing form's leading digit is buffer input, and it reads back
    // with the display glyph: "3×".
    expect(t.onMeasurement).toHaveBeenLastCalledWith('3×')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1)
    const [, ids, affine, count] = t.wasmScene.duplicate_selection_array.mock.calls[1]
    expect(Array.from(ids as BigUint64Array)).toEqual([1n]) // the ORIGINAL source
    expect(thetaOfZAffine(affine as Float64Array)).toBeCloseTo(Math.PI / 2)
    expect(count).toBe(3)
  })

  it('the leading form x3 resolves the same way (both token orders produce the same step)', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))
    const call = t.wasmScene.duplicate_selection_array.mock.calls.at(-1)!
    expect(thetaOfZAffine(call[2] as Float64Array)).toBeCloseTo(Math.PI / 2)
    expect(call[3]).toBe(3)
  })

  it('the trailing divide form 4/ + Enter divides the swept angle across the copies', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, '4/')
    t.tool.onKey(makeKeyEvent('Enter'))
    const call = t.wasmScene.duplicate_selection_array.mock.calls.at(-1)!
    expect(thetaOfZAffine(call[2] as Float64Array)).toBeCloseTo(Math.PI / 2 / 4)
    expect(call[3]).toBe(4)
  })

  it('the leading divide form /4 resolves the same way (both token orders)', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, '/4')
    t.tool.onKey(makeKeyEvent('Enter'))
    const call = t.wasmScene.duplicate_selection_array.mock.calls.at(-1)!
    expect(thetaOfZAffine(call[2] as Float64Array)).toBeCloseTo(Math.PI / 2 / 4)
    expect(call[3]).toBe(4)
  })

  it('re-entering a different count while hot retracts the previous array with ONE undo and replaces it', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, 'x3')
    expect(t.tool.capturingInput()).toBe(true) // digits must not switch tools
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    // All three clones become the selection via the full-refresh commit path.
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
    expect((t.onArrayCommit.mock.calls[0][0] as NodeRef[]).length).toBe(3)

    typeKeys(t.tool, '/5')
    t.tool.onKey(makeKeyEvent('Enter'))
    // A SECOND refinement retracts with exactly one more undo — never one
    // undo per element of the array being replaced.
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(2)
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(3)
    const call = t.wasmScene.duplicate_selection_array.mock.calls.at(-1)!
    expect(call[3]).toBe(5)
    expect(thetaOfZAffine(call[2] as Float64Array)).toBeCloseTo(Math.PI / 2 / 5)
    expect(t.onArrayCommit).toHaveBeenCalledTimes(2)
  })

  it('a refused refinement restores the retracted copies with redo and keeps the window hot', () => {
    const t = makeTool()
    commitOneCopy(t)

    t.wasmScene.duplicate_selection_array.mockImplementationOnce(() => {
      throw new Error('Transform: refused')
    })
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.scene_redo).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalled()

    // The recovery undo+redo moved the history generation; the window
    // re-stamped its token, so a fresh count still resolves.
    typeKeys(t.tool, 'x2')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(2)
    const call = t.wasmScene.duplicate_selection_array.mock.calls.at(-1)!
    expect(call[3]).toBe(2)
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
  })

  it('refuses the refinement when the HISTORY moved since the copy (history-generation guard)', () => {
    const t = makeTool()
    commitOneCopy(t)

    // Force the generation on so the window's stamped token is stale — the
    // same guard MoveTool relies on to survive a net-zero content edit.
    t.wasmScene.transform_selection() // an unrelated recorded action
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))

    expect(t.wasmScene.scene_undo).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalledWith(expect.stringContaining('the model changed'))
  })

  it('refuses a count above the kernel cap with a toast, before any undo fires', () => {
    const t = makeTool()
    commitOneCopy(t)

    typeKeys(t.tool, 'x1001')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).toHaveBeenCalledWith(expect.stringContaining('1000'))
    expect(t.wasmScene.scene_undo).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.max_array_count).toHaveBeenCalled()
  })

  it('disarmArray (explicit delete/undo/redo commands) closes the window cleanly', () => {
    const t = makeTool()
    commitOneCopy(t)
    expect(t.tool.capturingInput()).toBe(true)

    t.tool.disarmArray()
    expect(t.tool.capturingInput()).toBe(false)

    // A later x3 + Enter does nothing — no wrong-action undo, no second
    // array, no toast spam.
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_undo).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.onToast).not.toHaveBeenCalled()
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

  it('a mid-gesture VCB (REF stage) still captures the whole keyboard, mirroring MoveTool\'s base stage', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference — now sweeping (REF)

    for (const key of ['5', '.', '-', 'Backspace', 'Enter', 'q', ' ']) {
      expect(tool.capturesKey(key), `REF stage must capture ${JSON.stringify(key)}`).toBe(true)
    }
  })

  it('Space exit is quiet: the tool-switch cancel disarms without undoing the copies', () => {
    const t = makeTool()
    commitOneCopy(t)
    typeKeys(t.tool, '5') // even with a partial buffer typed
    expect(t.tool.capturesKey(' ')).toBe(false)

    // What the Viewport does on the fall-through: switch tools, which
    // cancels the outgoing RotateTool.
    t.tool.cancel()

    expect(t.tool.capturingInput()).toBe(false)
    // The committed copy is untouched — no retraction, no toast.
    expect(t.wasmScene.scene_undo).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.onToast).not.toHaveBeenCalled()
    // A stray Enter afterwards is inert.
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_undo).not.toHaveBeenCalled()
  })

  it('the PIVOT stage lets tool-switch shortcuts fire but still guards Delete/Backspace (no angle to type, but a live gesture to protect)', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot only — no reference yet
    expect(tool.capturingInput()).toBe(true) // still mid-gesture

    // No numeric VCB is live yet (there is no reference to measure an angle
    // from), so tool-switch letters/digits are free to fall through to the
    // Viewport's shortcut routing instead of being silently eaten.
    for (const key of ['3', 'm', 'q', 'r', '5', ' ']) {
      expect(tool.capturesKey(key), `PIVOT stage must not capture ${JSON.stringify(key)}`).toBe(false)
    }

    // Delete/Backspace ARE captured in this stage regardless — the real
    // routing path the Viewport/App actually consult (`isCapturingInput`,
    // Viewport.tsx ~2191-2200) prioritizes `capturesKey` over
    // `capturingInput()` once a key is given, so `capturingInput()` alone
    // (asserted above by the round-1 fix) is not the predicate that guards
    // Delete here — `capturesKey` is, and it must name these two keys
    // explicitly or a bare Delete with a pivot placed falls through to
    // App's edit-delete and destroys the node mid-gesture (see App.tsx's
    // Delete handler and MoveTool's identical Delete-guard doc comment).
    expect(tool.capturesKey('Delete'), 'PIVOT stage must capture Delete').toBe(true)
    expect(tool.capturesKey('Backspace'), 'PIVOT stage must capture Backspace').toBe(true)

    // Digits/letters/Delete reaching onKey anyway (e.g. via the Viewport's
    // uncaptured-key fallback, or because capturesKey routed it there) are
    // harmlessly ignored in this stage — no crash, no stray measurement, no
    // deletion (the stage has no VCB and no delete action of its own).
    tool.onKey(makeKeyEvent('3'))
    expect(tool.capturingInput()).toBe(true) // still just the pivot, unchanged
    tool.onKey(makeKeyEvent('Delete'))
    expect(tool.capturingInput()).toBe(true) // pivot untouched — no delete happened inside the tool

    // Once the reference is placed (REF stage), the same keys DO capture.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0))
    expect(tool.capturesKey('3')).toBe(true)
  })

  it('a bare Delete keystroke mid-pivot never reaches the app-level delete handler (Viewport.isCapturingInput\'s real per-key routing)', () => {
    // Mirrors Viewport.tsx's `isCapturingInput` (~2191-2200) exactly: with a
    // key given, a tool with `capturesKey` is asked THAT, not
    // `capturingInput()` — this is the actual predicate App.tsx's Delete
    // handler consults, and the one the round-1 fix's test got wrong.
    function isCapturingInputLike(t: RotateTool, key: string): boolean {
      return t.capturesKey(key)
    }

    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot placed, mid-gesture

    // What the keymap dispatcher (keymap/dispatch.ts) does: skip Delete's
    // command when isCapturingInput(ev.key) is true. It must be true here, or the
    // pivoted node gets deleted out from under the live gesture.
    expect(isCapturingInputLike(tool, 'Delete')).toBe(true)
    expect(isCapturingInputLike(tool, 'Backspace')).toBe(true)
  })

  it('a new gesture (new pivot click) ends the window entirely', () => {
    const t = makeTool()
    commitOneCopy(t)

    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // new pivot
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Escape')) // cancel the new gesture
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_undo).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
  })
})

// Post-commit angle retype (retypeWindow.ts): type degrees after a rotation
// commits and it is redone about the same pivot and axis.
describe('RotateTool — retype the angle after the commit', () => {
  const lastTheta = (t: ReturnType<typeof makeTool>) => {
    const calls = (t.wasmScene.transform_selection as ReturnType<typeof vi.fn>).mock.calls
    return thetaOfZAffine(calls[calls.length - 1][3] as Float64Array)
  }

  it('a 90° rotation redone at 45° (one undo), then flipped with -30°', () => {
    const t = makeTool()
    runNinetyDegreeGesture(t.tool)
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(lastTheta(t)).toBeCloseTo(Math.PI / 2, 6)
    expect(t.tool.statusHint()).toContain('redo the rotation')
    expect(t.tool.capturesKey('4')).toBe(true)
    expect(t.tool.capturesKey('q')).toBe(false)

    typeKeys(t.tool, '45')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(2)
    expect(lastTheta(t)).toBeCloseTo(Math.PI / 4, 6)

    typeKeys(t.tool, '-30')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(2)
    expect(lastTheta(t)).toBeCloseTo(-Math.PI / 6, 6)
  })

  it('Escape closes the window and a new pivot click starts fresh', () => {
    const t = makeTool()
    runNinetyDegreeGesture(t.tool)
    typeKeys(t.tool, '4')
    t.tool.onKey(makeKeyEvent('Escape'))
    expect(t.tool.capturesKey('4')).toBe(false)
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    expect(t.tool.capturingInput()).toBe(true)
  })
})

describe('RotateTool — retype after an arranging drawn-shape rotation', () => {
  it('re-commits with the PRE-commit shape refs (the undo restores the original keys), while the visible selection is the re-resolved unit', () => {
    const t = makeTool({ selection: [di(40n)] })
    const scene = t.wasmScene as unknown as Record<string, ReturnType<typeof vi.fn>> & {
      drawing: { units: DUnit[] }
    }
    const baseUndo = scene.scene_undo.getMockImplementation() as () => unknown
    const original = scene.drawing.units.map((u) => ({
      ...u,
      edges: u.edges.map((e) => ({ ...e, a: [...e.a] as V3, b: [...e.b] as V3 })),
    }))
    // EVERY landing arranges: the rotated edge re-keys into fresh unit 99.
    const applyImpl = scene.draft_transform_edges.getMockImplementation() as (...a: unknown[]) => void
    scene.draft_transform_edges.mockImplementation((...args: unknown[]) => {
      applyImpl(...args)
      const moved = scene.drawing.units[0].edges[0]
      scene.drawing.units = [{ key: 99n, sketch: 3, edges: [{ key: 60n, a: moved.a, b: moved.b }] }, scene.drawing.units[1], scene.drawing.units[2]]
    })
    scene.scene_undo.mockImplementation(() => {
      scene.drawing.units = original.map((u) => ({
        ...u,
        edges: u.edges.map((e) => ({ ...e, a: [...e.a] as V3, b: [...e.b] as V3 })),
      }))
      return baseUndo()
    })

    runNinetyDegreeGesture(t.tool)
    expect(scene.draft_transform_edges).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenLastCalledWith([di(99n)])

    typeKeys(t.tool, '45')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(scene.draft_transform_edges).toHaveBeenCalledTimes(2)
    // Both commits name the ORIGINAL unit's edge, never the dead merged key 60.
    expect([...(scene.draft_transform_edges.mock.calls[0][1] as BigUint64Array)]).toEqual([10n])
    expect([...(scene.draft_transform_edges.mock.calls[1][1] as BigUint64Array)]).toEqual([10n])
    expect(thetaOfZAffine(scene.draft_transform_edges.mock.calls[1][2] as Float64Array)).toBeCloseTo(Math.PI / 4, 6)
    expect(t.onCommit).toHaveBeenLastCalledWith([di(99n)])
  })
})

describe('RotateTool — a typed angle after an array re-fans the array', () => {
  it('copy 90°, then 3x, then 45 → three copies fanned by 45°', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    const dup = t.wasmScene.duplicate_selection_array as ReturnType<typeof vi.fn>
    expect(dup).toHaveBeenCalledTimes(1)
    typeKeys(t.tool, '3x'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(dup).toHaveBeenCalledTimes(2)
    expect(dup.mock.calls[1][3]).toBe(3)
    expect(thetaOfZAffine(dup.mock.calls[1][2] as Float64Array)).toBeCloseTo(Math.PI / 2, 6)

    typeKeys(t.tool, '45'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(dup).toHaveBeenCalledTimes(3)
    expect(dup.mock.calls[2][3]).toBe(3)
    expect(thetaOfZAffine(dup.mock.calls[2][2] as Float64Array)).toBeCloseTo(Math.PI / 4, 6)

    typeKeys(t.tool, '2x'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(dup.mock.calls[3][3]).toBe(2)
    expect(thetaOfZAffine(dup.mock.calls[3][2] as Float64Array)).toBeCloseTo(Math.PI / 4, 6)
  })
})

describe('RotateTool — a refused mixed array resolve is atomic', () => {
  it('aborts the attempt (drawn half included) and redoes the previous copy', () => {
    const t = makeTool({ selection: [{ kind: 'object', id: 1n }, di(40n)] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)

    t.wasmScene.duplicate_selection_array.mockImplementationOnce(() => {
      throw new Error('Transform: refused')
    })
    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))

    // One undo retracted the previous commit; the refused attempt's bracket
    // aborted (restoring document and redo stack exactly), so ONE redo
    // brings the previous copy back — nothing is re-laid by hand.
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.scene_redo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.brackets.aborted).toBe(1)
    expect(t.wasmScene.draft_copy_edges).toHaveBeenCalledTimes(1 + 3) // first copy + the aborted attempt's 3 reps
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    expect(t.wasmScene.undo_depth()).toBe(1)
    expect(t.onToast).toHaveBeenCalled()
  })
})

describe('RotateTool — array history edge cases', () => {
  it('a refused xN that recorded nothing keeps the typed-angle window armed', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    t.wasmScene.duplicate_selection_array.mockImplementationOnce(() => { throw new Error('Transform: refused') })
    typeKeys(t.tool, 'x3'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.scene_redo).toHaveBeenCalledTimes(1)
    t.onToast.mockClear()
    typeKeys(t.tool, '45'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).not.toHaveBeenCalled()
    const call = t.wasmScene.duplicate_selection_array.mock.calls.at(-1)!
    expect(thetaOfZAffine(call[2] as Float64Array)).toBeCloseTo(Math.PI / 4, 6)
  })

  it('a refused relay that recorded nothing re-stamps the array window so the next xN is accepted', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    typeKeys(t.tool, '3x'); t.tool.onKey(makeKeyEvent('Enter'))
    t.wasmScene.duplicate_selection_array.mockImplementationOnce(() => { throw new Error('Transform: refused') })
    typeKeys(t.tool, '45'); t.tool.onKey(makeKeyEvent('Enter')) // relay refused
    t.onToast.mockClear()
    typeKeys(t.tool, '5x'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).not.toHaveBeenCalledWith(expect.stringContaining('Array entry ended'), undefined)
    expect(t.wasmScene.duplicate_selection_array.mock.calls.at(-1)![3]).toBe(5)
  })

  it('the drawn half succeeding and the object half refusing during a relay nets out', () => {
    const t = makeTool({ selection: [{ kind: 'object', id: 1n }, di(40n)] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    typeKeys(t.tool, '2x'); t.tool.onKey(makeKeyEvent('Enter'))
    const depth = t.wasmScene.undo_depth()
    t.wasmScene.duplicate_selection_array.mockImplementationOnce(() => { throw new Error('Transform: refused') })
    typeKeys(t.tool, '45'); t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.undo_depth()).toBe(depth)
  })
})

describe('RotateTool — one gesture, one undo step (history bracket)', () => {
  it('rotating an object together with drawing records ONE undo step labeled Rotate', () => {
    const t = makeTool({ selection: [{ kind: 'object', id: 1n }, di(40n), di(41n)] })
    runNinetyDegreeGesture(t.tool)
    expect(t.wasmScene.draft_end).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.undo_depth()).toBe(1)
    expect(t.wasmScene.brackets.labels).toEqual(['Rotate'])
  })
})

// Click-drag-release (#52b, viewport/dragCommit.ts): the ×N array window keeps
// `capturingInput()` true at idle, so the gate reads `gestureInFlight()`.
describe('RotateTool — click-drag-release gate', () => {
  it('a press during the ×N window arms a drag that sets the reference; a mid-gesture press never arms', () => {
    const t = makeTool()
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    expect(t.tool.capturingInput()).toBe(true) // the array window
    expect(t.tool.gestureInFlight()).toBe(false)

    const gate = new DragCommitGate()
    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    expect(gate.press(t.tool, 1, 100, 100, () => t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)))).toBe(true)
    expect(new DragCommitGate().press(t.tool, 2, 0, 0, () => { /* not delivered */ })).toBe(false)
    gate.move(1, 160, 100)
    expect(gate.release(t.tool, 1, 160, 100, () => t.tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)))).toBe(true)
    // The release set the reference: the tool now asks for the angle.
    expect(t.tool.gestureInFlight()).toBe(true)
    expect(t.tool.statusHint()).not.toContain('center of rotation')
  })
})

describe('RotateTool — the drag ghost is session geometry, mirrored onto sibling placements', () => {
  it('the reference click puts a marked ghost in the preview (the protractor disk is not one); the mirror shows it and clears with Esc', () => {
    const objectsGroup = new THREE.Group()
    const sub = new THREE.Group()
    sub.name = 'Object_1'
    sub.add(new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial()))
    objectsGroup.add(sub)
    const preview = new THREE.Group()
    const wasmScene = makeWasmScene()
    const tool = new RotateTool(wasmScene as never, preview, objectsGroup, [{ kind: 'object', id: 1n }], vi.fn(), vi.fn(), null, vi.fn(), vi.fn(), vi.fn())
    const root = new THREE.Group()
    const mirror = new SiblingPreviewMirror({ source: preview, root, frames: () => [1, 0, 0, 4, 0, 1, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 7, 0, 0, 1, 0] })

    tool.onPointerDown(makeSnap(), rayThrough(0, 0)) // pivot
    expect(preview.children.filter(isSessionGhost)).toHaveLength(0) // disk only
    mirror.sync()
    expect(mirror.mirrorCount).toBe(0)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference arm: the ghost appears
    const ghosts = preview.children.filter(isSessionGhost)
    expect(ghosts).toHaveLength(1)
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)
    expect((root.children[0] as THREE.Group).children).toHaveLength(1) // the ghost, not the disk

    tool.onKey(makeKeyEvent('Escape'))
    mirror.sync()
    expect(mirror.mirrorCount).toBe(0)
  })
})

describe('RotateTool — a click-taken selection lasts only for the gesture (as Move)', () => {
  const NODE: NodeRef = { kind: 'object', id: 7n }

  function makeAcquiring() {
    const t = makeTool({ selection: [] })
    const acquire = vi.fn(() => [NODE])
    const release = vi.fn()
    t.tool.setSelectionAcquirer(acquire, release)
    return { ...t, acquire, release }
  }

  it('a landed rotation empties the selection again', () => {
    const t = makeAcquiring()
    runNinetyDegreeGesture(t.tool)
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(t.release).toHaveBeenCalledTimes(1)
  })

  it('Esc mid-gesture gives the selection back', () => {
    const t = makeAcquiring()
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    t.tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    t.tool.onKey(makeKeyEvent('Escape'))
    expect(t.release).toHaveBeenCalledTimes(1)
  })

  it('a refused rotation gives the selection back', () => {
    const t = makeAcquiring()
    t.wasmScene.transform_selection.mockImplementationOnce(() => { throw new Error('InvalidTransform: refused') })
    runNinetyDegreeGesture(t.tool)
    expect(t.onToast).toHaveBeenCalled()
    expect(t.release).toHaveBeenCalledTimes(1)
  })

  it('a refused retype leaves the gesture\'s release to the retype that lands', () => {
    const t = makeAcquiring()
    runNinetyDegreeGesture(t.tool)
    expect(t.release).toHaveBeenCalledTimes(1)

    t.wasmScene.transform_selection.mockImplementationOnce(() => { throw new Error('InvalidTransform: refused') })
    typeKeys(t.tool, '45')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.onToast).toHaveBeenCalled()
    expect(t.release).toHaveBeenCalledTimes(1)

    typeKeys(t.tool, '30')
    t.tool.onKey(makeKeyEvent('Enter'))
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(3)
    expect(t.release).toHaveBeenCalledTimes(2)
  })

  it('a copy and its ×N array leave nothing selected — the array still fans from the source', () => {
    const t = makeAcquiring()
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)
    expect(t.release).toHaveBeenCalledTimes(1)
    typeKeys(t.tool, '3x')
    t.tool.onKey(makeKeyEvent('Enter'))
    const [, ids, , count] = t.wasmScene.duplicate_selection_array.mock.calls.at(-1)!
    expect(Array.from(ids as BigUint64Array)).toEqual([7n])
    expect(count).toBe(3)
    expect(t.release).toHaveBeenCalledTimes(2)
  })

  it('a selection made beforehand is kept', () => {
    const t = makeTool()
    const release = vi.fn()
    t.tool.setSelectionAcquirer(() => [NODE], release)
    runNinetyDegreeGesture(t.tool)
    expect(release).not.toHaveBeenCalled()
  })
})

describe('RotateTool — what a click would take (clickAcquire, the hover highlight)', () => {
  it('idle with nothing selected: a guide first when guides can be taken, else a node', () => {
    const t = makeTool({ selection: [] })
    t.tool.setSelectionAcquirer(() => null)
    expect(t.tool.clickAcquire()).toEqual({ guides: false, nodes: true })
    t.tool.setGuideHooks(() => null, vi.fn())
    expect(t.tool.clickAcquire()).toEqual({ guides: true, nodes: true })
  })

  it('nothing with nodes or a guide selected (the click is the pivot), or mid-gesture', () => {
    const selected = makeTool()
    selected.tool.setSelectionAcquirer(() => null)
    expect(selected.tool.clickAcquire()).toBeNull()

    const guide = makeTool({ selection: [] })
    guide.tool.setSelectionAcquirer(() => null)
    guide.tool.setGuideHooks(() => null, vi.fn())
    guide.tool.setGuideTarget(5n)
    expect(guide.tool.clickAcquire()).toBeNull()

    const t = makeTool({ selection: [] })
    t.tool.setSelectionAcquirer(() => [{ kind: 'object', id: 7n }])
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    expect(t.tool.clickAcquire()).toBeNull()
  })
})

describe('RotateTool — dashed line from the center to the first point', () => {
  function dashedLine(preview: THREE.Object3D): THREE.LineSegments | undefined {
    let found: THREE.LineSegments | undefined
    preview.traverse((o) => {
      if (o instanceof THREE.LineSegments && o.material instanceof THREE.LineDashedMaterial) found = o
    })
    return found
  }
  const ends = (line: THREE.LineSegments): number[] => {
    const pos = line.geometry.getAttribute('position')
    return [pos.getX(0), pos.getY(0), pos.getX(1), pos.getY(1)]
  }

  it('runs to the cursor while the first point is chosen, then holds at it through the sweep', () => {
    const { tool, preview } = makeTool()
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // center
    expect(dashedLine(preview)).toBeUndefined()

    tool.onPointerMove(makeSnap({ x: 2, y: 1, z: 0 }), rayThrough(2, 1))
    expect(ends(dashedLine(preview)!)).toEqual([0, 0, 2, 1])

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // first point
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep
    expect(ends(dashedLine(preview)!)).toEqual([0, 0, 1, 0])

    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit
    expect(dashedLine(preview)).toBeUndefined()
  })
})
