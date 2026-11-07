/**
 * DimensionTool unit tests — fake-WasmScene pattern (TapeMeasureTool.test.ts).
 * Covers the linear-dimension gesture stages (click, click, drag-offset,
 * click commits), the radial-dimension entry from a drawn circle/arc plus
 * the Tab radius/diameter toggle, and the degenerate-offset refusal.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { DimensionTool, PREVIEW_COLOR } from './DimensionTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { kernelErrorMessage } from '../kernelErrors'
import { antipodalTolerance, distPointToLine } from '../viewport/annotationLayout'
import { axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { ctxOfPlane } from './planeUnits'
import { planeUnitsMock, type MockListing, type MockUnit } from './planeUnitsMock'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** `antipodalTolerance(radius=2)`, named for the disambiguation-table tests
 * below (`0.02 * radius`, since 2 is well above the `1e-6` absolute floor). */
const ANTIPODAL_TOL_R2 = antipodalTolerance(2)

/**
 * A point on a radius-`r` circle centered at the origin, on the far side
 * from `(r,0,0)` (angle 0), whose CHORD back to `(r,0,0)` passes exactly
 * `targetDist` from the centre — used to pin the antipodal-tolerance
 * boundary precisely rather than approximating it via a raw y-offset (the
 * y-offset of the second point is roughly `2*targetDist` for points this
 * close to antipodal, not `targetDist` — this closed form avoids that
 * off-by-factor-of-2 trap). Self-verifies against `distPointToLine` (the
 * exact function `chordPassesNearCentre` itself uses) so a formula slip
 * fails loudly here, in the test helper, rather than silently mis-placing
 * the boundary.
 */
function rimPointNearAntipodal(r: number, targetDist: number): { x: number; y: number; z: number } {
  const phi = 2 * Math.asin(targetDist / r)
  const angle = Math.PI - phi
  const point = { x: r * Math.cos(angle), y: r * Math.sin(angle), z: 0 }
  const actual = distPointToLine([0, 0, 0], [r, 0, 0], [point.x, point.y, point.z])
  if (Math.abs(actual - targetDist) > 1e-9) {
    throw new Error(`rimPointNearAntipodal: wanted dist ${targetDist}, got ${actual}`)
  }
  return point
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** Node has no global `KeyboardEvent` (these are `.test.ts` — no jsdom); a
 * plain mock with the fields/methods `Tool.onKey` implementations actually
 * use is the established pattern (idlePlaneLock.test.ts). */
function makeKeyEvent(key: string): KeyboardEvent {
  return { key, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

function makeWasmScene() {
  const linearCalls: unknown[][] = []
  const radialCalls: unknown[][] = []
  const scene = {
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn((_s: bigint) => new Float64Array([0, 0, 0, 0, 0, 1])),
    sketch_edge_curve: vi.fn((_s: bigint, _e: bigint) => 9n),
    sketch_curve_geom: vi.fn((_s: bigint, _c: bigint) => new Float64Array([0, 0, 0, 2])),
    add_linear_dimension: vi.fn((...args: unknown[]) => {
      linearCalls.push(args)
      return 1n
    }),
    add_radial_dimension: vi.fn((...args: unknown[]) => {
      radialCalls.push(args)
      return 2n
    }),
    // Alignment-snap candidate accessors (`DimensionTool._alignmentCandidates`)
    // — empty by default so no pre-existing test picks up a spurious
    // candidate; the alignment-snap describe block below overrides these.
    annotation_ids: vi.fn(() => new BigUint64Array()),
    annotation_kind: vi.fn((_id: bigint) => undefined as string | undefined),
    annotation_anchor_point: vi.fn((_id: bigint, _which: number) => undefined as Float64Array | undefined),
    annotation_offset: vi.fn((_id: bigint) => undefined as Float64Array | undefined),
    annotation_plane: vi.fn((_id: bigint) => undefined as Float64Array | undefined),
  }
  return { scene: scene as unknown as WasmScene, linearCalls, radialCalls }
}

function makeTool(scene: WasmScene) {
  const onCreated = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new DimensionTool(scene, new THREE.Group(), onCreated, onToast, onMeasurement)
  return { tool, onCreated, onToast, onMeasurement }
}

describe('DimensionTool — linear dimension gesture', () => {
  it('click, click, drag offset, click commits add_linear_dimension', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 5n, element: 1n }), RAY)
    expect(tool.statusHint()).toMatch(/second point/i)

    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 5n, element: 2n }), RAY)
    expect(tool.statusHint()).toMatch(/drag/i)

    // Drag the offset out perpendicular to the a-b line (+Y).
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    // Commit.
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)

    expect(linearCalls.length).toBe(1)
    expect(onCreated).toHaveBeenCalledTimes(1)
    const [aKind, aId, aPoint, bKind, bId, bPoint, offset] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array,
    ]
    expect(aKind).toBe(0) // NodeId::Object
    expect(aId).toBe(5n)
    expect(Array.from(aPoint)).toEqual([0, 0, 0])
    expect(bKind).toBe(0)
    expect(bId).toBe(5n)
    expect(Array.from(bPoint)).toEqual([2, 0, 0])
    // Offset is perpendicular to the a-b line (+X), so its X component is ~0.
    expect(offset[0]).toBeCloseTo(0, 9)
    expect(offset[1]).toBeGreaterThan(0)

    // Tool returns to idle after commit.
    expect(tool.statusHint()).toMatch(/click a point/i)
  })

  it('a free-space anchor (no object/instance) encodes as node kind -1', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)

    expect(linearCalls.length).toBe(1)
    const [aKind, , , bKind] = linearCalls[0] as [number, bigint, Float64Array, number]
    expect(aKind).toBe(-1)
    expect(bKind).toBe(-1)
  })

  it('an instance-placed anchor encodes as node kind 2 (Instance), not 0', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 5n, instance: 77n }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)

    expect(linearCalls.length).toBe(1)
    const [aKind, aId] = linearCalls[0] as [number, bigint]
    expect(aKind).toBe(2)
    expect(aId).toBe(77n)
  })

  it('refuses to commit with zero offset (cursor still on the a-b line)', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool, onToast, onCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)
    // Cursor sits ON the a-b line — no perpendicular offset ever set.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind: 'ground' }), RAY)

    expect(linearCalls.length).toBe(0)
    expect(onCreated).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('Escape cancels a linear gesture in progress without creating anything', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onKey(makeKeyEvent('Escape'))
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)

    // The Escape reset the gesture, so this second click starts a NEW
    // gesture at (2,0,0) rather than completing the old one.
    expect(linearCalls.length).toBe(0)
    expect(tool.statusHint()).toMatch(/second point/i)
  })
})

describe('DimensionTool — a click on a line dimensions the whole line', () => {
  /** A scene whose edges all run (0,0,0)→(2,0,0); sketch edges belong to no
   *  curve, so a sketch line is a plain line, not a radial target. */
  function edgeScene() {
    const made = makeWasmScene()
    const ends = () => new Float64Array([0, 0, 0, 2, 0, 0])
    Object.assign(made.scene, {
      sketch_edge_curve: vi.fn(() => undefined),
      edge_endpoints: vi.fn(ends),
      edge_endpoints_in_instance: vi.fn(ends),
      sketch_edge_endpoints: vi.fn(ends),
      sketch_edge_endpoints_in_instance: vi.fn(ends),
    })
    return made
  }
  type LinearCall = [number, bigint, Float64Array, number, bigint, Float64Array]
  /** Drag the offset out to +Y and click to place it. */
  function place(tool: DimensionTool): void {
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
  }

  it('one click on a solid edge takes both its ends, then the offset drag places the dimension', () => {
    const { scene, linearCalls } = edgeScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0.7, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 5n, element: 3n }), RAY)
    expect(scene.edge_endpoints).toHaveBeenCalledWith(5n, 3n)
    expect(tool.statusHint()).toMatch(/drag/i)
    place(tool)

    expect(linearCalls.length).toBe(1)
    const [aKind, aId, aPoint, bKind, bId, bPoint] = linearCalls[0] as LinearCall
    expect([aKind, aId, bKind, bId]).toEqual([0, 5n, 0, 5n]) // both ends follow the object
    expect(Array.from(aPoint)).toEqual([0, 0, 0])
    expect(Array.from(bPoint)).toEqual([2, 0, 0])
  })

  it('a drawn sketch line works the same, with free anchors', () => {
    const { scene, linearCalls } = edgeScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1.2, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 7n, element: 4n }), RAY)
    expect(scene.sketch_edge_endpoints).toHaveBeenCalledWith(7n, 4n)
    place(tool)

    expect(linearCalls.length).toBe(1)
    const [aKind, , aPoint, bKind, , bPoint] = linearCalls[0] as LinearCall
    expect([aKind, bKind]).toEqual([-1, -1])
    expect(Array.from(aPoint)).toEqual([0, 0, 0])
    expect(Array.from(bPoint)).toEqual([2, 0, 0])
  })

  it('an edge of a placed component resolves through the instance and anchors to it', () => {
    const { scene, linearCalls } = edgeScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 5n, instance: 77n, element: 3n }), RAY)
    expect(scene.edge_endpoints_in_instance).toHaveBeenCalledWith(77n, 5n, 3n)
    place(tool)

    const [aKind, aId, , bKind, bId] = linearCalls[0] as LinearCall
    expect([aKind, aId, bKind, bId]).toEqual([2, 77n, 2, 77n])
  })

  it('inside a component, a definition sketch line resolves through the edited instance', () => {
    const { scene } = edgeScene()
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 77n, component: 3n })

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 7n, element: 4n }), RAY)
    expect(scene.sketch_edge_endpoints_in_instance).toHaveBeenCalledWith(77n, 7n, 4n)
    expect(tool.statusHint()).toMatch(/drag/i)
  })

  it('a point on the line (endpoint, midpoint) still starts a point-to-point dimension', () => {
    for (const kind of ['endpoint', 'midpoint']) {
      const { scene } = edgeScene()
      const { tool } = makeTool(scene)
      tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind, elementKind: 'edge', object: 5n, element: 3n }), RAY)
      expect(tool.statusHint()).toMatch(/second point/i)
    }
  })

  it('a stale edge falls back to a point-to-point dimension from the click', () => {
    const { scene } = edgeScene()
    Object.assign(scene, { edge_endpoints: vi.fn(() => undefined) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 5n, element: 3n }), RAY)
    expect(tool.statusHint()).toMatch(/second point/i)
  })

  it('a drawn circle\'s rim keeps the radial gesture', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 7n, element: 4n }), RAY)
    tool.onPointerMove(makeSnap({ x: 4, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0 }), RAY)
    expect(radialCalls.length).toBe(1)
  })
})

