import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RotateTool } from './RotateTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { worldPerPixelPerspective, worldPerPixelOrtho } from '../viewport/math'
import type { DrawingAxes } from './drawingAxes'

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
  return {
    axes: vi.fn(() => new Float64Array(frame)),
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
      sketch === 3n || sketch === 6n ? new Float64Array([0, 0, 0, 0, 0, 1]) : undefined,
    ),
    sketch_island_ids: vi.fn((sketch: bigint) =>
      sketch === 3n ? [40n, 41n] : sketch === 6n ? [45n] : sketch >= 900n ? [77n] : [],
    ),
    sketch_island_edges: vi.fn((_sketch: bigint, island: bigint) =>
      island === 40n ? [10n] : island === 41n ? [20n] : island === 45n ? [30n] : [],
    ),
    sketch_edge_island: vi.fn((_sketch: bigint, edge: bigint) =>
      edge >= 500n ? 78n : edge === 10n ? 40n : edge === 20n ? 41n : edge === 30n ? 45n : undefined,
    ),
    sketch_edge_endpoints: vi.fn((_sketch: bigint, edge: bigint) =>
      edge === 10n ? [0, 0, 0, 1, 0, 0]
      : edge === 20n ? [5, 5, 0, 6, 5, 0]
      : edge === 30n ? [3, 0, 0, 2, 1, 0]
      : undefined,
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
        return { new_edges: () => [id], free: () => { /* no-op */ } }
      },
    ),
    // The ghost preview builds a sketch-island NodeRef's lines regardless of
    // objectsGroup (null in these logic tests) — no geometry needed to
    // exercise the copy/array logic itself.
    sketch_island_lines: vi.fn(() => new Float32Array(0)),
    copy_sketch_islands: vi.fn(
      (_sketch: bigint, _islands: BigUint64Array, _affine: Float64Array) => {
        const id = nextSketchId++
        gen++
        return id
      },
    ),
  }
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
    mesh.name = `InstanceFace_${instance}_${member}`
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

