/**
 * PushPullTool unit tests — Path A (object face) and Path B (sketch region,
 * now resolved via `pick_sketch_region` across ALL live sketches rather than
 * the old single "active sketch handle" bookkeeping — "sketches are
 * first-class interactable"). Mirrors the fake-WasmScene pattern used by
 * CircleTool.test.ts/ArcTool.test.ts.
 */
import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import * as THREE from 'three'
import { PushPullTool } from './PushPullTool'
import type { Snap } from './types'
import { setLengthUnit, getLengthUnit, setPrecision, DEFAULT_PRECISION } from '../settings/units'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { ctxOfPlane } from './planeUnits'
import { planeUnitsMock, type MockListing } from './planeUnitsMock'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** A fake `FacePickJs` returning the seeded handles, `depth` metres along
 *  the (unit) ray — 1 unless a test is about depth ordering. */
function makeFacePick(object: bigint, face: bigint, instance?: bigint, depth = 1) {
  return {
    object: () => object,
    face: () => face,
    depth: () => depth,
    instance: () => instance,
    free: vi.fn(),
  }
}

/** A fake `SketchRegionPickJs` returning the seeded handles, `depth` in ray
 *  parameter units (RAY's direction is unit, so metres too). */
function makeRegionPick(sketch: bigint, region: bigint, depth = 1, detached = false) {
  return {
    sketch: () => sketch,
    region: () => region,
    depth: () => depth,
    detached: () => detached,
    free: vi.fn(),
  }
}

function makeWasmScene(opts: {
  facePick?: ReturnType<typeof makeFacePick>
  regionPick?: ReturnType<typeof makeRegionPick>
  /** `pick_sketch_region_in_instance`'s own result, when it must differ from
   *  `regionPick` (defaults to `regionPick` otherwise). */
  regionPickInInstance?: ReturnType<typeof makeRegionPick>
  /** node_parent(0, id) result per object (a grouped object's group id). */
  parents?: Map<bigint, bigint>
  /** Unit normal `face_normal` reports for the picked face (default +Z). */
  faceNormal?: [number, number, number]
  /** `face_plane` result for the picked face — `[px,py,pz,nx,ny,nz]`
   *  (default: origin, normal +Z) — `snapConstraint`'s idle-hover Path A. */
  facePlane?: [number, number, number, number, number, number]
  /** `sketch_plane` result for the region's sketch — `[px,py,pz,nx,ny,nz]`
   *  (default: ground, point at origin, normal +Z). `undefined` simulates a
   *  stale handle (the pick must then miss, not fall back to ground). */
  sketchPlane?: [number, number, number, number, number, number] | undefined
  /** Handles `component_member_sketches` should report as live members of
   *  the active component (component-edit-parity.md phase A2). */
  memberSketches?: bigint[]
  componentThroughResults?: bigint[]
  /** A component/group editing session is open (`session_stack` non-empty). */
  sessionOpen?: boolean
  /** Plane listings by context (the face / sketch plane's `plane_units_json`).
   *  Absent: the scene lists nothing and the struck face stands. */
  listings?: Record<string, MockListing>
} = {}): WasmScene {
  const faceNormal = opts.faceNormal ?? [0, 0, 1]
  const facePlane = opts.facePlane ?? [0, 0, 0, ...faceNormal]
  const sketchPlane = 'sketchPlane' in opts ? opts.sketchPlane : [0, 0, 0, 0, 0, 1]
  return {
    ...(opts.listings !== undefined ? planeUnitsMock(opts.listings) : {}),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => opts.facePick),
    session_stack: vi.fn(() => (opts.sessionOpen === true ? [{ kind: 'instance', id: 77n }] : [])),
    // `pick_sketch_region` only ever walks WORLD-tree sketches — Path B calls
    // its `_in_instance` sibling instead while inside an instance context, so
    // the two are seeded independently (`regionPick` covers whichever one the
    // test actually expects to fire; see the instance-context describe block
    // below for a `regionPickInInstance`-specific seed).
    pick_sketch_region: vi.fn(() => opts.regionPick),
    pick_sketch_region_in_instance: vi.fn(() => opts.regionPickInInstance ?? opts.regionPick),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])), // identity
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
    face_normal: vi.fn(() => new Float64Array(faceNormal)),
    face_plane: vi.fn(() => new Float64Array(facePlane)),
    sketch_plane: vi.fn(() => (sketchPlane !== undefined ? new Float64Array(sketchPlane) : undefined)),
    region_boundary: vi.fn(() => new Float32Array([])),
    face_boundary: vi.fn(() => new Float32Array([])),
    extrude_region: vi.fn(() => 55n),
    extrude_region_in_instance: vi.fn(() => 55n),
    push_pull: vi.fn(() => ({
      is_through: () => false,
      result_objects: () => new BigUint64Array([]),
      free: vi.fn(),
    })),
    push_pull_in_component: vi.fn(() => ({
      is_through: () => opts.componentThroughResults !== undefined,
      result_objects: () => new BigUint64Array(opts.componentThroughResults ?? []),
      free: vi.fn(),
    })),
    component_member_sketches: vi.fn(() => new BigUint64Array(opts.memberSketches ?? [])),
    extrude_face_as_new_object: vi.fn(() => 42n),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const onExtrudeAsNewModeChange = vi.fn()
  const tool = new PushPullTool(
    scene,
    preview,
    onCommit,
    onToast,
    onMeasurement,
    onExtrudeAsNewModeChange,
  )
  return { tool, preview, onCommit, onToast, onMeasurement, onExtrudeAsNewModeChange }
}