describe('DimensionTool — camera-aware working plane, no shared sketch (dimensions-playtest2.md §3, as corrected by the angle-dimensions fix)', () => {
  // RED-CHECK (all tests in this block, against the pre-playtest2 code): it
  // had no `updateCamera` method at all (`tool.updateCamera is not a
  // function`), and its `have-b` offset always came straight from the
  // resolved snap point with no plane math — a free-space cursor snap at
  // z=0 (the ground fallback) was used completely literally, which was
  // exactly Kurt's "the dimension strongly wants to jump down to the
  // ground" complaint.
  //
  // NOTE: playtest2 §3's original cure ("view-facing" screen-parallel
  // planes) was itself later found defective from oblique cameras (the
  // angle-dimensions fix — see the next describe block and
  // annotationLayout.ts §3). These tests all use level, axis-aligned
  // cameras, where the corrected axis-aligned rule and the old view-facing
  // rule coincide — they now pin the corrected rule's behavior in exactly
  // the poses where the two rules agree, and they are precisely the tests
  // that could never have caught the oblique-camera defect on their own.

  function perspCamera(pos: [number, number, number], target: [number, number, number]): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    cam.position.set(...pos)
    cam.lookAt(...target)
    cam.updateMatrixWorld(true)
    return cam
  }

  it('without updateCamera, behaves exactly as before (offset taken straight from the snap point)', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 3, kind: 'ground' }), RAY)
    // A free-space cursor "collapsed to the ground" at z=0.
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), RAY)
    expect(linearCalls.length).toBe(1)
    const [, , , , , , offset] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array,
    ]
    expect(offset[2]).toBeCloseTo(-3, 9) // pulled straight down toward the ground
  })

  it('with a registered camera, the offset comes from the cursor ray against the vertical axis plane through the baseline — NOT the ground', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    // Baseline a->b is horizontal at height z=3, in free space (no sketch —
    // `pick_sketch` mocks to `undefined`). Camera looks straight along +Y at
    // the baseline's own height; the cursor ray aims 3 above the baseline,
    // so `axisDimensionPlane` picks the vertical axis plane y=0 through it
    // (the flat candidate is pierced at the camera itself, degenerate and
    // out-scored) — the plane a SketchUp user would expect to drag the
    // dimension's offset within.
    const camera = perspCamera([1, -10, 3], [1, 0, 3])
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 3, kind: 'ground' }), RAY)

    // The resolved SNAP point still reports a ground-collapsed z=0 (the old
    // bug's symptom) — but the cursor ray, aimed from the camera at a point
    // 3 units above the baseline (1,0,6), must win instead.
    const ray: Ray = { origin: [1, -10, 3], direction: [0, 10, 3] }
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0 }), ray)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), ray)

    expect(linearCalls.length).toBe(1)
    const [, , , , , , offset, plane] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array,
    ]
    // Offset is purely vertical (+Z), landing the dimension ABOVE the
    // baseline, at its own height — not pulled down toward z=0. (Precision
    // is 1e-3, not tighter: `camera.lookAt`'s quaternion round-trip through
    // `getWorldDirection` introduces a little float noise ahead of the
    // hand-computed geometry.)
    expect(offset[0]).toBeCloseTo(0, 3)
    expect(offset[1]).toBeCloseTo(0, 3)
    expect(offset[2]).toBeCloseTo(3, 3)
    // The committed plane is never the ground plane's normal (0,0,1).
    const planeNormal: [number, number, number] = [plane[3], plane[4], plane[5]]
    expect(Math.abs(planeNormal[2])).toBeLessThan(0.5)
  })

  it('degenerate guard: baseline nearly parallel to the view direction still commits a finite plane, never NaN', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    // Baseline runs along +Z; camera looks almost straight down (-Z) at it
    // (tilted a few degrees off vertical — enough that the second click's
    // vertical displacement clears `OFF_PLANE_VISIBLE_MIN_DEG` and is still
    // honored, see below), and the test's fixed cursor ray (`RAY`) runs
    // along -Z EXACTLY — parallel to every candidate plane, so
    // `axisDimensionPlane` pierces none and returns null, exercising the
    // raw-snapped-cursor fallback. The camera's own tilt and the fixed
    // ray's exact verticality are independent: `axisDimensionPlane` only
    // ever sees the ray, `_haveAPoint`'s visibility gate only ever sees the
    // camera's view direction.
    const camera = perspCamera([0, 0, 100], [6, 0, 0])
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    // A real endpoint (design v1.1 "Dimensions on-plane": `have-a` now
    // freezes a gesture plane from the first click — here the camera looks
    // nearly straight down, so it freezes the ground plane — and projects
    // any OTHER kind onto it; an actual second endpoint 2 units up stays
    // honored off that plane because the camera's few-degree tilt makes the
    // vertical displacement visible — see the stacked-corner tests below for
    // the invisible case, where the same shape of endpoint gets projected
    // instead).
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 1, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 1, kind: 'ground' }), RAY)

    expect(linearCalls.length).toBe(1)
    const [, , , , , , offset, plane] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array,
    ]
    for (const v of [...offset, ...plane]) {
      expect(Number.isFinite(v)).toBe(true)
    }
  })

  it('a shared sketch plane (case 1) still wins over the axis-aligned fallback, unchanged', () => {
    const { scene, linearCalls } = makeWasmScene()
    // Adopt a non-ground sketch plane at the first click (mirrors the
    // existing sketch-plane machinery — untouched by this fix).
    ;(scene as unknown as { pick_sketch: ReturnType<typeof vi.fn> }).pick_sketch = vi.fn(() => 21n)
    ;(scene as unknown as { sketch_plane: ReturnType<typeof vi.fn> }).sketch_plane = vi.fn(
      (_s: bigint) => new Float64Array([0, 0, 3, 0, 1, 0]), // plane y=0 through z=3, non-ground
    )
    const { tool } = makeTool(scene)
    const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    camera.position.set(0, -10, 30) // a very different view direction...
    camera.lookAt(0, 0, 0)
    camera.updateMatrixWorld(true)
    tool.updateCamera(camera) // ...must NOT override the adopted sketch plane.

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 3, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 5, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 5, kind: 'ground' }), RAY)

    expect(linearCalls.length).toBe(1)
    const [, , , , , , offset] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array,
    ]
    // The offset comes straight from the (plane-constrained-by-convention)
    // snap point, exactly as before — (1,0,5) - (0,0,3) perpendicular to
    // (1,0,0) is (0,0,2).
    expect(offset[2]).toBeCloseTo(2, 9)
  })
})