describe('RotateTool — sketch copy (Move\'s "you can\'t Copy a Sketch" envelope)', () => {
  // `runNinetyDegreeGesture` rotates 90° about the origin on the inferred
  // (default) axis, world +Z — exactly normal to sketch 3's ground plane,
  // with the pivot ON that plane. That is the in-plane case (spokes/petals/
  // bolt-ring): the copy must REPLAY into the SAME sketch, not detach onto a
  // new one.
  it('an in-plane copy REPLAYS into the SAME sketch id, with the geometry already rotated', () => {
    const t = makeTool({ selection: [{ kind: 'sketch-island', id: 40n, sketch: 3n }] })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    // One gesture bracket on the SOURCE sketch — never a new one.
    expect(t.wasmScene.sketch_begin_gesture).toHaveBeenCalledWith(3n)
    expect(t.wasmScene.sketch_end_gesture).toHaveBeenCalledWith(3n)
    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()

    // The edge (0,0,0)-(1,0,0) rotated 90° about Z through the origin lands
    // at (0,0,0)-(0,1,0) — the geometry itself arrives already rotated, not
    // merely offset.
    expect(t.wasmScene.sketch_add_segment).toHaveBeenCalledTimes(1)
    const [sketch, ax, ay, az, bx, by, bz] = t.wasmScene.sketch_add_segment.mock.calls[0]
    expect(sketch).toBe(3n)
    expect(ax).toBeCloseTo(0, 9)
    expect(ay).toBeCloseTo(0, 9)
    expect(az).toBeCloseTo(0, 9)
    expect(bx).toBeCloseTo(0, 9)
    expect(by).toBeCloseTo(1, 9)
    expect(bz).toBeCloseTo(0, 9)

    // A COPY, not a rotate — and no object-duplicate call for a pure sketch
    // selection.
    expect(t.wasmScene.transform_selection).not.toHaveBeenCalled()
    expect(t.wasmScene.duplicate_selection_array).not.toHaveBeenCalled()
    // The new island becomes the committed selection, on the SOURCE sketch.
    expect(t.onCommit).toHaveBeenCalledWith([{ kind: 'sketch-island', id: 78n, sketch: 3n }])
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('an in-plane copy of a drawn-circle island keeps curve identity: rotated center, same radius', () => {
    const t = makeTool({ selection: [{ kind: 'sketch-island', id: 45n, sketch: 6n }] })
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
    expect(t.onCommit).toHaveBeenCalledWith([{ kind: 'sketch-island', id: 78n, sketch: 6n }])
  })

  it('an out-of-plane rotation (axis not normal to the sketch) still lands a new sketch', () => {
    const t = makeTool({ selection: [{ kind: 'sketch-island', id: 40n, sketch: 3n }] })
    t.tool.onKey(makeKeyEvent('Alt'))
    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    t.tool.onKey(makeKeyEvent('ArrowLeft')) // lock Y — in the sketch's own plane, NOT its normal
    t.tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, on the Y-locked plane)
    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0)) // sweep toward +Z (~90° about Y)
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0)) // commit

    expect(t.wasmScene.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [sketch, islands] = t.wasmScene.copy_sketch_islands.mock.calls[0]
    expect(sketch).toBe(3n)
    expect(Array.from(islands as BigUint64Array)).toEqual([40n])
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled() // never a same-sketch replay
    expect(t.onCommit).toHaveBeenCalledWith([{ kind: 'sketch-island', id: 77n, sketch: 900n }])
  })

  it('an orientation-flipping rotation (180° about an in-plane axis) routes to the new-sketch arm, never a reflective in-plane replay', () => {
    const t = makeTool({ selection: [{ kind: 'sketch-island', id: 40n, sketch: 3n }] })
    t.tool.onKey(makeKeyEvent('Alt'))
    t.tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    t.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    t.tool.onKey(makeKeyEvent('ArrowRight')) // lock X — in the sketch's own plane
    t.tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(1, 0)) // reference (+Y, on the X-locked plane)
    t.tool.onPointerMove(makeSnap({ x: 0, y: -1, z: 0 }), rayThrough(0, -1)) // sweep to −Y: 180°
    t.tool.onPointerDown(makeSnap({ x: 0, y: -1, z: 0 }), rayThrough(0, -1)) // commit

    expect(t.wasmScene.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [sketch] = t.wasmScene.copy_sketch_islands.mock.calls[0]
    expect(sketch).toBe(3n)
    expect(t.wasmScene.sketch_add_segment).not.toHaveBeenCalled() // never a reflective in-plane replay
    expect(t.onCommit).toHaveBeenCalledWith([{ kind: 'sketch-island', id: 77n, sketch: 900n }])
  })

  it('a sketch copy ARMS the ×N array window: typing 5x retracts the single-copy commit with ONE scene undo, then replays 5 cumulative copies into the SAME sketch inside a SECOND (still singular) gesture bracket', () => {
    const t = makeTool({ selection: [{ kind: 'sketch-island', id: 40n, sketch: 3n }] })
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
      expect(call[0]).toBe(3n) // every rep lands on the SOURCE sketch
    }
    expect(t.onArrayCommit).toHaveBeenCalledTimes(1)
  })

  it('a mixed selection duplicates objects AND copies sketches, and BOTH array together when ×N is typed (retracting BOTH prior entries)', () => {
    const t = makeTool({
      selection: [
        { kind: 'object', id: 1n },
        { kind: 'sketch-island', id: 40n, sketch: 3n },
      ],
    })
    t.tool.onKey(makeKeyEvent('Alt'))
    runNinetyDegreeGesture(t.tool)

    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(1)
    // The default gesture (Z axis, pivot at the origin) is in-plane for
    // sketch 3 — the sketch half replays into the source sketch, it does
    // not detach onto a new one.
    expect(t.wasmScene.copy_sketch_islands).not.toHaveBeenCalled()
    expect(t.wasmScene.sketch_add_segment).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenCalledWith([
      { kind: 'sketch-island', id: 78n, sketch: 3n },
      { kind: 'object', id: 100n },
    ])

    typeKeys(t.tool, 'x3')
    t.tool.onKey(makeKeyEvent('Enter'))

    // The initial commit recorded TWO history entries (one sketch replay,
    // one object array) — both get retracted before the array reissues at
    // count 3.
    expect(t.wasmScene.scene_undo).toHaveBeenCalledTimes(2)
    expect(t.wasmScene.duplicate_selection_array).toHaveBeenCalledTimes(2)
    expect(t.wasmScene.sketch_add_segment).toHaveBeenCalledTimes(1 + 3)
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

    // What App.tsx's onDeleteKey does: skip menuActionRef('edit-delete')
    // when isCapturingInput(ev.key) is true. It must be true here, or the
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