describe('PushPullTool — Path A (object face)', () => {
  it('two clicks on a face commit push_pull with the picked object/face', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    expect(scene.push_pull).toHaveBeenCalledTimes(1)
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(4n)
    expect(call[2]).toBeCloseTo(2)
    expect(onCommit).toHaveBeenCalledWith(3n)
    expect(onToast).not.toHaveBeenCalled()
  })

  // Deliberate contract change (selection-UX overhaul, policy consistency
  // with the draw tools — see faceDraw.ts): at the top level only PLAIN
  // objects are directly editable. Faces inside a group or a component
  // instance keep their explicit double-click editing step, for push/pull
  // exactly as for drawing.
  it('a GROUPED object\'s face is not push/pullable from outside its group', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      parents: new Map([[3n, 9n]]), // object 3 lives inside group 9
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(tool.capturingInput()).toBe(false) // no drag started
    expect(scene.push_pull).not.toHaveBeenCalled()
  })

  it('instanced (component) geometry is not push/pullable from outside its instance', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 12n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(tool.capturingInput()).toBe(false)
    expect(scene.push_pull).not.toHaveBeenCalled()
  })

  // Fail-closed, not fail-open: an INELIGIBLE face under the cursor must
  // CONSUME the click — never fall through to Path B, where a sketch region
  // along the same ray (a ground sketch behind the group — ordinary
  // mid-modeling state) would silently start a drag and extrude geometry
  // the user did not aim at.
  it('an ineligible (grouped) face with a region on the same ray consumes the click — no drag, no extrude, hint shown', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      parents: new Map([[3n, 9n]]),          // grouped → ineligible
      regionPick: makeRegionPick(50n, 51n, 2),  // live region behind it (farther along the ray)
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2 }), RAY)

    expect(tool.capturingInput()).toBe(false)      // no drag ever started
    // (The region pick now runs alongside the face pick so a region IN
    // FRONT of a face can win — GitHub issue 13 — but a region BEHIND the
    // ineligible face still never extrudes.)
    expect(scene.extrude_region).not.toHaveBeenCalled()
    expect(scene.push_pull).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(2)       // one explicable refusal per click
    expect(String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])).toContain('group')
  })

  it('an ineligible (instanced) face over a region consumes the click with the component hint', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, 12n),
      regionPick: makeRegionPick(50n, 51n, 2), // under it, farther along the ray
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(scene.extrude_region).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
    expect(String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])).toContain('component')
    // The refusal is GUIDANCE, tagged so the app emphasizes the toast
    // rather than filing it as an ordinary warning (bug report: the plain
    // bottom toast was easy to miss).
    expect((onToast as ReturnType<typeof vi.fn>).mock.calls[0][1]).toBe('EditContextHint')
  })

  // Part A of the live-siblings bug report: a person coming from SketchUp
  // pushes on a copied component, sees nothing happen, and reads the
  // viewport as broken. The way in now shows BEFORE the click: hovering a
  // component face outside its edit session puts the same hint beside the
  // cursor (Tool.hoverHint) and in the status bar (statusHint).
  it('hovering a component face outside its edit session explains itself before any click', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 12n) })
    const { tool, onToast } = makeTool(scene)

    expect(tool.hoverHint()).toBeNull()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)

    expect(tool.hoverHint()).toContain('double-click it to edit the component')
    expect(tool.statusHint()).toContain('double-click it to edit the component')
    expect(onToast).not.toHaveBeenCalled() // a hover never toasts
  })

  it('with a session open, the hint names Esc as the way out instead of a dead-end double-click', () => {
    // Double-clicking another copy while a component is open for editing
    // is ignored by the viewport (outside the session scope), so the hover
    // and the refused click must not promise it.
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 12n), sessionOpen: true })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    expect(tool.hoverHint()).toContain('press Esc to finish editing the open component first')
    expect(tool.hoverHint()).not.toContain('double-click it to edit')
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    expect(String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])).toContain('press Esc')
  })

  it('the hover hint is dropped on cancel, on a context change, and when the pointer leaves the canvas', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 12n) })
    const { tool } = makeTool(scene)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    expect(tool.hoverHint()).not.toBeNull()
    tool.onPointerLeave()
    expect(tool.hoverHint()).toBeNull()
    expect(tool.statusHint()).toContain('Click a face')

    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    expect(tool.hoverHint()).not.toBeNull()
    tool.cancel()
    expect(tool.hoverHint()).toBeNull()

    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    expect(tool.hoverHint()).not.toBeNull()
    tool.setEditContext({ kind: 'instance', id: 12n, component: 100n })
    expect(tool.hoverHint()).toBeNull()
  })

  it('the hover hint clears over a pushable face and over nothing', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 12n) })
    const { tool } = makeTool(scene)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    expect(tool.hoverHint()).not.toBeNull()

    // A fresh ray object: the per-ray pick cache must not serve the old
    // instanced pick for a ray that now hits a plain face.
    const plain = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool: tool2 } = makeTool(plain)
    tool2.onPointerMove(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    expect(tool2.hoverHint()).toBeNull()
    expect(tool2.statusHint()).toContain('Click a face')

    const empty = makeWasmScene({})
    const { tool: tool3 } = makeTool(empty)
    tool3.onPointerMove(null, RAY)
    expect(tool3.hoverHint()).toBeNull()
  })

  it('free drawing lying ON an ineligible face (drawn on a component from outside) takes the click and extrudes', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, 12n),
      regionPick: makeRegionPick(50n, 51n), // same depth: the drawing sits on the face
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(tool.capturingInput()).toBe(true)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('inside a GROUP context the refusal is the scoped hint, never the group default', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onToast } = makeTool(scene)
    // Mirror the real Viewport wiring for a deepest 'group' context: the two
    // id channels stay null (they only carry object/instance contexts) and
    // the injected context-path predicate does the rejecting.
    tool.setEditContext({ kind: 'group', id: 9n })
    tool.setFaceEligibility(() => false)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(tool.capturingInput()).toBe(false)
    expect(scene.push_pull).not.toHaveBeenCalled()
    const msg = String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])
    // The clicked face may not be in any group; 'double-click to enter'
    // would be a lie here — the correct guidance is stepping out.
    expect(msg).toContain('step out')
    expect(msg).not.toContain('double-click')
  })

  it('inside a scoped context an out-of-scope COMPONENT face also gets the scoped hint (double-click cannot enter it from here)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 12n) })
    const { tool, onToast } = makeTool(scene)
    tool.setEditContext({ kind: 'group', id: 9n })
    tool.setFaceEligibility(() => false)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    const msg = String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])
    expect(msg).toContain('step out')
  })

  it('in-context: a foreign face consumes the click with the scoped-editing hint', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(999n, 4n) })
    const { tool, onToast } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(tool.capturingInput()).toBe(false)
    expect(onToast).toHaveBeenCalledTimes(1)
  })
})

describe('PushPullTool — Path B (sketch region, any live sketch)', () => {
  it('extrudes a region resolved by pick_sketch_region, even from a sketch handle the tool never saw before', () => {
    // 99n stands in for "not the most recently drawn sketch" — the tool has no
    // per-tool bookkeeping of it at all anymore; pick_sketch_region is the only
    // source of truth.
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'endpoint' }), RAY)

    expect(scene.pick_sketch_region).toHaveBeenCalled()
    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(99n)
    expect(call[1]).toBe(7n)
    expect(call[2]).toBeCloseTo(3)
    expect(onCommit).toHaveBeenCalledWith(55n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('is suppressed inside an editing context (region extrusion is a top-level act)', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 1n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(scene.pick_sketch_region).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
  })

  it('a total miss (no face, no region) leaves the tool idle', () => {
    const scene = makeWasmScene({ facePick: undefined, regionPick: undefined })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(tool.capturingInput()).toBe(false)
  })

  // A typed push/pull takes its DIRECTION from the live drag but its MAGNITUDE
  // from the typed value. With no meaningful drag (click a ground region, type
  // an exact distance), the ray-projected drag distance is essentially zero and
  // its sign is pure floating-point noise — on the order of the ~1e-15 m low-bit
  // wobble the camera projection matrix carries, which varies per page load.
  // Reading that raw sign made a 1 m ground box extrude DOWNWARD (z in [-1,0])
  // at random ~40% of the time, which then broke every downstream step that
  // assumed the box sat at z in [0,1] (the Follow Me "guide Scenario 2" flake).
  it('a typed region push/pull does NOT flip downward on a sub-tolerance (noise) drag', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    // The spurious "drag": a hard snap a sub-picometer BELOW the anchor, so the
    // tool reads a distance of -1e-15 — exactly the noise the bug amplified.
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: -1e-15, kind: 'endpoint' }), RAY)
    tool.onKey({ key: '1' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0]
    // Outward/up — never a coin-flip into the ground.
    expect(call[2]).toBeGreaterThan(0)
  })

  it('a deliberate inward drag still flips a typed push/pull negative', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    // A genuine inward pull (half a meter below the anchor) is well past the
    // noise threshold and must still invert the typed magnitude.
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: -0.5, kind: 'endpoint' }), RAY)
    tool.onKey({ key: '1' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[2]).toBeLessThan(0)
  })
})

// Sketches on any plane (Phase 1, the sketch-planes design §3): the
// region target's normal must come from the sketch's OWN plane
// (`sketch_plane`), not a hardcoded [0,0,1] — it drives the drag axis, the
// ghost preview, and (unchanged) the sign the kernel extrudes along.
describe('PushPullTool — Path B on a rotated sketch (sketch_plane normal)', () => {
  it('drags and commits along the sketch plane\'s own normal, not [0,0,1]', () => {
    const regionPick = makeRegionPick(99n, 7n)
    // A vertical sketch: plane through the origin with normal +X.
    const scene = makeWasmScene({
      facePick: undefined,
      regionPick,
      sketchPlane: [0, 0, 0, 1, 0, 0],
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    // A ray traveling -X so it can reach the x=0 plane and a snap that lands
    // 3 m along the normal (+X) from the anchor.
    const rayX: Ray = { origin: [5, 0, 0], direction: [-1, 0, 0] }
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), rayX)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0, kind: 'endpoint' }), rayX)

    expect(scene.sketch_plane).toHaveBeenCalledWith(99n)
    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(99n)
    expect(call[1]).toBe(7n)
    // Signed distance along +X (the sketch's normal), not along Z.
    expect(call[2]).toBeCloseTo(3)
    expect(onCommit).toHaveBeenCalledWith(55n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a stale sketch handle (sketch_plane undefined) is treated as a pick miss, not a ground fallback', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick, sketchPlane: undefined })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(scene.sketch_plane).toHaveBeenCalledWith(99n)
    expect(tool.capturingInput()).toBe(false)
    expect(scene.extrude_region).not.toHaveBeenCalled()
  })
})