describe('DimensionTool — have-a gesture-plane freeze (design v1.1 Lane E "Dimensions on-plane")', () => {
  function perspCamera(pos: [number, number, number], target: [number, number, number]): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    cam.position.set(...pos)
    cam.lookAt(...target)
    cam.updateMatrixWorld(true)
    return cam
  }

  it('snapProjected() reports true when a have-a snap is actually projected onto the frozen plane', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = perspCamera([1, 0, 50], [1, 0, 0]) // straight down — freezes the z=2 plane below
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    expect(tool.snapProjected?.()).toBe(false) // the first click itself never projects anything

    // A non-precise kind (`on-face`) reporting a DIFFERENT height (z=0) —
    // must be projected onto the frozen z=2 plane, and disclosed as such.
    tool.onPointerMove(makeSnap({ x: 3, y: 0, z: 0, kind: 'on-face' }), RAY)
    expect(tool.snapProjected?.()).toBe(true)
  })

  it('a precise off-plane point whose displacement is INVISIBLE from this camera (straight down) gets projected, not honored — the stacked-corner playtest fix', () => {
    // RED-CHECK (against the pre-fix code, which honored ANY off-plane
    // precise point unconditionally): in a Top view every endpoint in a
    // framed wall's corner is stacked straight down the view ray — slab,
    // sole plate, stud top, top plates — and looks identical on screen no
    // matter which one a snap resolves. Honoring one 7cm below the frozen
    // plane (an ordinary stud-top-to-top-plate gap) commits a dimension
    // that reads as coplanar but isn't, with no visible cue that it moved.
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = perspCamera([1, 0, 50], [1, 0, 0]) // straight down — freezes the z=2 plane below
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    expect(tool.snapProjected?.()).toBe(false) // the first click itself never projects anything

    // A genuine endpoint 7cm BELOW the frozen plane — its entire
    // displacement runs straight along the view ray (0,0,-1), the sharpest
    // possible invisible case.
    tool.onPointerMove(makeSnap({ x: 3, y: 0, z: 1.93, kind: 'endpoint' }), RAY)
    expect(tool.snapProjected?.()).toBe(true)
  })

  it('a precise off-plane point whose displacement IS visible from this camera (ISO) is still honored — an ISO height measurement is unaffected', () => {
    // The paired case: from an oblique ISO camera, a top-corner endpoint
    // well off the frozen plane reads clearly as a different point on
    // screen (the maintainer's own "bottom corner -> top corner" height
    // measurement, displacement well off the view direction) and must stay
    // honored exactly, same as before this fix.
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = perspCamera([8, -8, 8], [1, 1, 1]) // standard ISO
    tool.updateCamera(camera)

    // First click freezes the Y-normal plane through the origin (the most
    // view-aligned axis for this pose — see the "axis-aligned working
    // plane" block below for the same camera's own axis pick).
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.snapProjected?.()).toBe(false)

    // A "top corner" well off that plane in Y — its displacement sits at a
    // steep angle from the ISO view direction, nowhere near the 2-degree
    // invisibility band.
    tool.onPointerMove(makeSnap({ x: 2, y: 3, z: 2, kind: 'endpoint' }), RAY)
    expect(tool.snapProjected?.()).toBe(false)
  })

  it('Top view, stacked corner: a real endpoint below the frozen plane commits COPLANAR with the first click, not at its own raw height', () => {
    // The full end-to-end version of the invisible-displacement test above:
    // a framed-wall corner with several endpoints stacked straight down the
    // view ray (a wall-top corner at z=2.4765, a stud top at z=2.4003 —
    // 7.6cm lower). The second click's snap resolves the stud top (a real,
    // precise `endpoint`) — it must still commit coplanar with the first
    // click, not at its own different height.
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = perspCamera([1, 1, 50], [1, 1, 0]) // Top view, straight down
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2.4765, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 2.4003, kind: 'endpoint' }), RAY)
    expect(tool.statusHint()).toMatch(/drag/i) // reached have-b, not swallowed as a same-point click

    const dragRay: Ray = { origin: [1, 1, 50], direction: [0, 0, -1] }
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 2.4765 }), dragRay)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2.4765 }), dragRay)

    expect(linearCalls.length).toBe(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array,
    ]
    expect(aPoint[2]).toBeCloseTo(2.4765, 9)
    // Projected onto the frozen plane — NOT the stud top's own raw z.
    expect(bPoint[2]).toBeCloseTo(2.4765, 9)
    expect(bPoint[0]).toBeCloseTo(3, 9)
  })

  it('Top view (parallel projection): a second point that snaps to on-face/ground resolves at a.z, not the literal ground', () => {
    // Straight-down camera — the same reading a Top-view Parallel Projection
    // camera gives `updateCamera` (it only ever reads world direction).
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = perspCamera([1, 0, 50], [1, 0, 0])
    tool.updateCamera(camera)

    // First click: a wall-top corner, 2m up.
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    // Second click: an ordinary on-face/ground hover on the OTHER wall top —
    // its raw z happens to read 0 (an occluded-geometry ground read, or a
    // genuine `on-face` snap whose reported point is still what the old
    // ground-plane fallback would have used) — neither a precise point kind,
    // so it must be projected onto the frozen (Top-view horizontal) plane
    // through the first click's own height, landing at z=2 — never the
    // literal ground.
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0, kind: 'on-face' }), RAY)
    // Drag the offset out, then commit — a ray straight down through
    // (1,1,*), matching the Top-view direction (unlike `RAY`, which points
    // straight down through the ORIGIN and so would pierce the working
    // plane at `a` itself, a degenerate zero offset).
    const dragRay: Ray = { origin: [1, 1, 50], direction: [0, 0, -1] }
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 2 }), dragRay)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2 }), dragRay)

    expect(linearCalls.length).toBe(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array,
    ]
    expect(Array.from(aPoint)).toEqual([0, 0, 2])
    // BOTH endpoints land at a.z = 2 — the old bug would have committed
    // b.z = 0 (the literal ground the on-face/ground snap reported).
    expect(bPoint[2]).toBeCloseTo(2, 9)
  })

  it('ISO camera: a real endpoint at a DIFFERENT height than the first click is still honored, off the frozen plane', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = perspCamera([8, -8, 8], [1, 1, 1])
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    // A genuine endpoint at a DIFFERENT height (5, not 2) — a precise point
    // kind, so `have-a` must honor it exactly, never project it down onto
    // whatever plane the first click's own freeze happened to pick.
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 5, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 3.5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 3.5 }), RAY)

    expect(linearCalls.length).toBe(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array,
    ]
    expect(Array.from(aPoint)).toEqual([0, 0, 2])
    expect(Array.from(bPoint)).toEqual([0, 0, 5]) // honored exactly, not projected
  })

  it('an arrow-key lock pressed BEFORE the first click freezes that axis plane immediately — the second click\'s own snap search is constrained to it', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    // No camera registered at all — the lock must win over the "no camera,
    // no freeze" fallback, unlike the ordinary view-aligned case.
    tool.onKey(makeKeyEvent('ArrowUp')) // locks the blue (Z-normal) plane
    expect(tool.statusHint()).toMatch(/blue/i)

    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'ground' }), RAY)
    const constraint = tool.snapConstraint(RAY)
    expect(constraint?.constraintPlane).toEqual({ point: [1, 2, 3], normal: [0, 0, 1] })
    expect(constraint?.offPlanePoints).toBe(true)
  })

  it('a lock pressed BETWEEN the two clicks (mid have-a) also changes the frozen plane for the still-pending second click', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = perspCamera([8, -8, 8], [1, 1, 1]) // would otherwise pick a view-aligned axis
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'ground' }), RAY)
    // No lock yet: some view-aligned axis plane is frozen (whichever ISO
    // picks) — then the user locks mid-gesture, before the second click.
    tool.onKey(makeKeyEvent('ArrowRight')) // locks the red (X-normal) plane
    expect(tool.statusHint()).toMatch(/red/i)
    // The lock is reflected in the constraint IMMEDIATELY — a live check
    // (`_haveAPlane`), not only at the next click.
    expect(tool.snapConstraint(RAY)?.constraintPlane).toEqual({
      point: [1, 2, 3],
      normal: [1, 0, 0],
    })

    // A `'ground'` snap NOT already on the locked plane (x=1) — must be
    // projected onto it, not honored as the raw (4,2,6).
    tool.onPointerDown(makeSnap({ x: 4, y: 2, z: 6, kind: 'ground' }), RAY)
    expect(tool.statusHint()).toMatch(/drag/i) // reached have-b — not a same-point refusal

    tool.onPointerMove(makeSnap({ x: 1, y: 5, z: 8 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 5, z: 8 }), RAY)

    expect(linearCalls.length).toBe(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array,
    ]
    expect(Array.from(aPoint)).toEqual([1, 2, 3])
    // The committed baseline's second point lies on the LOCKED plane
    // (x = 1, the first click's own x) — projected from the raw (4,2,6) —
    // not on whatever view-aligned axis plane had been frozen before the
    // lock was pressed.
    expect(bPoint[0]).toBeCloseTo(1, 9)
    expect(bPoint[1]).toBeCloseTo(2, 9)
    expect(bPoint[2]).toBeCloseTo(6, 9)
  })
})

describe('DimensionTool — have-b plane continuity seeded from the have-a freeze (design v1.1 "Dimensions on-plane")', () => {
  // Camera/baseline pinned empirically (searched, not hand-derived): with a
  // FRESH (unseeded) drag state the natural best-scoring candidate for THIS
  // ray is the vertical (Y-normal) plane, but the `have-a` freeze picks the
  // flat (Z-normal) plane for the SAME camera (a different, baseline-
  // independent formula — "most view-aligned axis" vs. the drag's own
  // screen-projected score) — robust across a wide range of near-baseline
  // ray offsets (0.001-0.05 NDC), not a knife's-edge tie.
  const camera = new THREE.PerspectiveCamera(45, 800 / 600, 0.1, 1000)
  camera.position.set(-5, -9, 12)
  camera.lookAt(1, 0, 2)
  camera.updateMatrixWorld(true)
  camera.updateProjectionMatrix()

  function normalKey(n: [number, number, number]): string {
    const ax = Math.abs(n[0])
    const ay = Math.abs(n[1])
    const az = Math.abs(n[2])
    return ax >= ay && ax >= az ? 'x' : ay >= az ? 'y' : 'z'
  }

  /** Same baseline (0,0,2)->(2,0,2), differing only in the SECOND click's
   *  snap kind: `'endpoint'` bypasses the freeze/seed entirely (a precise
   *  point, honored off-plane per `_haveAPoint`) — the pre-Lane-E, unseeded
   *  behavior; `'on-face'` is projected, so it both lands on the SAME
   *  committed point (already on the frozen plane by construction, since
   *  the frozen plane's normal is ⟂ to this baseline) AND seeds the fresh
   *  `have-b` drag state's latch with it. */
  function haveBPlaneAt(kind: 'endpoint' | 'on-face', ndcX: number): string | undefined {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(camera)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 2, kind }), RAY)
    expect(tool.statusHint()).toMatch(/drag/i) // sanity: really reached have-b

    const raycaster = new THREE.Raycaster()
    raycaster.setFromCamera(new THREE.Vector2(ndcX, 0), camera)
    const ray: Ray = {
      origin: [raycaster.ray.origin.x, raycaster.ray.origin.y, raycaster.ray.origin.z],
      direction: [raycaster.ray.direction.x, raycaster.ray.direction.y, raycaster.ray.direction.z],
    }
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3 }), ray)
    const normal = tool.snapConstraint(ray)?.constraintPlane?.normal
    return normal !== undefined ? normalKey(normal) : undefined
  }

  it('an off-plane-honored second click (endpoint) leaves have-b unseeded — the natural best-scoring plane wins', () => {
    expect(haveBPlaneAt('endpoint', 0.01)).toBe('y')
  })

  it('a projected second click (on-face) seeds have-b with the have-a freeze\'s own plane — continuity, not the natural pick', () => {
    expect(haveBPlaneAt('on-face', 0.01)).toBe('z')
  })
})