describe('PushPullTool — typed sign precedence (face targets)', () => {
  // _commitFromTyped resolves the extrude DIRECTION by a fixed precedence, most
  // deliberate signal first:
  //   1. An explicit typed sign (a leading `-`, which editLengthBuffer keeps as
  //      a sign) wins outright — the user spelled out a recess, and that beats
  //      both the live drag and the outward default.
  //   2. Otherwise the (positive) magnitude takes its direction from a live drag
  //      once it clears MIN_INWARD_DRAG_M, else defaults OUTWARD (the 2d7883d
  //      behavior that pins against sub-tolerance camera-projection noise).
  // The 2d7883d tests only covered a `region` target with a positive typed
  // value; these pin the FACE target across orientations and the explicit-sign
  // cases the earlier fix left ambiguous.

  /** Feed a VCB string one key at a time, then commit on Enter. */
  const typeAndCommit = (tool: PushPullTool, buf: string): void => {
    for (const ch of buf) tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
  }

  const pushPullDistance = (scene: WasmScene): number =>
    (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0][2]

  // Explicit-negative typed + NO drag: the "click a face, type an exact depth,
  // commit" gesture, where the live drag distance is exactly 0 and cannot
  // supply a direction. The typed `-` is the whole intent — recess inward.
  for (const normal of [[0, 0, 1], [0, 0, -1], [1, 0, 0]] as [number, number, number][]) {
    it(`explicit-negative typed + no drag on a [${normal.join(',')}] face recesses (push_pull negative)`, () => {
      const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n), faceNormal: normal })
      const { tool } = makeTool(scene)

      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
      // No onPointerMove — the drag distance stays 0.
      typeAndCommit(tool, '-0.5')

      expect(scene.push_pull).toHaveBeenCalledTimes(1)
      // Negative = inward along the face normal = the recess the user asked for.
      expect(pushPullDistance(scene)).toBeLessThan(0)
    })
  }

  it('the recessed magnitude equals the typed magnitude (|-0.5| = 0.5 m inward)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndCommit(tool, '-0.5')

    expect(pushPullDistance(scene)).toBeCloseTo(-0.5)
  })

  it('unsigned-positive typed + no drag extrudes OUTWARD (2d7883d default preserved)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndCommit(tool, '0.5')

    expect(pushPullDistance(scene)).toBeGreaterThan(0)
  })

  it('unsigned-positive typed + genuine inward drag inverts (2d7883d behavior preserved)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    // Half a metre inward along +Z — well past MIN_INWARD_DRAG_M.
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: -0.5, kind: 'endpoint' }), RAY)
    typeAndCommit(tool, '0.5')

    expect(pushPullDistance(scene)).toBeLessThan(0)
  })

  it('explicit-negative typed beats an OUTWARD live drag (the typed sign wins)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    // The cursor is dragging OUTWARD (+Z), yet the explicit typed `-` must win.
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0.5, kind: 'endpoint' }), RAY)
    typeAndCommit(tool, '-0.5')

    expect(pushPullDistance(scene)).toBeLessThan(0)
  })
})

describe('PushPullTool — status hint', () => {
  it('switches from pick guidance to extrude guidance and back across a commit', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool } = makeTool(scene)

    expect(tool.statusHint()).toContain('Click a face')
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.statusHint()).toContain('click to commit')
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    // Committed: the retype window offers to redo the push/pull at a typed
    // distance; Escape closes it and the pick guidance returns.
    expect(tool.statusHint()).toContain('redo the push/pull')
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.statusHint()).toContain('Click a face')
  })
})

describe('PushPullTool — instance editing context (component-edit-parity.md phase A2)', () => {
  const INSTANCE = 42n
  const COMPONENT = 5n

  it('a member face routes to push_pull_in_component, never the world push_pull', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, INSTANCE) })
    const { tool, onCommit } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    expect(scene.push_pull_in_component).toHaveBeenCalledTimes(1)
    const call = (scene.push_pull_in_component as ReturnType<typeof vi.fn>).mock.calls[0]
    // Delta-review fix: the kernel needs the INSTANCE (not the bare
    // component) to map the WORLD drag distance through its pose — the same
    // `_in_instance` convention `extrude_region_in_instance` already follows.
    // This replaces a prior assertion that pinned `component` as the first
    // argument, which was the raw-distance contract this fix corrects.
    expect(call[0]).toBe(INSTANCE)
    expect(call[1]).toBe(3n)
    expect(call[2]).toBe(4n)
    expect(scene.push_pull).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(3n)
  })

  it('selects the live replacement member after an in-component through-cut', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, INSTANCE),
      componentThroughResults: [88n],
    })
    const { tool, onCommit } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: -2, kind: 'endpoint' }), RAY)

    expect(onCommit).toHaveBeenCalledWith(88n)
  })

  it('a def-owned sketch region resolves through pick_sketch_region_in_instance (never the world-only pick_sketch_region) and extrudes via extrude_region_in_instance', () => {
    // `pick_sketch_region` only ever walks WORLD-tree sketches
    // (`Document::sketch_ids()` deliberately excludes definition-owned
    // ones) — it can never see a region drawn inside a component's own
    // definition, so an instance context must call the scoped, pose-mapping
    // `_in_instance` sibling instead (proven kernel-side in
    // `pick_sketch_region_is_blind_to_a_def_owned_sketch_but_the_in_
    // instance_sibling_finds_it`); this only checks the APP calls the right
    // one and wires its result through.
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPickInInstance: regionPick })
    const { tool, onCommit } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'endpoint' }), RAY)

    expect(scene.pick_sketch_region_in_instance).toHaveBeenCalled()
    const pickCall = (scene.pick_sketch_region_in_instance as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(pickCall[0]).toBe(INSTANCE)
    expect(scene.pick_sketch_region).not.toHaveBeenCalled()

    expect(scene.extrude_region_in_instance).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_region_in_instance as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(INSTANCE)
    expect(call[1]).toBe(99n)
    expect(call[2]).toBe(7n)
    expect(scene.extrude_region).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(55n)
  })

  it('a miss from pick_sketch_region_in_instance (no def-owned region under the cursor) extrudes nothing', () => {
    const scene = makeWasmScene({ facePick: undefined, regionPickInInstance: undefined })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(scene.extrude_region_in_instance).not.toHaveBeenCalled()
    expect(scene.extrude_region).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
  })
})

// The in-instance ghost preview used to fall straight to the bare arrow +
// tip-cross fallback — `face_boundary`/`region_boundary` answer in
// DEFINITION-local coordinates, and drawing that boundary directly (without
// mapping it through the instance's pose) would draw the swept-prism ghost
// in the wrong place, so the old code deliberately skipped it whenever
// `target.instance` was set. That was the only visible difference between an
// in-instance and a plain-object push/pull drag. The fix pose-maps the
// boundary instead of skipping the prism (component-edit-parity.md Finding 3).
describe('PushPullTool — in-instance ghost preview matches the plain-object swept prism (Finding 3)', () => {
  const INSTANCE = 12n
  const COMPONENT = 77n
  /** Translated (not identity) pose — proves the ghost is actually POSE-
   *  MAPPED, not coincidentally right under a no-op transform. */
  const TRANSLATED_POSE = new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])
  /** A definition-local unit square, the kind `face_boundary`/
   *  `region_boundary` return for a component member/def-owned sketch. */
  const LOCAL_SQUARE = new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0])

  /** The prism's world-space vertex X extents, or `null` if the preview
   *  holds no Mesh (i.e. it fell back to the arrow). */
  function prismXExtent(preview: THREE.Group): { min: number; max: number } | null {
    const mesh = preview.children.find((c) => (c as THREE.Mesh).isMesh === true) as THREE.Mesh | undefined
    if (mesh === undefined) return null
    const pos = mesh.geometry.getAttribute('position')
    let min = Infinity
    let max = -Infinity
    for (let i = 0; i < pos.count; i++) {
      min = Math.min(min, pos.getX(i))
      max = Math.max(max, pos.getX(i))
    }
    return { min, max }
  }

  it('an in-instance FACE push/pull draws a pose-mapped swept prism, not the bare arrow', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, INSTANCE) })
    ;(scene.face_boundary as ReturnType<typeof vi.fn>).mockReturnValue(LOCAL_SQUARE)
    ;(scene.instance_pose as ReturnType<typeof vi.fn>).mockReturnValue(TRANSLATED_POSE)
    const { tool, preview } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    tool.onPointerDown(makeSnap({ x: 5, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 5, y: 0, z: 1, kind: 'endpoint' }), RAY)

    // A plain LineSegments arrow+cross is what the old bug drew; the fix
    // draws the same swept-prism Mesh a plain-object drag gets.
    expect(preview.children.some((c) => c instanceof THREE.LineSegments)).toBe(false)
    const extent = prismXExtent(preview)
    expect(extent).not.toBeNull()
    // Local x ∈ [0,1] maps through the +5 translation to world x ∈ [5,6] —
    // proof the boundary was actually mapped, not drawn at its raw local
    // position (which would extend [0,1], not [5,6]).
    expect(extent?.min).toBeCloseTo(5)
    expect(extent?.max).toBeCloseTo(6)
  })

  it('an in-instance REGION (def-owned sketch) push/pull draws a pose-mapped swept prism, not the bare arrow', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({
      facePick: undefined,
      regionPickInInstance: regionPick,
      sketchPlane: [0, 0, 0, 0, 0, 1],
    })
    ;(scene.region_boundary as ReturnType<typeof vi.fn>).mockReturnValue(LOCAL_SQUARE)
    ;(scene.instance_pose as ReturnType<typeof vi.fn>).mockReturnValue(TRANSLATED_POSE)
    const { tool, preview } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(makeSnap({ x: 5, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 5, y: 0, z: 1, kind: 'endpoint' }), RAY)

    expect(preview.children.some((c) => c instanceof THREE.LineSegments)).toBe(false)
    const extent = prismXExtent(preview)
    expect(extent).not.toBeNull()
    expect(extent?.min).toBeCloseTo(5)
    expect(extent?.max).toBeCloseTo(6)
  })

  it('a stale instance pose mid-drag (instance vanished after the click) falls back to the arrow instead of a misplaced ghost', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, INSTANCE) })
    ;(scene.face_boundary as ReturnType<typeof vi.fn>).mockReturnValue(LOCAL_SQUARE)
    // A valid pose for onPointerDown's own `worldFaceNormal` lookup (else the
    // click misses outright — "normal === null ... treated exactly like a
    // miss"), then gone by the time the ghost preview asks for it again —
    // the genuine "instance vanished mid-drag" case.
    // (However many lookups the click itself makes — normal, plane context —
    // the pose is live for ALL of them and gone only after the click.)
    let poseLive = true
    ;(scene.instance_pose as ReturnType<typeof vi.fn>).mockImplementation(() => (poseLive ? TRANSLATED_POSE : undefined))
    const { tool, preview } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    tool.onPointerDown(makeSnap({ x: 5, y: 0, z: 0, kind: 'endpoint' }), RAY)
    poseLive = false
    expect(tool.capturingInput()).toBe(true)
    tool.onPointerMove(makeSnap({ x: 5, y: 0, z: 1, kind: 'endpoint' }), RAY)

    // No prism (no pose left to map through) — the arrow fallback, same as a
    // stale handle does for a plain object.
    expect(prismXExtent(preview)).toBeNull()
    expect(preview.children.some((c) => c instanceof THREE.LineSegments)).toBe(true)
  })

  it('a plain (non-instance) push/pull is unaffected — still the swept prism at its raw (already-world) boundary', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    ;(scene.face_boundary as ReturnType<typeof vi.fn>).mockReturnValue(LOCAL_SQUARE)
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'endpoint' }), RAY)

    expect(preview.children.some((c) => c instanceof THREE.LineSegments)).toBe(false)
    const extent = prismXExtent(preview)
    expect(extent?.min).toBeCloseTo(0)
    expect(extent?.max).toBeCloseTo(1)
  })
})

describe('PushPullTool — setEditContext aborts an armed gesture on a genuine change (component-edit-parity.md phase A2)', () => {
  it('a genuine context change cancels an armed drag instead of silently retargeting its eventual commit', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 42n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'top' })

    expect(tool.capturingInput()).toBe(false)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    expect(scene.push_pull).not.toHaveBeenCalled() // the cancelled gesture never commits
  })

  it('re-pushing the SAME context is a no-op — an armed drag survives it untouched', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 42n) })
    const { tool } = makeTool(scene)
    const ctx = { kind: 'instance' as const, id: 42n, component: 5n }
    tool.setEditContext(ctx)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })

    expect(tool.capturingInput()).toBe(true)
  })
})

// Double-click repeat (design tool-parity §2): repeats `lastCommittedDistance`
// on whatever face/region a fresh double-click lands on. The gesture's FIRST
// click always routes through the normal onPointerDown first (the Viewport
// only skips the phantom SECOND pointerdown), so these tests always issue a
// real onPointerDown immediately before calling onDoubleClick, mirroring the
// real event sequence.
describe('PushPullTool — double-click repeat', () => {
  it('with nothing committed yet, a double-click does not repeat and falls through', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.capturingInput()).toBe(true) // click 1 started the drag

    const handled = tool.onDoubleClick(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(handled).toBe(false)
    expect(tool.capturingInput()).toBe(false) // the phantom drag was reset
    expect(scene.push_pull).not.toHaveBeenCalled()
  })

  it('repeats the last committed distance on a double-clicked face', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    expect(scene.push_pull).toHaveBeenCalledTimes(1)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    const handled = tool.onDoubleClick(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(handled).toBe(true)
    expect(scene.push_pull).toHaveBeenCalledTimes(2)
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[1]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(4n)
    expect(call[2]).toBeCloseTo(2)
    expect(onCommit).toHaveBeenCalledTimes(2)
  })

  it('repeats a signed (recessed) distance with the same sign', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    for (const ch of '-0.5') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(scene.push_pull).toHaveBeenCalledTimes(1)
    expect((scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0][2]).toBeCloseTo(-0.5)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onDoubleClick(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(scene.push_pull).toHaveBeenCalledTimes(2)
    expect((scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[1][2]).toBeCloseTo(-0.5)
  })

  it('repeats onto a sketch region target too (extrude_region)', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'endpoint' }), RAY)
    expect(scene.extrude_region).toHaveBeenCalledTimes(1)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    const handled = tool.onDoubleClick(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(handled).toBe(true)
    expect(scene.extrude_region).toHaveBeenCalledTimes(2)
    expect((scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[1][2]).toBeCloseTo(3)
  })

  it('an ineligible face click never starts a drag, so the double-click falls through', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      parents: new Map([[3n, 9n]]), // grouped → ineligible
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // toasts, stays idle
    const handled = tool.onDoubleClick(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(handled).toBe(false)
    expect(scene.push_pull).not.toHaveBeenCalled()
  })
})

// Ctrl/Cmd extrude-as-new-object modifier (design tool-parity §2): a durable
// per-gesture toggle (Move-copy idiom — driven externally via
// toggleExtrudeAsNew(), mirroring how the Viewport's dedicated Ctrl/Cmd
// listener calls it; see PushPullTool.onKey's note on why a bare Control/Meta
// keydown can't ride the tool's own onKey).
describe('PushPullTool — Ctrl/Cmd extrude-as-new-object modifier', () => {
  it('routes a face commit through extrude_face_as_new_object while the mode is on', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onCommit } = makeTool(scene)

    tool.toggleExtrudeAsNew()
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    expect(scene.extrude_face_as_new_object).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_face_as_new_object as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(4n)
    expect(call[2]).toBeCloseTo(2)
    expect(scene.push_pull).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(42n)
  })

  it('tapping the toggle twice returns to the normal push_pull commit', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.toggleExtrudeAsNew()
    tool.toggleExtrudeAsNew()
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    expect(scene.push_pull).toHaveBeenCalledTimes(1)
    expect(scene.extrude_face_as_new_object).not.toHaveBeenCalled()
  })

  it('does not apply to a sketch-region target — extrude_region commits normally', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool } = makeTool(scene)

    tool.toggleExtrudeAsNew()
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'endpoint' }), RAY)

    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    expect(scene.extrude_face_as_new_object).not.toHaveBeenCalled()
  })

  it('falls back to push_pull_in_component inside a component context (no definition-member surface)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 42n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })
    tool.setFaceEligibility(() => true)
    tool.toggleExtrudeAsNew()

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    expect(scene.extrude_face_as_new_object).not.toHaveBeenCalled()
    expect(scene.push_pull_in_component).toHaveBeenCalledTimes(1)
  })

  it('notifies the mode-change callback on each toggle', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onExtrudeAsNewModeChange } = makeTool(scene)

    tool.toggleExtrudeAsNew()
    expect(onExtrudeAsNewModeChange).toHaveBeenCalledWith(true)
    tool.toggleExtrudeAsNew()
    expect(onExtrudeAsNewModeChange).toHaveBeenCalledWith(false)
  })

  it('the status hint reflects the toggle', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    expect(tool.statusHint()).not.toContain('Ctrl is on')
    tool.toggleExtrudeAsNew()
    expect(tool.statusHint()).toContain('Ctrl is on')
  })
})