describe('DimensionTool — axis-aligned working plane from oblique cameras + arrow-key plane lock (angle-dimensions fix)', () => {
  // RED-CHECK (whole block, against the pre-fix code): the unfixed tool
  // builds the have-b offset in the SCREEN-PARALLEL ("view-facing") plane
  // through the baseline. From the genuinely oblique ISO camera below that
  // plane's in-plane offset direction is the 45-degree world diagonal
  // ~(0, 0.61, 0.79) — so the "vertical drag" test gets offset
  // ~(0, 0.64, 0.82) instead of (0, 0, 1.2), the "flat drag" test gets
  // ~(0, 0.89, 1.15) instead of (0, 3, 0), and every arrow-key test fails
  // because the unfixed `onKey` ignores arrows entirely. This block, plus
  // e2e/dimensions-oblique.spec.ts, is the coverage the original suite
  // lacked: every pre-fix test used a level or axis-aligned camera, where
  // the screen-parallel plane coincides with an axis plane and the defect
  // is invisible.

  function perspCamera(pos: [number, number, number], target: [number, number, number]): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    cam.position.set(...pos)
    cam.lookAt(...target)
    cam.updateMatrixWorld(true)
    return cam
  }

  /** The standard ISO pose: no view axis aligned with any world axis. */
  function isoCamera(): THREE.PerspectiveCamera {
    return perspCamera([8, -8, 8], [1, 1, 1])
  }

  // Baseline: (0,0,2) -> (2,0,2), along +X at height 2 (a box's top edge).
  function startBaseline(tool: DimensionTool): void {
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 2, kind: 'endpoint' }), RAY)
  }

  // Cursor ray from the ISO camera aimed straight above the baseline's
  // midpoint, through world (1, 0, 3.2): the vertical-plane reading is
  // "hang the dimension 1.2 above the edge" (offset (0,0,1.2)); the flat
  // plane z=2 is pierced at (-0.75, 2, 2), OUTSIDE the baseline's span.
  const UP_RAY: Ray = { origin: [8, -8, 8], direction: [-7, 8, -4.8] }
  // Cursor ray aimed through world (1, 3, 2): ON the flat plane through the
  // baseline, 3 behind it, within the span; the vertical plane y=0 is
  // pierced at (2.91, 0, 3.63), outside the span.
  const SIDE_RAY: Ray = { origin: [8, -8, 8], direction: [-7, 11, -6] }

  function committedOffsetAndNormal(calls: unknown[][]): { offset: Float64Array; normal: [number, number, number] } {
    expect(calls.length).toBe(1)
    const [, , , , , , offset, plane] = calls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array,
    ]
    return { offset, normal: [plane[3], plane[4], plane[5]] }
  }

  it('ISO camera, dragging above the edge: offset lies in the VERTICAL axis plane through the baseline — never the screen-parallel 45-degree diagonal', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    startBaseline(tool)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset, normal } = committedOffsetAndNormal(linearCalls)
    expect(offset[0]).toBeCloseTo(0, 6)
    expect(offset[1]).toBeCloseTo(0, 6) // pre-fix: ~0.64 (the 45-degree diagonal)
    expect(offset[2]).toBeCloseTo(1.2, 6)
    expect(Math.abs(normal[1])).toBeCloseTo(1, 6) // the y=0 axis plane
  })

  it('ISO camera, dragging back across the flat plane: offset lies in the FLAT axis plane through the baseline', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    startBaseline(tool)
    tool.onPointerMove(makeSnap({ x: 1, y: 3, z: 2 }), SIDE_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 3, z: 2 }), SIDE_RAY)
    const { offset, normal } = committedOffsetAndNormal(linearCalls)
    expect(offset[0]).toBeCloseTo(0, 6)
    expect(offset[1]).toBeCloseTo(3, 6)
    expect(offset[2]).toBeCloseTo(0, 6) // pre-fix: ~1.15 (diagonal again)
    expect(Math.abs(normal[2])).toBeCloseTo(1, 6) // the z=2 axis plane
  })

  it('ArrowUp locks the blue (flat) plane, overriding the drag\'s natural vertical choice — the draw tools\' arrow convention', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    startBaseline(tool)
    tool.onKey(makeKeyEvent('ArrowUp'))
    expect(tool.statusHint()).toMatch(/blue/i)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset } = committedOffsetAndNormal(linearCalls)
    // The flat plane z=2 through the baseline is pierced at (-0.75, 2, 2).
    expect(offset[0]).toBeCloseTo(0, 6)
    expect(offset[1]).toBeCloseTo(2, 6)
    expect(offset[2]).toBeCloseTo(0, 6)
  })

  it('the same arrow pressed again unlocks — the drag reverts to the natural axis plane', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    startBaseline(tool)
    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset } = committedOffsetAndNormal(linearCalls)
    expect(offset[1]).toBeCloseTo(0, 6)
    expect(offset[2]).toBeCloseTo(1.2, 6)
  })

  it('ArrowDown clears an armed lock', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    startBaseline(tool)
    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onKey(makeKeyEvent('ArrowDown'))
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset } = committedOffsetAndNormal(linearCalls)
    expect(offset[1]).toBeCloseTo(0, 6)
    expect(offset[2]).toBeCloseTo(1.2, 6)
  })

  it('a lock whose axis is parallel to the baseline is unusable — the natural axis plane applies instead', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    startBaseline(tool) // baseline along +X
    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock — no plane with normal X can contain an X baseline
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset } = committedOffsetAndNormal(linearCalls)
    expect(offset[1]).toBeCloseTo(0, 6)
    expect(offset[2]).toBeCloseTo(1.2, 6)
  })

  it('an idle-armed lock survives an Escape-cancelled gesture and applies to the next one (draw-tool parity)', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    tool.onKey(makeKeyEvent('ArrowUp')) // armed while idle
    expect(tool.statusHint()).toMatch(/blue/i)
    startBaseline(tool)
    tool.onKey(makeKeyEvent('Escape')) // cancels the gesture, keeps the lock
    expect(tool.statusHint()).toMatch(/blue/i)
    startBaseline(tool)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset } = committedOffsetAndNormal(linearCalls)
    expect(offset[1]).toBeCloseTo(2, 6)
    expect(offset[2]).toBeCloseTo(0, 6)
  })

  it('Escape while idle clears the lock first (draw-tool parity)', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.statusHint()).not.toMatch(/blue/i)
    startBaseline(tool)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset } = committedOffsetAndNormal(linearCalls)
    expect(offset[1]).toBeCloseTo(0, 6)
    expect(offset[2]).toBeCloseTo(1.2, 6)
  })

  it('perpendicular view: the drag alone cannot reach the edge-on plane, and the blue arrow lock is the pinned escape hatch', () => {
    // Camera pitched up off the Front view, looking PERPENDICULAR to the
    // baseline (viewDir·base = 0) — the pose where both candidate planes
    // project onto the same screen strip and the drag carries no plane
    // information (annotationLayout.ts §3 AMBIGUOUS-POSES; function-level
    // pin in annotationLayout.test.ts). A drag aimed square across the
    // FLAT plane still commits in the face-on vertical plane — deliberate
    // — and ArrowUp then reaches the flat plane, which is exactly why the
    // lock exists.
    const camera = perspCamera([1, -10, 3.64], [1, 0, 0])
    const FLAT_AIM_RAY: Ray = { origin: [1, -10, 3.64], direction: [0, 12, -3.64] } // through (1, 2, 0)

    const unlocked = (() => {
      const { scene, linearCalls } = makeWasmScene()
      const { tool } = makeTool(scene)
      tool.updateCamera(camera)
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), FLAT_AIM_RAY)
      tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0 }), FLAT_AIM_RAY)
      return committedOffsetAndNormal(linearCalls)
    })()
    // Face-on vertical plane (normal Y): offset purely vertical, no Y part.
    expect(Math.abs(unlocked.normal[1])).toBeCloseTo(1, 6)
    expect(unlocked.offset[1]).toBeCloseTo(0, 6)
    expect(unlocked.offset[2]).toBeGreaterThan(0)

    const locked = (() => {
      const { scene, linearCalls } = makeWasmScene()
      const { tool } = makeTool(scene)
      tool.updateCamera(camera)
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onKey(makeKeyEvent('ArrowUp')) // blue lock: the flat plane
      tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), FLAT_AIM_RAY)
      tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0 }), FLAT_AIM_RAY)
      return committedOffsetAndNormal(linearCalls)
    })()
    // The same drag now lands in the flat plane: offset (0, 2, 0).
    expect(locked.offset[1]).toBeCloseTo(2, 6)
    expect(locked.offset[2]).toBeCloseTo(0, 6)
  })

  it('an active lock beats an adopted sketch plane (the draw tools\' "lock overrides adoption" rule)', () => {
    const { scene, linearCalls } = makeWasmScene()
    // The first click adopts a non-ground sketch plane (y=0 through z=3)...
    ;(scene as unknown as { pick_sketch: ReturnType<typeof vi.fn> }).pick_sketch = vi.fn(() => 21n)
    ;(scene as unknown as { sketch_plane: ReturnType<typeof vi.fn> }).sketch_plane = vi.fn(
      (_s: bigint) => new Float64Array([0, 0, 3, 0, 1, 0]),
    )
    const { tool } = makeTool(scene)
    tool.updateCamera(isoCamera())
    startBaseline(tool)
    tool.onKey(makeKeyEvent('ArrowUp')) // ...but a mid-gesture blue lock overrides it.
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3.2 }), UP_RAY)
    const { offset } = committedOffsetAndNormal(linearCalls)
    expect(offset[1]).toBeCloseTo(2, 6)
    expect(offset[2]).toBeCloseTo(0, 6)
  })
})