// Design v1.1 Lane E "Push/Pull face-first": the idle hover constrains the
// next snap query to the face/region plane under the cursor, restricted to
// its `on-face` point (`facesOnly`) — inference's rank order can never let
// `on-face` outrank a precise point (endpoint/midpoint/…), so without this a
// hover near a box corner would show — and a drag from there would push/pull
// from — the corner's chip instead of the face itself.
describe('PushPullTool — snapConstraint (design v1.1 Lane E "Push/Pull face-first")', () => {
  it('idle hover over an eligible face returns its plane with facesOnly, memoized per ray (FacePickCache)', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      facePlane: [1, 2, 3, 0, 0, 1],
    })
    const { tool } = makeTool(scene)

    const constraint = tool.snapConstraint?.(RAY)
    expect(constraint).toEqual({
      constraintPlane: { point: [1, 2, 3], normal: [0, 0, 1] },
      facesOnly: true,
    })
    // A second call for the SAME ray object must reuse the cached pick
    // rather than re-raycasting (FacePickCache — same pattern as the draw
    // tools).
    tool.snapConstraint?.(RAY)
    expect(scene.pick_face).toHaveBeenCalledTimes(1)
  })

  it('idle hover with no eligible face but a live sketch region falls back to the region plane, still facesOnly', () => {
    const scene = makeWasmScene({
      facePick: undefined,
      regionPick: makeRegionPick(99n, 7n),
      sketchPlane: [0, 0, 5, 0, 0, 1],
    })
    const { tool } = makeTool(scene)

    const constraint = tool.snapConstraint?.(RAY)
    expect(constraint).toEqual({
      constraintPlane: { point: [0, 0, 5], normal: [0, 0, 1] },
      facesOnly: true,
    })
  })

  it('idle hover over nothing (no face, no region) returns null — unconstrained', () => {
    const scene = makeWasmScene({ facePick: undefined, regionPick: undefined })
    const { tool } = makeTool(scene)

    expect(tool.snapConstraint?.(RAY)).toBeNull()
  })

  it('an ineligible face never offers a constraint (matches the click-time refusal, not the ground fallthrough)', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      parents: new Map([[3n, 9n]]), // grouped → ineligible
      regionPick: undefined,
    })
    const { tool } = makeTool(scene)

    expect(tool.snapConstraint?.(RAY)).toBeNull()
  })

  // Adversarial-review finding: an ineligible face used to collapse to
  // `eligible === null` indistinguishably from "no face hit at all", so
  // Path B ran anyway and a sketch region BEHIND the group became the
  // constraint plane — the hover cue would land on the GROUND plane
  // through the solid instead of failing closed like onPointerDown does.
  // `rawPickFor` now lets snapConstraint tell the two cases apart.
  it('an ineligible face WITH a live sketch region behind it still offers no constraint — never the region\'s plane', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      parents: new Map([[3n, 9n]]), // grouped → ineligible
      regionPick: makeRegionPick(50n, 51n, 2), // a real region along the SAME ray, behind the face
      sketchPlane: [0, 0, 7, 0, 0, 1], // a distinctive plane — must never surface
    })
    const { tool } = makeTool(scene)

    expect(tool.snapConstraint?.(RAY)).toBeNull()
    // The region IS picked (its depth decides whether it is in front of
    // the face — GitHub issue 13), but behind an ineligible face it never
    // becomes the constraint.
  })

  it('once a drag has anchored, the constraint is null — HARD_SNAP_KINDS keep working unconstrained ("pull to that edge")', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.capturingInput()).toBe(true)
    expect(tool.snapConstraint?.(RAY)).toBeNull()
  })

  it('anchor lies on the face plane: a null snap no longer falls back to the ground plane, only to the ray origin', () => {
    // Old behavior: a null snap on the first click fell back to
    // `intersectGroundPlane(ray)` — for this ray that lands at world
    // z = 0, two units below the face's own plane (z = 5, the ray's
    // origin). With `snapConstraint` now always offering a facesOnly
    // constraint plane over an eligible face, a null snap can no longer
    // happen in practice (the constraint plane's ray intersection always
    // produces one) — the ray-origin fallback here is defensive only, and
    // this pins it to the CORRECT (non-ground) value rather than silently
    // reintroducing the old ground-plane bug if it were ever hit.
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) }) // normal +Z
    const { tool } = makeTool(scene)

    tool.onPointerDown(null, RAY) // RAY origin is [0, 0, 5]
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    expect(scene.push_pull).toHaveBeenCalledTimes(1)
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0]
    // anchor.z = 5 (ray origin) → signed distance to (0,0,2) along +Z is -3.
    // The old ground-plane anchor (z = 0) would have committed +2 instead.
    expect(call[2]).toBeCloseTo(-3)
  })

  // Adversarial-review finding: the Viewport calls `snapConstraint(ray)`
  // then `onPointerDown(snap, ray)` with the IDENTICAL `Ray` object for a
  // real click — onPointerDown used to re-raycast from scratch instead of
  // reusing `snapConstraint`'s already-memoized pick, costing a click two
  // `pick_face` calls instead of one.
  it('a real click costs ONE pick_face call — onPointerDown reuses snapConstraint\'s cached pick for the same ray', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.snapConstraint?.(RAY) // the Viewport's hover-cue probe
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY) // the click itself

    expect(scene.pick_face).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(true) // the click still landed correctly
  })

  it('a real click costs ONE pick_sketch_region call — onPointerDown reuses snapConstraint\'s cached region pick for the same ray', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool } = makeTool(scene)

    tool.snapConstraint?.(RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)

    expect(scene.pick_sketch_region).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(true)
  })
})

// Post-commit distance retype (retypeWindow.ts): type a distance after the
// commit click and the push/pull just made is redone at that distance.
describe('PushPullTool — retype the distance after the commit', () => {
  function makeRetypeScene(opts: Parameters<typeof makeWasmScene>[0] = {}) {
    const base = makeWasmScene(opts) as unknown as Record<string, unknown>
    let gen = 1n
    const basePush = base.push_pull as (...args: unknown[]) => unknown
    const scene = {
      ...base,
      history_generation: vi.fn(() => gen),
      push_pull: vi.fn((...args: unknown[]) => { const r = basePush(...args); gen += 1n; return r }),
      scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
      scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    }
    return scene as unknown as WasmScene
  }
  const key = (tool: PushPullTool, k: string) => tool.onKey({ key: k } as KeyboardEvent)
  const typeIn = (tool: PushPullTool, text: string) => { for (const ch of text) key(tool, ch); key(tool, 'Enter') }
  const pushDistances = (scene: WasmScene) => (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[2] as number)

  it('click-drag-click, then a typed distance: one undo and the same face pushed at the new distance', () => {
    const scene = makeRetypeScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    expect(pushDistances(scene)).toEqual([2])
    expect(tool.statusHint()).toContain('redo the push/pull')
    expect(tool.capturesKey('3')).toBe(true)
    expect(tool.capturesKey('p')).toBe(false) // shortcuts still work with an empty buffer
    typeIn(tool, '3')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(pushDistances(scene)).toEqual([2, 3])
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[1]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(4n)
    expect(onCommit).toHaveBeenCalledTimes(2)
    // A negative value flips the direction; the window stays open.
    typeIn(tool, '-1')
    expect(scene.scene_undo).toHaveBeenCalledTimes(2)
    expect(pushDistances(scene)).toEqual([2, 3, -1])
  })

  it('a typed (Enter) commit arms the window too, and Escape closes it', () => {
    const scene = makeRetypeScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeIn(tool, '1')
    expect(pushDistances(scene)).toEqual([1])
    typeIn(tool, '2')
    expect(pushDistances(scene)).toEqual([1, 2])
    key(tool, '5')
    key(tool, 'Escape')
    expect(tool.capturesKey('5')).toBe(false)
    key(tool, '5'); key(tool, 'Enter')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
  })

  it('a zero typed distance is refused with the usual toast and nothing is undone', () => {
    const scene = makeRetypeScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    typeIn(tool, '0')
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledWith('Move more before committing push/pull')
  })
})

describe('PushPullTool — retype after extruding a sketch REGION re-picks the region', () => {
  it('the undo re-mints the region handle; the re-commit extrudes the freshly picked one at the typed distance', () => {
    const base = makeWasmScene({ regionPick: makeRegionPick(9n, 7n) }) as unknown as Record<string, unknown>
    let gen = 1n
    const baseExtrude = base.extrude_region as (...args: unknown[]) => unknown
    const scene = {
      ...base,
      history_generation: vi.fn(() => gen),
      extrude_region: vi.fn((...args: unknown[]) => { const r = baseExtrude(...args); gen += 1n; return r }),
      scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
      scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    } as unknown as WasmScene
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'region' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
    const extrude = scene.extrude_region as ReturnType<typeof vi.fn>
    expect(extrude).toHaveBeenCalledTimes(1)
    expect(extrude.mock.calls[0].slice(0, 3)).toEqual([9n, 7n, 2])

    // After the undo the same spot holds region 7n's successor, 12n.
    ;(scene.pick_sketch_region as ReturnType<typeof vi.fn>).mockImplementation(() => makeRegionPick(9n, 12n))
    for (const ch of '3') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(extrude).toHaveBeenCalledTimes(2)
    expect(extrude.mock.calls[1].slice(0, 3)).toEqual([9n, 12n, 3])
    // The pick was dropped onto the plane at the click point, along −normal.
    const pickCall = (scene.pick_sketch_region as ReturnType<typeof vi.fn>).mock.calls.at(-1)!
    expect(pickCall[5]).toBeCloseTo(-1, 9)
  })
})

describe('PushPullTool — hard snaps that are not a depth (inference-permissiveness fixes)', () => {
  /** Arm a drag on a +Z face at the origin and return the tool + the live
   *  distance readout spy. */
  function armDrag() {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const made = makeTool(scene)
    made.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    return { scene, ...made }
  }
  /** The distance the ghost preview is currently at, read back through the
   *  second click's commit argument. */
  function commitAt(tool: PushPullTool, scene: WasmScene, snap: Snap | null, ray: Ray): number {
    tool.onPointerDown(snap, ray)
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls.at(-1)!
    return call[2] as number
  }
  /** A ray whose closest approach to the +Z axis through the origin is at
   *  height `z` — the free-drag depth the cursor alone would give. */
  function freeDragRay(z: number): Ray {
    return { origin: [5, 0, z], direction: [-1, 0, 0] }
  }

  it('a snap ON the face\'s own plane (its edge, an edge midpoint) is ignored — the drag keeps following the cursor instead of dropping to 0', () => {
    const { tool, scene } = armDrag()
    // Cursor is 0.4 up the axis; the kernel meanwhile snapped the face's own
    // boundary edge (z = 0) — the ground rectangle's edge under a pull.
    tool.onPointerMove(makeSnap({ x: 0.5, y: 0, z: 0, kind: 'on-edge' }), freeDragRay(0.4))
    expect(commitAt(tool, scene, makeSnap({ x: 0.5, y: 0.5, z: 0, kind: 'midpoint' }), freeDragRay(0.4))).toBeCloseTo(0.4)
  })

  it('a drawing-axis snap is never a depth reference — crossing the blue axis mid-pull does not flip the preview below ground', () => {
    const { tool, scene } = armDrag()
    // The cursor ray passes over the +Z axis; the origin-relative on-axis
    // candidate would sit BELOW the anchor (a fact about the eye, not the
    // drag), and used to be honoured as a negative depth.
    expect(commitAt(tool, scene, makeSnap({ x: 0, y: 0, z: -0.7, kind: 'on-axis' }), freeDragRay(0.6))).toBeCloseTo(0.6)
  })

  it('a real off-plane reference (an edge midpoint at height 0.5) still sets the depth exactly', () => {
    const { tool, scene } = armDrag()
    expect(commitAt(tool, scene, makeSnap({ x: 2, y: 2, z: 0.5, kind: 'midpoint' }), freeDragRay(0.9))).toBeCloseTo(0.5)
  })

  it('a guide line stays a depth reference (the user placed it on purpose)', () => {
    const { tool, scene } = armDrag()
    expect(commitAt(tool, scene, makeSnap({ x: 2, y: 2, z: 0.75, kind: 'on-guide' }), freeDragRay(0.9))).toBeCloseTo(0.75)
  })

  it('the line where two objects\' faces meet is a depth reference, like a real edge', () => {
    const { tool, scene } = armDrag()
    const meetingLine: Snap['meetingLine'] = [
      [2, 1, 0.5],
      [2, 3, 0.5],
    ]
    expect(
      commitAt(tool, scene, makeSnap({ x: 2, y: 2, z: 0.5, kind: 'on-intersection-line', meetingLine }), freeDragRay(0.9)),
    ).toBe(0.5)
  })

  it('a face under the cursor is not: the drag follows the cursor past it', () => {
    const { tool, scene } = armDrag()
    expect(commitAt(tool, scene, makeSnap({ x: 2, y: 2, z: 0.5, kind: 'on-face' }), freeDragRay(0.9))).toBeCloseTo(0.9)
  })
})

describe('PushPullTool — a drawn region in front of an object face wins the pick (GitHub issue 13)', () => {
  it('idle hover: a sketch region NEARER than the face on the same ray offers the region\'s plane, not the face\'s', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, undefined, 2.0), // the cube's top, behind
      facePlane: [0, 0, 1, 0, 0, 1],
      regionPick: makeRegionPick(99n, 7n, 1.5), // the vertical rectangle, in front
      sketchPlane: [0, -1, 0, 0, -1, 0],
    })
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint?.(RAY)).toEqual({
      constraintPlane: { point: [0, -1, 0], normal: [0, -1, 0] },
      facesOnly: true,
    })
  })

  it('click: the nearer region extrudes (extrude_region), the face behind it is left alone', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, undefined, 2.0),
      regionPick: makeRegionPick(99n, 7n, 1.5),
      sketchPlane: [0, -1, 0, 0, -1, 0],
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: -1, z: 1.1, kind: 'on-face' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: -1.4, z: 1.1, kind: 'endpoint' }), RAY)
    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    expect((scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(99n)
    expect(scene.push_pull).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a region BEHIND the face (or coplanar with it) still yields to the face — Path A precedence is unchanged there', () => {
    for (const regionDepth of [2.5, 2.0]) {
      const scene = makeWasmScene({
        facePick: makeFacePick(3n, 4n, undefined, 2.0),
        facePlane: [1, 2, 3, 0, 0, 1],
        regionPick: makeRegionPick(99n, 7n, regionDepth),
      })
      const { tool } = makeTool(scene)
      expect(tool.snapConstraint?.(RAY)?.constraintPlane).toEqual({ point: [1, 2, 3], normal: [0, 0, 1] })
      tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'on-face' }), RAY)
      tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 4, kind: 'endpoint' }), RAY)
      expect(scene.push_pull).toHaveBeenCalledTimes(1)
      expect(scene.extrude_region).not.toHaveBeenCalled()
    }
  })

  it('a DETACHED shape lying on an ELIGIBLE face wins the tie and extrudes into a new object (separate-shapes D8)', () => {
    // Same depth: the disk was drawn on the face to stay apart from it.
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, undefined, 2.0),
      facePlane: [1, 2, 3, 0, 0, 1],
      regionPick: makeRegionPick(99n, 7n, 2.0, true),
    })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'on-face' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 4, kind: 'endpoint' }), RAY)
    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    expect(scene.push_pull).not.toHaveBeenCalled()
  })

  it('an ORDINARY region lying on an eligible face still yields to the face (the tie-break is detached-only)', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, undefined, 2.0),
      facePlane: [1, 2, 3, 0, 0, 1],
      regionPick: makeRegionPick(99n, 7n, 2.0, false),
    })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'on-face' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 4, kind: 'endpoint' }), RAY)
    expect(scene.push_pull).toHaveBeenCalledTimes(1)
    expect(scene.extrude_region).not.toHaveBeenCalled()
  })

  it('a region in front of an INELIGIBLE (grouped) face wins too — the fail-closed rule only guards regions behind the face', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, undefined, 2.0),
      parents: new Map([[3n, 9n]]),
      regionPick: makeRegionPick(99n, 7n, 1.0),
    })
    const { tool, onToast } = makeTool(scene)
    expect(tool.snapConstraint?.(RAY)).not.toBeNull()
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'endpoint' }), RAY)
    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('the region depth is compared in metres even when the pick ray is not unit length', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, undefined, 2.0),
      facePlane: [1, 2, 3, 0, 0, 1],
      // Region at ray parameter 1.5 along a length-2 direction = 3 m: BEHIND
      // the face at 2 m, so the face must still win.
      regionPick: makeRegionPick(99n, 7n, 1.5),
    })
    const { tool } = makeTool(scene)
    const longRay: Ray = { origin: [0, 0, 5], direction: [0, 0, -2] }
    expect(tool.snapConstraint?.(longRay)?.constraintPlane).toEqual({ point: [1, 2, 3], normal: [0, 0, 1] })
  })
})