describe('DimensionTool — working-plane continuity during a single drag (plane latching)', () => {
  // RED-CHECK (against the first, hysteresis-free axisDimensionPlane): the
  // adversarial review reproduced a non-monotonic plane flip on the very
  // first standard ISO camera tried — baseline (0,0,0)->(0,0,2), eye
  // (8,-8,8) looking at (0,0,1), a smooth sideways NDC sweep never
  // reversing direction picked normal Y for ndcX in [-0.40,-0.21], X for
  // [-0.20,-0.16], then Y again — the working plane flipped away and BACK
  // during one continuous drag, snapping the whole preview twice. Cause:
  // the per-candidate in-span eligibility regions are perspective-warped,
  // non-complementary screen regions, and the |n·viewDir| score is a
  // constant of the drag — worse, from any ISO view it is EXACTLY tied
  // between candidates (an ISO view direction has equal-magnitude
  // components on all world axes), leaving array order as the decider.
  // These sweeps drive the tool exactly like the review's repro (a real
  // THREE.PerspectiveCamera + Raycaster path) and assert the property that
  // was missing from every earlier test: single final-click assertions can
  // never see a mid-drag flicker.

  function raycasterCamera(
    pos: [number, number, number],
    target: [number, number, number],
  ): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(45, 800 / 600, 0.1, 1000)
    cam.position.set(...pos)
    cam.lookAt(...target)
    cam.updateMatrixWorld(true)
    cam.updateProjectionMatrix()
    return cam
  }

  /** Sign-insensitive identity of an axis-plane normal: its dominant axis. */
  function normalKey(n: [number, number, number]): string {
    const ax = Math.abs(n[0])
    const ay = Math.abs(n[1])
    const az = Math.abs(n[2])
    return ax >= ay && ax >= az ? 'x' : ay >= az ? 'y' : 'z'
  }

  /**
   * Drive a full monotonic NDC sweep through the have-b drag and return the
   * run-length-compressed sequence of working-plane identities the tool
   * advertised (via `snapConstraint`) along the way.
   */
  function sweepPlanes(
    tool: DimensionTool,
    camera: THREE.PerspectiveCamera,
    fromNdc: [number, number],
    toNdc: [number, number],
    steps: number,
  ): string[] {
    const raycaster = new THREE.Raycaster()
    const runs: string[] = []
    for (let i = 0; i <= steps; i++) {
      const f = i / steps
      const ndc = new THREE.Vector2(
        fromNdc[0] + (toNdc[0] - fromNdc[0]) * f,
        fromNdc[1] + (toNdc[1] - fromNdc[1]) * f,
      )
      raycaster.setFromCamera(ndc, camera)
      const ray: Ray = {
        origin: [raycaster.ray.origin.x, raycaster.ray.origin.y, raycaster.ray.origin.z],
        direction: [raycaster.ray.direction.x, raycaster.ray.direction.y, raycaster.ray.direction.z],
      }
      tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1 }), ray)
      const constraint = tool.snapConstraint(ray)
      const normal = constraint?.constraintPlane?.normal
      if (normal === undefined) continue
      const key = normalKey(normal)
      if (runs.length === 0 || runs[runs.length - 1] !== key) runs.push(key)
    }
    return runs
  }

  it('the review repro: a monotonic sideways sweep across a vertical baseline from ISO never returns to an abandoned plane', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = raycasterCamera([8, -8, 8], [0, 0, 1])
    tool.updateCamera(camera)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    const runs = sweepPlanes(tool, camera, [-0.4, 0], [0.4, 0], 80)
    expect(runs.length).toBeGreaterThan(0)
    // A plane once abandoned during a monotonic drag must never come back
    // (the unfixed rule produced Y -> X -> Y here) ...
    expect(new Set(runs).size).toBe(runs.length)
    // ... and a single monotonic drag has at most one genuine intent change.
    expect(runs.length).toBeLessThanOrEqual(2)
  })

  it('a monotonic upward-to-backward sweep over a horizontal baseline from ISO never returns to an abandoned plane', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    const camera = raycasterCamera([8, -8, 8], [1, 1, 1])
    tool.updateCamera(camera)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 2, kind: 'endpoint' }), RAY)

    const runs = sweepPlanes(tool, camera, [-0.35, -0.3], [-0.35, 0.45], 80)
    expect(runs.length).toBeGreaterThan(0)
    expect(new Set(runs).size).toBe(runs.length)
    expect(runs.length).toBeLessThanOrEqual(2)
  })

  it('from ISO (where face-on scores tie exactly) the drag direction decides the plane, not candidate order', () => {
    // Vertical baseline: the two candidate planes have normals X and Y and
    // in-plane offset directions along Y and X respectively; from an ISO
    // camera |n·viewDir| ties EXACTLY for both, so a camera-only measure
    // would leave the choice to candidate order. The user's own drag must
    // decide instead: a drag whose ON-SCREEN direction is a candidate's own
    // projected offset direction picks that candidate. (The aim points are
    // derived from the camera's projection rather than hand-picked world
    // points — from ISO, a world-axis aim point does NOT read as an
    // axis-aligned drag on screen.)
    const camera = raycasterCamera([8, -8, 8], [0, 0, 1])
    const f = new THREE.Vector3()
    camera.getWorldDirection(f)
    // The screen-plane image of a world direction d: d - (d·f)f.
    const screenImage = (d: THREE.Vector3) => d.clone().addScaledVector(f, -d.dot(f))

    const planeForDragAlong = (offsetDir: THREE.Vector3): [number, number, number] | undefined => {
      const { scene } = makeWasmScene()
      const { tool } = makeTool(scene)
      tool.updateCamera(camera)
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
      // Aim at a = (0,0,0) displaced purely along the candidate's projected
      // offset direction — a drag that reads on screen as exactly that
      // candidate's offset motion.
      const aim = screenImage(offsetDir).multiplyScalar(1.5)
      const dir = aim.clone().sub(camera.position)
      const ray: Ray = {
        origin: [camera.position.x, camera.position.y, camera.position.z],
        direction: [dir.x, dir.y, dir.z],
      }
      tool.onPointerMove(makeSnap({ x: aim.x, y: aim.y, z: aim.z }), ray)
      return tool.snapConstraint(ray)?.constraintPlane?.normal
    }

    // Offset direction X belongs to the normal-Y candidate; offset
    // direction Y belongs to the normal-X candidate.
    const towardX = planeForDragAlong(new THREE.Vector3(1, 0, 0))
    expect(towardX).toBeDefined()
    expect(normalKey(towardX!)).toBe('y')

    const towardY = planeForDragAlong(new THREE.Vector3(0, 1, 0))
    expect(towardY).toBeDefined()
    expect(normalKey(towardY!)).toBe('x')
  })
})

describe('DimensionTool — radial dimension gesture', () => {
  const CIRCLE_SNAP: Partial<Snap> = {
    x: 2, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 3n,
  }

  it('clicking a drawn circle/arc enters radial mode and commits add_radial_dimension', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap(CIRCLE_SNAP), RAY)
    expect(tool.statusHint()).toMatch(/leader/i)

    tool.onPointerMove(makeSnap({ x: 4, y: 0, z: 0 }), RAY) // drag the leader outward
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0 }), RAY) // commit

    expect(radialCalls.length).toBe(1)
    expect(onCreated).toHaveBeenCalledTimes(1)
    const [anchorKind, , anchorPoint, kind, center, radius] = radialCalls[0] as [
      number, bigint, Float64Array, string, Float64Array, number,
    ]
    // A drawn (unextruded) sketch curve is never a document tree node.
    expect(anchorKind).toBe(-1)
    // Anchor projects onto the circle's rim (radius 2 centered at origin).
    expect(Math.hypot(anchorPoint[0], anchorPoint[1], anchorPoint[2])).toBeCloseTo(2, 6)
    expect(kind).toBe('radius')
    expect(Array.from(center)).toEqual([0, 0, 0])
    expect(radius).toBe(2)
  })

  it('Tab toggles radius/diameter while the radial gesture is hot', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap(CIRCLE_SNAP), RAY)
    tool.onKey(makeKeyEvent('Tab'))
    expect(onMeasurement).toHaveBeenLastCalledWith(expect.stringContaining('Ø'))

    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0 }), RAY) // commit as diameter
    expect(radialCalls.length).toBe(1)
    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('diameter')
  })

  it('a second Tab toggles back to radius', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap(CIRCLE_SNAP), RAY)
    tool.onKey(makeKeyEvent('Tab'))
    tool.onKey(makeKeyEvent('Tab'))
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0 }), RAY)

    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('radius')
  })
})