describe('PushPullTool — a clicked depth is exactly the displayed depth', () => {
  // These pin the Exact digits (9.399 mm); the default mm rounding is coarser.
  beforeAll(() => setPrecision('exact', 'mm'))
  afterAll(() => setPrecision(DEFAULT_PRECISION.mm, 'mm'))
  const prev = getLengthUnit()
  const pushPullDistance = (scene: WasmScene): number =>
    (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0][2] as number
  // A horizontal ray crossing the push axis at z = 12.3456 mm (shows "12.346 mm").
  const SIDE: Ray = { origin: [1, 0, 0.0123456], direction: [-1, 0, 0] }

  function pull(kind: string, depthSnap: number, ray: Ray = SIDE) {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onMeasurement } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face' }), RAY)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: depthSnap, kind }), ray)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: depthSnap, kind }), ray)
    return { d: pushPullDistance(scene), onMeasurement }
  }

  it('a free click commits the shown distance', () => {
    setLengthUnit('mm')
    try {
      const r = pull('on-face', 0.0123456)
      expect(r.onMeasurement).toHaveBeenCalledWith('12.346 mm')
      expect(r.d).toBe(12.346 * 0.001)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('a free inward click commits the shown negative distance (typed sign rules)', () => {
    setLengthUnit('mm')
    try {
      const r = pull('on-face', -0.0123456, { origin: [1, 0, -0.0123456], direction: [-1, 0, 0] })
      expect(r.d).toBe(-12.346 * 0.001)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('a hard-snap depth (endpoint) stays exact', () => {
    setLengthUnit('mm')
    try {
      expect(pull('endpoint', 0.0123456).d).toBeCloseTo(0.0123456, 15)
    } finally {
      setLengthUnit(prev)
    }
  })
})


// The plane is the drafting context (unified drafting §4.6): the region under
// the cursor resolves through the plane's listing. A region over the struck
// object's face pushes that face; a free region extrudes as a profile; a
// region over the object that no face can hold (a pinch of an owned sketch)
// refuses with the RegionNotPushable copy.
describe('PushPullTool — routing through the plane listing', () => {
  /** The context of the default face plane (origin, +Z). */
  const CTX = ctxOfPlane([0, 0, 0], [0, 0, 1]) as string
  const sq = (x0: number, y0: number, x1: number, y1: number): [number, number, number][] => [[x0, y0, 0], [x1, y0, 0], [x1, y1, 0], [x0, y1, 0]]
  const faceRay = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })

  it('a region drawn on the face pushes its own sub-face, whichever face the ray struck', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      listings: { [CTX]: { regions: [{ key: 1, outer: sq(0, 0, 2, 2), over: { object: 3, face: 11 } }] } },
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'endpoint' }), faceRay(1, 1))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2, kind: 'endpoint' }), faceRay(1, 1))

    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(11n)
    expect(call[2]).toBeCloseTo(2)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a bare face (its region lists the struck face) pushes that face as before', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      listings: { [CTX]: { regions: [{ key: 1, outer: sq(0, 0, 2, 2), over: { object: 3, face: 4 } }] } },
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'endpoint' }), faceRay(1, 1))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2, kind: 'endpoint' }), faceRay(1, 1))

    expect((scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 2)).toEqual([3n, 4n])
  })

  it.each([
    ['sliver', 'RegionSliver', 'too thin'],
    ['other', 'RegionNotPushable', 'part of it is drawn'],
    [undefined, 'RegionNotPushable', 'part of it is drawn'],
  ])('a face-less region over the object (why=%s) toasts %s', (why, code, text) => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      listings: {
        [CTX]: {
          regions: [
            { key: 1, outer: sq(0, 0, 2, 2), over: { object: 3, face: 4 }, holes: [sq(0.5, 0.5, 1.5, 1.5)] },
            { key: 2, outer: sq(0.5, 0.5, 1.5, 1.5), over: { object: 3, face: null, ...(why ? { why } : {}) } },
          ],
        },
      },
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'endpoint' }), faceRay(1, 1))
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe(code)
    expect(String(onToast.mock.calls[0][0])).toContain(text)
    expect(scene.push_pull).not.toHaveBeenCalled()
  })

  it('a pinched region (over the object, no face) toasts RegionNotPushable and starts nothing', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      listings: {
        [CTX]: {
          regions: [
            { key: 1, outer: sq(0, 0, 2, 2), over: { object: 3, face: 4 }, holes: [sq(0.5, 0.5, 1.5, 1.5)] },
            { key: 2, outer: sq(0.5, 0.5, 1.5, 1.5), over: { object: 3, face: null } },
          ],
        },
      },
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'endpoint' }), faceRay(1, 1))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('RegionNotPushable')
    expect(String(onToast.mock.calls[0][0])).toContain("can't be pushed or pulled")
    expect(tool.capturingInput()).toBe(false)
    expect(scene.push_pull).not.toHaveBeenCalled()

    // The face around the pinch still pushes as the face.
    tool.onPointerDown(makeSnap({ x: 0.25, y: 0.25, z: 0, kind: 'endpoint' }), faceRay(0.25, 0.25))
    expect(tool.capturingInput()).toBe(true)
  })

  it('a free region (listing names its sketch and region) extrudes as a profile', () => {
    const scene = makeWasmScene({
      regionPick: makeRegionPick(9n, 7n),
      listings: { [CTX]: { regions: [{ key: 1, outer: sq(0, 0, 2, 2), extrude: { sketch: 9, region: 7 } }] } },
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'endpoint' }), faceRay(1, 1))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2, kind: 'endpoint' }), faceRay(1, 1))

    const call = (scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(9n)
    expect(call[1]).toBe(7n)
    expect(call[2]).toBeCloseTo(2)
    expect(scene.push_pull).not.toHaveBeenCalled()
  })
})