// The plane is the drafting context: a circle drawn ON A SOLID'S FACE has no
// free sketch to ask, so its center and radius come from the face plane's
// listing (curve runs) — a radial dimension works on every plane.
describe('DimensionTool — radial dimension on a circle drawn on a face', () => {
  const FACE_CTX = ctxOfPlane([0, 0, 1], [0, 0, 1]) as string
  const ring: [number, number, number][] = Array.from({ length: 12 }, (_, i) => [
    0.5 * Math.cos((i * Math.PI) / 6), 0.5 * Math.sin((i * Math.PI) / 6), 1,
  ])
  const circleUnit: MockUnit = {
    key: 1,
    edges: ring.map((a, i) => ({ key: 10 + i, a, b: ring[(i + 1) % 12], store: { kind: 'topology', object: 5 } })),
    curves: [{ key: 50, curveKey: 51, center: [0, 0, 1], radius: 0.5, edges: ring.map((_, i) => 10 + i) }],
  }
  const down = (x: number, y: number): Ray => ({ origin: [x, y, 6], direction: [0, 0, -1] })

  function faceScene(listing: MockListing) {
    const { scene, radialCalls } = makeWasmScene()
    const s = scene as unknown as Record<string, unknown>
    s.history_generation = vi.fn(() => 1n)
    s.pick_face = vi.fn(() => ({ object: () => 5n, face: () => 6n, depth: () => 5, instance: () => undefined, free: vi.fn() }))
    s.face_plane = vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1]))
    s.face_normal = vi.fn(() => new Float64Array([0, 0, 1]))
    Object.assign(s, planeUnitsMock({ [FACE_CTX]: listing }))
    return { scene, radialCalls }
  }

  it('reads the circle from the listing and commits a radial dimension from it', () => {
    const { scene, radialCalls } = faceScene({ units: [circleUnit] })
    const { tool, onCreated, onToast } = makeTool(scene)

    // Click the rim of the face circle, then drag the leader off it.
    tool.onPointerDown(makeSnap({ x: 0.5, y: 0, z: 1, kind: 'on-edge', object: 5n }), down(0.5, 0))
    expect(tool.statusHint()).toMatch(/leader/i)
    tool.onPointerMove(makeSnap({ x: 1.2, y: 0, z: 1 }), down(1.2, 0))
    tool.onPointerDown(makeSnap({ x: 1.2, y: 0, z: 1 }), down(1.2, 0))

    expect(onToast).not.toHaveBeenCalled()
    expect(onCreated).toHaveBeenCalledTimes(1)
    const [anchorKind, , anchorPoint, kind, center, radius] = radialCalls[0] as [
      number, bigint, Float64Array, string, Float64Array, number,
    ]
    expect(anchorKind).toBe(-1)
    expect(Array.from(center)).toEqual([0, 0, 1])
    expect(radius).toBe(0.5)
    expect(kind).toBe('diameter') // a whole circle defaults to its diameter
    expect(anchorPoint[2]).toBeCloseTo(1, 9)
    expect(Math.hypot(anchorPoint[0], anchorPoint[1])).toBeCloseTo(0.5, 6)
  })

  it('a circle with a null radius (an ellipse in a stretched instance) offers no radial dimension', () => {
    const ellipse: MockUnit = {
      ...circleUnit,
      curves: [{ key: 50, curveKey: 51, center: [0, 0, 1], radius: null, edges: ring.map((_, i) => 10 + i) }],
    }
    const { scene, radialCalls } = faceScene({ units: [ellipse] })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0.5, y: 0, z: 1, kind: 'on-edge', object: 5n }), down(0.5, 0))
    expect(tool.statusHint()).not.toMatch(/leader/i)
    tool.onPointerDown(makeSnap({ x: 1.2, y: 0, z: 1 }), down(1.2, 0))
    expect(radialCalls).toHaveLength(0)
  })

  it('a listed ARC (a run that does not close) defaults to a radius', () => {
    const arc: MockUnit = {
      ...circleUnit,
      curves: [{ key: 50, curveKey: 51, center: [0, 0, 1], radius: 0.5, edges: [10, 11, 12, 13, 14, 15] }],
    }
    const { scene, radialCalls } = faceScene({ units: [arc] })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0.5, y: 0, z: 1, kind: 'on-edge', object: 5n }), down(0.5, 0))
    tool.onPointerDown(makeSnap({ x: 1.2, y: 0, z: 1 }), down(1.2, 0))

    expect((radialCalls[0] as unknown[])[3]).toBe('radius')
  })

  it('a rim click at a facet MIDPOINT of a 48-facet circle (on the chord, off the circle) still resolves the circle', () => {
    const n = 48
    const pts: [number, number, number][] = Array.from({ length: n }, (_, i) => [
      0.5 * Math.cos((i * 2 * Math.PI) / n), 0.5 * Math.sin((i * 2 * Math.PI) / n), 1,
    ])
    const unit48: MockUnit = {
      key: 1,
      edges: pts.map((a, i) => ({ key: 100 + i, a, b: pts[(i + 1) % n], store: { kind: 'topology', object: 5 } })),
      curves: [{ key: 50, curveKey: 51, center: [0, 0, 1], radius: 0.5, edges: pts.map((_, i) => 100 + i) }],
    }
    const mid: [number, number, number] = [
      (pts[0][0] + pts[1][0]) / 2, (pts[0][1] + pts[1][1]) / 2, 1,
    ]
    // The sagitta puts the midpoint well beyond the 1e-5 on-circle check.
    expect(Math.abs(Math.hypot(mid[0], mid[1]) - 0.5)).toBeGreaterThan(1e-4)
    const { scene, radialCalls } = faceScene({ units: [unit48] })
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: mid[0], y: mid[1], z: 1, kind: 'midpoint', object: 5n }), down(mid[0], mid[1]))
    expect(tool.statusHint()).toMatch(/leader/i)
    tool.onPointerDown(makeSnap({ x: 1.2, y: 0.1, z: 1 }), down(1.2, 0.1))
    expect(onCreated).toHaveBeenCalledTimes(1)
    const [, , , , center, radius] = radialCalls[0] as [number, bigint, Float64Array, string, Float64Array, number]
    expect(Array.from(center)).toEqual([0, 0, 1])
    expect(radius).toBe(0.5)
  })

  it('a snap that lies on no listed circle starts no radial gesture', () => {
    const { scene } = faceScene({ units: [circleUnit] })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0.9, y: 0, z: 1, kind: 'on-face', object: 5n }), down(0.9, 0))
    expect(tool.statusHint()).not.toMatch(/leader/i)
  })
})

describe('DimensionTool — sameCurve compares planes too (playtest-2 review finding 3)', () => {
  // Two concentric, same-radius circles on PERPENDICULAR planes (sketch 11n
  // in the z=0 plane, sketch 22n in the y=0 plane) — an ordinary modelling
  // situation, not a degenerate one. `sketch_curve_geom` reports the same
  // center+radius for both (the mock ignores its args), so only the plane
  // distinguishes them; `sketch_plane` is overridden per-sketch below.
  //
  // RED-CHECK: against the unfixed `sameCurve` (center+radius only), the
  // second click is misclassified as landing on curve A (the FIRST curve):
  // `nearestRimPoint(curveA, [0,0,2])` projects the click onto curve A's OWN
  // plane (normal +Z) — but [0,0,2] relative to the shared center is purely
  // ALONG that normal, so the in-plane component is the zero vector and the
  // projection degenerates to curve A's fallback in-plane direction, which
  // for this normal happens to land EXACTLY on the first click's own rim
  // point. The tool reads that as "the same rim point clicked twice" and
  // silently swallows the second click (`{ kind: 'ignore' }`) instead of
  // treating it as an ordinary second dimension point — a genuinely
  // different circle's rim click does nothing at all.
  function circlePlaneScene() {
    const { scene, linearCalls, radialCalls } = makeWasmScene()
    ;(scene as unknown as { sketch_plane: ReturnType<typeof vi.fn> }).sketch_plane = vi.fn((s: bigint) =>
      s === 22n ? new Float64Array([0, 0, 0, 0, 1, 0]) : new Float64Array([0, 0, 0, 0, 0, 1]),
    )
    return { scene, linearCalls, radialCalls }
  }

  it('a rim point on a second, differently-planed circle is NOT folded into the first curve', () => {
    const { scene, linearCalls, radialCalls } = circlePlaneScene()
    const { tool } = makeTool(scene)

    // First click: rim of circle A (z=0 plane) only — (0,2,0) is not on
    // circle B's rim (y=0 plane).
    tool.onPointerDown(
      makeSnap({ x: 0, y: 2, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 3n }),
      RAY,
    )
    expect(tool.statusHint()).toMatch(/leader/i)

    // Second click: rim of circle B (y=0 plane) only — (0,0,2) is not on
    // circle A's rim. A genuinely different curve -> falls through to an
    // ordinary second dimension point (pending drag-offset), NOT folded
    // into curve A.
    tool.onPointerDown(
      makeSnap({ x: 0, y: 0, z: 2, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 22n, element: 7n }),
      RAY,
    )
    // Exact text, not just /drag/i — the pending-curve hint ALSO says "drag"
    // ("Drag out the leader…"), so only the precise 'have-b' wording proves
    // the stage actually advanced instead of staying stuck on the curve.
    expect(tool.statusHint()).toBe('Drag out the dimension line, then click to place it — arrows lock the plane.')
    expect(radialCalls.length).toBe(0)

    // Drag the offset and commit -> an ordinary linear dimension between
    // the two ACTUAL click points, not a fabricated one.
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), RAY)

    expect(radialCalls.length).toBe(0)
    expect(linearCalls.length).toBe(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [number, bigint, Float64Array, number, bigint, Float64Array]
    expect(Array.from(aPoint)).toEqual([0, 2, 0])
    expect(Array.from(bPoint)).toEqual([0, 0, 2])
  })
})

describe('DimensionTool — radial second-click disambiguation (dimensions-playtest2.md §4)', () => {
  // RED-CHECK (all tests in this block): against the unfixed code,
  // `onPointerDown`'s idle branch enters `{ kind: 'radial', radialKind:
  // 'radius' }` on the FIRST click that resolves a curve — every case below
  // that expects anything OTHER than a hardcoded-radius commit on the
  // SECOND click (a center click, an antipodal click, a chord-to-linear
  // handoff, the reverse click order) fails against that code, because the
  // old code has no concept of a second click contributing a measurement at
  // all — its second click only ever supplies `leaderDir`.

  const CENTER: Snap = { x: 0, y: 0, z: 0, kind: 'endpoint', elementKind: 'sketch-curve', sketch: 11n, sketchCurve: 9n }
  const RIM_0: Snap = { x: 2, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 3n }
  const RIM_90: Snap = { x: 0, y: 2, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 4n }
  const RIM_180_ANTIPODAL: Snap = { x: -2, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 5n }
  const FREE_SPACE: Snap = { x: 6, y: 6, z: 0, kind: 'ground' }
  const OTHER_OBJECT: Snap = { x: 9, y: 9, z: 0, kind: 'endpoint', object: 42n }

  it('first click does NOT commit — the tool is still capturing input, nothing added yet', () => {
    const { scene, linearCalls, radialCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    expect(linearCalls.length).toBe(0)
    expect(radialCalls.length).toBe(0)
    expect(tool.capturingInput()).toBe(true)
  })

  it('rim then off-curve free space -> Radius, anchored at the first rim click (the one-entity flow)', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap(FREE_SPACE), RAY)
    expect(onCreated).toHaveBeenCalledTimes(1)
    expect(radialCalls.length).toBe(1)
    const [, , anchorPoint, kind, center, radius] = radialCalls[0] as [
      number, bigint, Float64Array, string, Float64Array, number,
    ]
    expect(kind).toBe('radius')
    expect(Array.from(anchorPoint)).toEqual([2, 0, 0])
    expect(Array.from(center)).toEqual([0, 0, 0])
    expect(radius).toBe(2)
  })

  it('rim then the bare interior of a solid\'s face -> the same radial dimension as on the ground', () => {
    const { scene, radialCalls, linearCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap({ x: 6, y: 6, z: 0, kind: 'on-face', object: 42n, element: 3n, elementKind: 'face' }), RAY)
    expect(onCreated).toHaveBeenCalledTimes(1)
    expect(radialCalls.length).toBe(1)
    expect(linearCalls.length).toBe(0)
  })

  it('rim then a face OFF the curve\'s plane (a side wall, another solid) -> a linear dimension', () => {
    const { scene, radialCalls, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap({ x: 6, y: 6, z: 1.5, kind: 'on-face', object: 77n, element: 4n, elementKind: 'face' }), RAY)
    expect(radialCalls.length).toBe(0)
    // The linear dimension waits for its offset drag; nothing radial was made.
    expect(linearCalls.length).toBe(0)
    expect(tool.statusHint()).toMatch(/drag/i) // reached the linear offset stage
  })

  it('rim then the curve\'s centre -> Radius, anchored at the FIRST click\'s rim point', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap(CENTER), RAY)
    expect(onCreated).toHaveBeenCalledTimes(1)
    expect(radialCalls.length).toBe(1)
    const [, , anchorPoint, kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('radius')
    expect(Array.from(anchorPoint)).toEqual([2, 0, 0])
  })

  it('reverse order — centre then a rim point -> Radius, anchored at the SECOND click\'s rim point', () => {
    // This is the case Kurt explicitly could not create under the old code.
    const { scene, radialCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap(CENTER), RAY)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    expect(onCreated).toHaveBeenCalledTimes(1)
    expect(radialCalls.length).toBe(1)
    const [, , anchorPoint, kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('radius')
    expect(Array.from(anchorPoint)).toEqual([2, 0, 0])
  })

  it('centre clicked twice is a degenerate no-op, not a commit', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap(CENTER), RAY)
    tool.onPointerDown(makeSnap(CENTER), RAY)
    expect(radialCalls.length).toBe(0)
    expect(onCreated).not.toHaveBeenCalled()
    // Gesture is still pending — a real rim click after the accidental
    // double-center-click still completes it.
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    expect(radialCalls.length).toBe(1)
  })

  it('rim then the antipodal rim point (through the centre) -> Diameter', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap(RIM_180_ANTIPODAL), RAY)
    expect(onCreated).toHaveBeenCalledTimes(1)
    expect(radialCalls.length).toBe(1)
    const [, , anchorPoint, kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('diameter')
    expect(Array.from(anchorPoint)).toEqual([2, 0, 0])
  })

  it('rim then a non-antipodal rim point -> ordinary LINEAR dimension of that chord', () => {
    const { scene, linearCalls, radialCalls } = makeWasmScene()
    const { tool, onCreated } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap(RIM_90), RAY) // 90 degrees around — not antipodal to RIM_0
    // Not a commit yet — this becomes an ordinary linear dimension, which
    // still needs the drag-offset + third click.
    expect(radialCalls.length).toBe(0)
    expect(onCreated).not.toHaveBeenCalled()
    expect(tool.statusHint()).toMatch(/drag/i)

    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 5 }), RAY)

    expect(linearCalls.length).toBe(1)
    expect(onCreated).toHaveBeenCalledTimes(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array,
    ]
    expect(Array.from(aPoint)).toEqual([2, 0, 0])
    expect(Array.from(bPoint)).toEqual([0, 2, 0])
  })

  it('rim then an unrelated, identifiable point elsewhere -> ordinary linear dimension to it', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap(OTHER_OBJECT), RAY)
    expect(tool.statusHint()).toMatch(/drag/i)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 5 }), RAY)
    expect(linearCalls.length).toBe(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array,
    ]
    expect(Array.from(aPoint)).toEqual([2, 0, 0])
    expect(Array.from(bPoint)).toEqual([9, 9, 0])
  })

  it('centre first, then an unrelated point -> ordinary linear dimension from the centre', () => {
    const { scene, linearCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(CENTER), RAY)
    tool.onPointerDown(makeSnap(FREE_SPACE), RAY)
    expect(tool.statusHint()).toMatch(/drag/i)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 5 }), RAY)
    expect(linearCalls.length).toBe(1)
    const [, , aPoint, , , bPoint] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array,
    ]
    expect(Array.from(aPoint)).toEqual([0, 0, 0])
    expect(Array.from(bPoint)).toEqual([6, 6, 0])
  })

  it('antipodal tolerance: just inside 2% of radius still counts as a diameter', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    // radius 2 -> tolerance 0.04 (antipodalTolerance). A chord between angle
    // 0 and angle (pi - phi) on a radius-r circle sits `r*sin(phi/2)` from
    // the centre — solve for phi that lands exactly at 0.9x the tolerance,
    // just inside it.
    const rim1 = rimPointNearAntipodal(2, 0.9 * ANTIPODAL_TOL_R2)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap({ ...rim1, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 6n }), RAY)
    expect(radialCalls.length).toBe(1)
    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('diameter')
  })

  it('antipodal tolerance: just outside 2% of radius is an ordinary linear chord', () => {
    const { scene, linearCalls, radialCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    const rim1 = rimPointNearAntipodal(2, 1.1 * ANTIPODAL_TOL_R2)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap({ ...rim1, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 6n }), RAY)
    expect(radialCalls.length).toBe(0)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 5 }), RAY)
    expect(linearCalls.length).toBe(1)
  })

  it('Tab overrides the off-curve default kind (Diameter), and the toggle rebuilds the geometry', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onKey(makeKeyEvent('Tab')) // radius (default) -> diameter
    tool.onPointerDown(makeSnap(FREE_SPACE), RAY)
    expect(radialCalls.length).toBe(1)
    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('diameter')
  })

  it('Tab does not override an EXPLICIT centre click\'s definite Radius kind', () => {
    const { scene, radialCalls } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onKey(makeKeyEvent('Tab')) // sets a diameter preference...
    tool.onPointerDown(makeSnap(CENTER), RAY) // ...but this is an explicit centre click.
    expect(radialCalls.length).toBe(1)
    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('radius')
  })

  it('the default kind is Diameter for a full (closed) circle chain', () => {
    const { scene, radialCalls } = makeWasmScene()
    // A closed 2-edge ring: edge 3 goes rim(2,0,0)->rim(-2,0,0), edge 7 goes
    // back the other way, closing the loop.
    ;(scene as unknown as { sketch_curve_edges: ReturnType<typeof vi.fn> }).sketch_curve_edges = vi.fn(() => [3n, 7n])
    ;(scene as unknown as { sketch_edge_endpoints: ReturnType<typeof vi.fn> }).sketch_edge_endpoints = vi.fn(
      (_s: bigint, e: bigint) =>
        e === 3n ? new Float64Array([2, 0, 0, -2, 0, 0]) : new Float64Array([-2, 0, 0, 2, 0, 0]),
    )
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap(FREE_SPACE), RAY)
    expect(radialCalls.length).toBe(1)
    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('diameter')
  })

  it('the default kind stays Radius for an open (arc) chain', () => {
    const { scene, radialCalls } = makeWasmScene()
    // An open 1-edge chain: no other edge's start matches its end -> arc.
    ;(scene as unknown as { sketch_curve_edges: ReturnType<typeof vi.fn> }).sketch_curve_edges = vi.fn(() => [3n])
    ;(scene as unknown as { sketch_edge_endpoints: ReturnType<typeof vi.fn> }).sketch_edge_endpoints = vi.fn(
      () => new Float64Array([2, 0, 0, 0, 2, 0]),
    )
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(RIM_0), RAY)
    tool.onPointerDown(makeSnap(FREE_SPACE), RAY)
    expect(radialCalls.length).toBe(1)
    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('radius')
  })

  it('the default kind is STILL Diameter for a full circle when the first click lands on an analytic quadrant point (no sample edge)', () => {
    // Regression: `isFullCircle` originally needed a specific EDGE handle
    // from the resolving snap (`sketch_curve_chain`), which a quadrant/
    // centre analytic snap never carries (`elementKind: 'sketch-curve'` has
    // no `element`) — a very natural way to click a circle, since exact
    // axis-aligned points are exactly what inference snaps TO. That silently
    // fell back to the conservative "arc" default every time, which this
    // pins against: `sketch_curve_edges` takes the CURVE handle directly, so
    // it works with no edge in the snap at all.
    const { scene, radialCalls } = makeWasmScene()
    ;(scene as unknown as { sketch_curve_edges: ReturnType<typeof vi.fn> }).sketch_curve_edges = vi.fn(() => [3n, 7n])
    ;(scene as unknown as { sketch_edge_endpoints: ReturnType<typeof vi.fn> }).sketch_edge_endpoints = vi.fn(
      (_s: bigint, e: bigint) =>
        e === 3n ? new Float64Array([2, 0, 0, -2, 0, 0]) : new Float64Array([-2, 0, 0, 2, 0, 0]),
    )
    const quadrantSnap: Snap = { x: 2, y: 0, z: 0, kind: 'endpoint', elementKind: 'sketch-curve', sketch: 11n, sketchCurve: 9n }
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap(quadrantSnap), RAY)
    tool.onPointerDown(makeSnap(FREE_SPACE), RAY)
    expect(radialCalls.length).toBe(1)
    const [, , , kind] = radialCalls[0] as [number, bigint, Float64Array, string]
    expect(kind).toBe('diameter')
  })
})