describe('PushPullTool — a flat surface is pulled into a solid (Model Health, design F7)', () => {
  const INSTANCE = 42n
  const COMPONENT = 5n

  /** A scene whose health report gives `kind` for every object asked about. */
  function healthScene(kind: string | null, opts: Parameters<typeof makeWasmScene>[0] = {}, faces = 1): WasmScene {
    const base = makeWasmScene(opts) as unknown as Record<string, unknown>
    base.thicken_sheet = vi.fn(() => new BigUint64Array())
    base.thicken_sheet_in_instance = vi.fn(() => new BigUint64Array())
    base.health_report_json = vi.fn(() =>
      JSON.stringify({
        rows: kind === null ? [] : [{ object: '3', verdict: { kind, holes: 0 }, rims: [], faces }],
        families: [],
        counts: {},
      }),
    )
    return base as unknown as WasmScene
  }

  const drag = (tool: PushPullTool) => {
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)
  }

  it('a flat_sheet face commits thicken_sheet(object, 0, distance), not push_pull', () => {
    const scene = healthScene('flat_sheet', { facePick: makeFacePick(3n, 4n) })
    const { tool, onCommit, onToast } = makeTool(scene)
    drag(tool)
    expect(scene.thicken_sheet).toHaveBeenCalledTimes(1)
    const call = (scene.thicken_sheet as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(0)
    expect(call[2]).toBeCloseTo(2)
    expect(scene.push_pull).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(3n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a flat_sheet picked through an instance commits thicken_sheet_in_instance with the world distance', () => {
    const scene = healthScene('flat_sheet', { facePick: makeFacePick(3n, 4n, INSTANCE) })
    const { tool, onCommit } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)
    drag(tool)
    expect(scene.thicken_sheet_in_instance).toHaveBeenCalledTimes(1)
    const call = (scene.thicken_sheet_in_instance as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(INSTANCE)
    expect(call[1]).toBe(3n)
    expect(call[2]).toBeCloseTo(2)
    expect(scene.thicken_sheet).not.toHaveBeenCalled()
    expect(scene.push_pull_in_component).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(3n)
  })

  it('a solid (no report row) and a bent sheet keep the ordinary push_pull', () => {
    for (const kind of [null, 'bent_sheet', 'cappable']) {
      const scene = healthScene(kind, { facePick: makeFacePick(3n, 4n) })
      const { tool } = makeTool(scene)
      drag(tool)
      expect(scene.push_pull).toHaveBeenCalledTimes(1)
      expect(scene.thicken_sheet).not.toHaveBeenCalled()
    }
  })

  it('a flat sheet of several faces (a region drawn on it) keeps the ordinary push_pull', () => {
    const scene = healthScene('flat_sheet', { facePick: makeFacePick(3n, 4n) }, 2)
    const { tool } = makeTool(scene)
    drag(tool)
    expect(scene.thicken_sheet).not.toHaveBeenCalled()
    expect(scene.push_pull).toHaveBeenCalledTimes(1)
  })

  it('the extrude-as-new modifier is unchanged on a flat sheet', () => {
    const scene = healthScene('flat_sheet', { facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)
    tool.toggleExtrudeAsNew()
    drag(tool)
    expect(scene.thicken_sheet).not.toHaveBeenCalled()
  })

  it('a refusal from thicken_sheet toasts the kernel copy', () => {
    const scene = healthScene('flat_sheet', { facePick: makeFacePick(3n, 4n) })
    ;(scene.thicken_sheet as ReturnType<typeof vi.fn>).mockImplementation(() => {
      throw new Error('ThicknessTooSmall: too thin')
    })
    const { tool, onToast, onCommit } = makeTool(scene)
    drag(tool)
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledWith(expect.stringMatching(/too small/), 'ThicknessTooSmall')
  })
})

describe('PushPullTool — preview tint per mode (#41)', () => {
  const SQUARE = new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0])

  function ghostColor(extrudeAsNew: boolean, z = 1): number {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    ;(scene.face_boundary as ReturnType<typeof vi.fn>).mockReturnValue(SQUARE)
    const { tool, preview } = makeTool(scene)
    if (extrudeAsNew) tool.toggleExtrudeAsNew()
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z, kind: 'endpoint' }), RAY)
    const mesh = preview.children.find((c) => (c as THREE.Mesh).isMesh === true) as THREE.Mesh
    return (mesh.material as THREE.MeshBasicMaterial).color.getHex()
  }

  it('toggling extrude-as-new changes the ghost color the renderer receives', () => {
    const plain = ghostColor(false)
    const asNew = ghostColor(true)
    expect(asNew).not.toBe(plain)
    expect(ghostColor(true, -1)).toBe(asNew) // pulling the other way is still "new"
  })

  it('a free-sketch region ghost is the new-object blue on both signs, never the red cut', () => {
    const regionGhost = (z: number): number => {
      const scene = makeWasmScene({ regionPick: makeRegionPick(50n, 51n) })
      ;(scene.region_boundary as ReturnType<typeof vi.fn>).mockReturnValue(SQUARE)
      const { tool, preview } = makeTool(scene)
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onPointerMove(makeSnap({ x: 0, y: 0, z, kind: 'endpoint' }), RAY)
      const mesh = preview.children.find((c) => (c as THREE.Mesh).isMesh === true) as THREE.Mesh
      return (mesh.material as THREE.MeshBasicMaterial).color.getHex()
    }
    const asNew = ghostColor(true)
    expect(regionGhost(1)).toBe(asNew)
    expect(regionGhost(-1)).toBe(asNew)
    // A face pushed in stays the default cut look (not the new-object blue).
    expect(ghostColor(false, -1)).not.toBe(asNew)
  })

  it('inside a component edit the extrude-as-new toggle leaves the ghost on the push colors (the commit falls back to an ordinary push)', () => {
    const INSTANCE = 42n
    const inEdit = (extrudeAsNew: boolean): number => {
      const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, INSTANCE) })
      ;(scene.face_boundary as ReturnType<typeof vi.fn>).mockReturnValue(SQUARE)
      const { tool, preview } = makeTool(scene)
      tool.setEditContext({ kind: 'instance', id: INSTANCE, component: 5n })
      tool.setFaceEligibility((_object, instance) => instance === INSTANCE)
      if (extrudeAsNew) tool.toggleExtrudeAsNew()
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'endpoint' }), RAY)
      const mesh = preview.children.find((c) => (c as THREE.Mesh).isMesh === true) as THREE.Mesh
      expect(mesh).toBeDefined()
      return (mesh.material as THREE.MeshBasicMaterial).color.getHex()
    }
    expect(inEdit(true)).toBe(inEdit(false))
    expect(inEdit(true)).not.toBe(ghostColor(true))
  })

  it('the arrow/cross fallback (no swept prism) follows the mode too', () => {
    const arrowColors = (extrudeAsNew: boolean): number[] => {
      const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
      // A stale handle mid-drag: no ghost prism, so the bare arrow is drawn.
      ;(scene.face_boundary as ReturnType<typeof vi.fn>).mockImplementation(() => {
        throw new Error('stale handle')
      })
      const { tool, preview } = makeTool(scene)
      if (extrudeAsNew) tool.toggleExtrudeAsNew()
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
      tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'endpoint' }), RAY)
      const lines = preview.children.filter((c) => (c as THREE.LineSegments).isLineSegments === true) as THREE.LineSegments[]
      expect(preview.children.some((c) => (c as THREE.Mesh).isMesh === true)).toBe(false)
      return lines.map((l) => (l.material as THREE.LineBasicMaterial).color.getHex())
    }
    const plain = arrowColors(false)
    const asNew = arrowColors(true)
    expect(plain).toHaveLength(2) // shaft + tip cross
    expect(asNew).toHaveLength(2)
    expect(asNew[0]).not.toBe(plain[0])
    expect(asNew[1]).toBe(asNew[0])
    expect(plain[1]).toBe(plain[0])
  })

  it('reads the tints from the theme tokens', () => {
    const tokens: Record<string, string> = { '--preview-extrude-new': '#123456', '--preview-pushpull': '#654321' }
    vi.stubGlobal('document', { documentElement: {} })
    vi.stubGlobal('window', { getComputedStyle: () => ({ getPropertyValue: (n: string) => tokens[n] ?? '' }) })
    try {
      expect(ghostColor(true)).toBe(0x123456)
      expect(ghostColor(false)).toBe(0x654321)
    } finally {
      vi.unstubAllGlobals()
    }
  })

  it('defines both tokens in the dark and the light theme blocks', () => {
    const css = readFileSync(resolve(__dirname, '../theme/tokens.css'), 'utf8')
    const light = css.slice(css.indexOf("[data-theme='light']"))
    const dark = css.slice(css.indexOf("[data-theme='dark']"), css.indexOf("[data-theme='light']"))
    for (const block of [dark, light]) {
      expect(block).toMatch(/--preview-pushpull:\s*#[0-9a-f]{6}/i)
      expect(block).toMatch(/--preview-extrude-new:\s*#[0-9a-f]{6}/i)
    }
  })
})