describe('DimensionTool — wasm refusal toasts route through friendlyErrorText', () => {
  // Both commit paths (`_commitLinear`/`_commitRadial`) used to toast the raw
  // `err.message` on a wasm-thrown refusal — the one call site in this tool
  // that skipped the plain-language mapping every sibling annotation call
  // site (Viewport.tsx's annotation handlers) already goes through. A
  // genuine `"CODE: message"` throw, exactly like the wasm boundary emits
  // (docs/dev/DEVELOPMENT.md B3), must come out the OTHER end mapped to its
  // `DESCRIPTIONS` copy, not the raw code/message.

  it('a linear-dimension refusal toasts the mapped copy, not the raw wasm message', () => {
    const { scene } = makeWasmScene()
    const rawMessage = 'DegenerateAnnotation: annotation geometry is degenerate'
    ;(scene.add_linear_dimension as ReturnType<typeof vi.fn>).mockImplementation(() => {
      throw new Error(rawMessage)
    })
    const { tool, onToast, onCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // commit — wasm throws

    expect(onCreated).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    const toasted = (onToast as ReturnType<typeof vi.fn>).mock.calls[0][0] as string
    expect(toasted).toContain(kernelErrorMessage('DegenerateAnnotation', 'annotation geometry is degenerate'))
    expect(toasted).not.toContain(rawMessage)
    expect(toasted).not.toContain('DegenerateAnnotation:')
  })

  it('a radial-dimension refusal toasts the mapped copy, not the raw wasm message', () => {
    const { scene } = makeWasmScene()
    const rawMessage = 'DegenerateAnnotation: annotation geometry is degenerate'
    ;(scene.add_radial_dimension as ReturnType<typeof vi.fn>).mockImplementation(() => {
      throw new Error(rawMessage)
    })
    const { tool, onToast, onCreated } = makeTool(scene)
    const CIRCLE_SNAP: Partial<Snap> = {
      x: 2, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 3n,
    }

    tool.onPointerDown(makeSnap(CIRCLE_SNAP), RAY)
    tool.onPointerMove(makeSnap({ x: 4, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0 }), RAY) // commit — wasm throws

    expect(onCreated).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    const toasted = (onToast as ReturnType<typeof vi.fn>).mock.calls[0][0] as string
    expect(toasted).toContain(kernelErrorMessage('DegenerateAnnotation', 'annotation geometry is degenerate'))
    expect(toasted).not.toContain(rawMessage)
    expect(toasted).not.toContain('DegenerateAnnotation:')
  })
})

describe('DimensionTool — plane-locked preview color (maintainer playtest: color in-progress lines by the active plane lock)', () => {
  function previewMaterial(tool: DimensionTool): THREE.LineBasicMaterial | null {
    const line = (tool as unknown as { previewLine: THREE.LineSegments | null }).previewLine
    return line === null ? null : (line.material as THREE.LineBasicMaterial)
  }

  const NEUTRAL_HEX = new THREE.LineBasicMaterial({ color: PREVIEW_COLOR }).color.getHex()

  function lockedHex(axis: 0 | 1 | 2): number {
    return new THREE.LineBasicMaterial({ color: axisColorsForTheme(getResolvedTheme())[axis] }).color.getHex()
  }

  it('an unlocked have-a baseline preview uses the neutral color, same as before this fix', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0, kind: 'ground' }), RAY)
    expect(previewMaterial(tool)?.color.getHex()).toBe(NEUTRAL_HEX)
  })

  it('a plane-locked have-a baseline preview uses the locked axis color (red = X)', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onKey(makeKeyEvent('ArrowRight')) // locks axis 0 (red)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0, kind: 'ground' }), RAY)
    expect(previewMaterial(tool)?.color.getHex()).toBe(lockedHex(0))
    expect(previewMaterial(tool)?.color.getHex()).not.toBe(NEUTRAL_HEX)
  })

  it('a plane-locked have-b dimension/extension-line preview uses the locked axis color (blue = Z)', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onKey(makeKeyEvent('ArrowUp')) // locks axis 2 (blue), mid-gesture
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    expect(previewMaterial(tool)?.color.getHex()).toBe(lockedHex(2))
  })

  it('releasing the lock (same arrow again) reverts the have-b preview to the neutral color', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onKey(makeKeyEvent('ArrowUp')) // same arrow again -> unlock
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)
    expect(previewMaterial(tool)?.color.getHex()).toBe(NEUTRAL_HEX)
  })

  it('a plane ADOPTED from a hovered sketch (no arrow lock) keeps the neutral preview color', () => {
    const { scene } = makeWasmScene()
    ;(scene as unknown as { pick_sketch: ReturnType<typeof vi.fn> }).pick_sketch = vi.fn(() => 21n)
    ;(scene as unknown as { sketch_plane: ReturnType<typeof vi.fn> }).sketch_plane = vi.fn(
      (_s: bigint) => new Float64Array([0, 0, 3, 0, 1, 0]),
    )
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3, kind: 'ground' }), RAY)
    expect(previewMaterial(tool)?.color.getHex()).toBe(NEUTRAL_HEX)
  })

  it('the radial gesture preview stays neutral even with an idle-armed lock — arrows are left alone in have-curve', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onKey(makeKeyEvent('ArrowRight'))
    const CIRCLE_SNAP: Partial<Snap> = {
      x: 2, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 11n, element: 3n,
    }
    tool.onPointerDown(makeSnap(CIRCLE_SNAP), RAY)
    tool.onPointerMove(makeSnap({ x: 4, y: 0, z: 0 }), RAY)
    expect(previewMaterial(tool)?.color.getHex()).toBe(NEUTRAL_HEX)
  })
})

describe('DimensionTool — dimension-row alignment snap (SketchUp parity, maintainer playtest)', () => {
  /** One existing linear dimension the fake wasm scene reports: baseline
   *  (0,0,0)->(4,0,0), offset (0,1,0) — its own drawn line runs
   *  (0,1,0)->(4,1,0), on the flat (Z-normal) plane. */
  function withExistingDimension(scene: WasmScene): void {
    const s = scene as unknown as {
      annotation_ids: ReturnType<typeof vi.fn>
      annotation_kind: ReturnType<typeof vi.fn>
      annotation_anchor_point: ReturnType<typeof vi.fn>
      annotation_offset: ReturnType<typeof vi.fn>
      annotation_plane: ReturnType<typeof vi.fn>
    }
    s.annotation_ids = vi.fn(() => new BigUint64Array([1n]))
    s.annotation_kind = vi.fn((id: bigint) => (id === 1n ? 'linear' : undefined))
    s.annotation_anchor_point = vi.fn((id: bigint, which: number) => {
      if (id !== 1n) return undefined
      return which === 0 ? new Float64Array([0, 0, 0]) : new Float64Array([4, 0, 0])
    })
    s.annotation_offset = vi.fn((id: bigint) => (id === 1n ? new Float64Array([0, 1, 0]) : undefined))
    s.annotation_plane = vi.fn((id: bigint) => (id === 1n ? new Float64Array([0, 0, 0, 0, 0, 1]) : undefined))
  }

  /** A ray hitting the flat (z=0) plane at `(x, y, 0)`, straight down —
   *  matches `_workingPlane`'s locked-plane pierce-point computation. */
  function downRayAt(x: number, y: number): Ray {
    return { origin: [x, y, 5], direction: [0, 0, -1] }
  }

  it('a drag within tolerance of a parallel, coplanar existing dimension snaps collinear with it', () => {
    const { scene, linearCalls } = makeWasmScene()
    withExistingDimension(scene)
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowUp')) // locks the flat (Z-normal) plane — no camera needed
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0, kind: 'ground' }), RAY)
    expect(tool.statusHint()).toMatch(/drag/i)

    // Drag 2cm off the existing dimension's own line (y=1) — well within the
    // no-camera fallback tolerance (`ALIGN_FALLBACK_TOLERANCE_M` = 0.15m).
    tool.onPointerMove(makeSnap({ x: 2, y: 1.02, z: 0, kind: 'ground' }), downRayAt(2, 1.02))
    expect(tool.lastSnap?.kind).toBe('aligned')

    tool.onPointerDown(makeSnap({ x: 2, y: 1.02, z: 0, kind: 'ground' }), downRayAt(2, 1.02))
    expect(linearCalls.length).toBe(1)
    const [, , , , , , offset] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array,
    ]
    // Snapped to the EXISTING dimension's own offset (1), not the raw 1.02
    // the cursor was actually at.
    expect(offset[1]).toBeCloseTo(1, 9)
  })

  it('a drag farther than the tolerance does not snap — the natural offset is used as-is', () => {
    const { scene, linearCalls } = makeWasmScene()
    withExistingDimension(scene)
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0, kind: 'ground' }), RAY)

    // 50cm off the existing dimension's line — outside the fallback tolerance.
    tool.onPointerMove(makeSnap({ x: 2, y: 1.5, z: 0, kind: 'ground' }), downRayAt(2, 1.5))
    expect(tool.lastSnap?.kind).not.toBe('aligned')

    tool.onPointerDown(makeSnap({ x: 2, y: 1.5, z: 0, kind: 'ground' }), downRayAt(2, 1.5))
    expect(linearCalls.length).toBe(1)
    const [, , , , , , offset] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array,
    ]
    expect(offset[1]).toBeCloseTo(1.5, 9)
  })

  it('moving away releases the snap immediately — no hysteresis', () => {
    const { scene } = makeWasmScene()
    withExistingDimension(scene)
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0, kind: 'ground' }), RAY)

    tool.onPointerMove(makeSnap({ x: 2, y: 1.02, z: 0, kind: 'ground' }), downRayAt(2, 1.02))
    expect(tool.lastSnap?.kind).toBe('aligned')

    // Move well away — the snap must release on THIS event, with no memory
    // of having just been aligned.
    tool.onPointerMove(makeSnap({ x: 2, y: 3, z: 0, kind: 'ground' }), downRayAt(2, 3))
    expect(tool.lastSnap?.kind).not.toBe('aligned')
  })

  it('a candidate whose line is not parallel to the new baseline is never offered, however close the cursor sits', () => {
    const { scene, linearCalls } = makeWasmScene()
    const s = scene as unknown as {
      annotation_ids: ReturnType<typeof vi.fn>
      annotation_kind: ReturnType<typeof vi.fn>
      annotation_anchor_point: ReturnType<typeof vi.fn>
      annotation_offset: ReturnType<typeof vi.fn>
      annotation_plane: ReturnType<typeof vi.fn>
    }
    // A PERPENDICULAR existing dimension: baseline (2,0,0)->(2,4,0), offset 0
    // — its own line runs straight through where the new drag will sit.
    s.annotation_ids = vi.fn(() => new BigUint64Array([1n]))
    s.annotation_kind = vi.fn(() => 'linear')
    s.annotation_anchor_point = vi.fn((_id: bigint, which: number) =>
      which === 0 ? new Float64Array([2, 0, 0]) : new Float64Array([2, 4, 0]),
    )
    s.annotation_offset = vi.fn(() => new Float64Array([0, 0, 0]))
    s.annotation_plane = vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1]))
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowUp'))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 0, kind: 'ground' }), RAY) // new baseline along +X

    tool.onPointerMove(makeSnap({ x: 2, y: 0.01, z: 0, kind: 'ground' }), downRayAt(2, 0.01))
    expect(tool.lastSnap?.kind).not.toBe('aligned')

    tool.onPointerDown(makeSnap({ x: 2, y: 0.01, z: 0, kind: 'ground' }), downRayAt(2, 0.01))
    expect(linearCalls.length).toBe(1)
    const [, , , , , , offset] = linearCalls[0] as [
      number, bigint, Float64Array, number, bigint, Float64Array, Float64Array,
    ]
    expect(offset[1]).toBeCloseTo(0.01, 9) // natural offset, not snapped
  })
})
