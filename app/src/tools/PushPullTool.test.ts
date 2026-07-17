/**
 * PushPullTool unit tests — Path A (object face) and Path B (sketch region,
 * now resolved via `pick_sketch_region` across ALL live sketches rather than
 * the old single "active sketch handle" bookkeeping — "sketches are
 * first-class interactable"). Mirrors the fake-WasmScene pattern used by
 * CircleTool.test.ts/ArcTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { PushPullTool } from './PushPullTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** A fake `FacePickJs` returning the seeded handles. */
function makeFacePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    free: vi.fn(),
  }
}

/** A fake `SketchRegionPickJs` returning the seeded handles. */
function makeRegionPick(sketch: bigint, region: bigint) {
  return {
    sketch: () => sketch,
    region: () => region,
    free: vi.fn(),
  }
}

function makeWasmScene(opts: {
  facePick?: ReturnType<typeof makeFacePick>
  regionPick?: ReturnType<typeof makeRegionPick>
  /** node_parent(0, id) result per object (a grouped object's group id). */
  parents?: Map<bigint, bigint>
  /** Unit normal `face_normal` reports for the picked face (default +Z). */
  faceNormal?: [number, number, number]
} = {}): WasmScene {
  const faceNormal = opts.faceNormal ?? [0, 0, 1]
  return {
    pick_face: vi.fn(() => opts.facePick),
    pick_sketch_region: vi.fn(() => opts.regionPick),
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
    face_normal: vi.fn(() => new Float64Array(faceNormal)),
    region_boundary: vi.fn(() => new Float32Array([])),
    face_boundary: vi.fn(() => new Float32Array([])),
    extrude_region: vi.fn(() => 55n),
    push_pull: vi.fn(() => ({
      is_through: () => false,
      result_objects: () => new BigUint64Array([]),
      free: vi.fn(),
    })),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new PushPullTool(scene, preview, onCommit, onToast, onMeasurement)
  return { tool, preview, onCommit, onToast, onMeasurement }
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
      regionPick: makeRegionPick(50n, 51n),  // live region behind it
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2 }), RAY)

    expect(tool.capturingInput()).toBe(false)      // no drag ever started
    expect(scene.pick_sketch_region).not.toHaveBeenCalled()
    expect(scene.extrude_region).not.toHaveBeenCalled()
    expect(scene.push_pull).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(2)       // one explicable refusal per click
    expect(String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])).toContain('group')
  })

  it('an ineligible (instanced) face over a region consumes the click with the component hint', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, 12n),
      regionPick: makeRegionPick(50n, 51n),
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(scene.extrude_region).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
    expect(String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])).toContain('component')
  })

  it('inside a GROUP context the refusal is the scoped hint, never the group default', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onToast } = makeTool(scene)
    // Mirror the real Viewport wiring for a deepest 'group' context: the two
    // id channels stay null (they only carry object/instance contexts) and
    // the injected context-path predicate does the rejecting.
    tool.setActiveContext(null)
    tool.setComponentContext(null)
    tool.setContextScoped(true)
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
    tool.setContextScoped(true)
    tool.setFaceEligibility(() => false)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    const msg = String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])
    expect(msg).toContain('step out')
  })

  it('in-context: a foreign face consumes the click with the scoped-editing hint', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(999n, 4n) })
    const { tool, onToast } = makeTool(scene)
    tool.setActiveContext(7n)

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
    tool.setActiveContext(1n)

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
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: -1e-15, kind: 'on-axis' }), RAY)
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
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: -0.5, kind: 'on-axis' }), RAY)
    tool.onKey({ key: '1' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[2]).toBeLessThan(0)
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
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: -0.5, kind: 'on-axis' }), RAY)
    typeAndCommit(tool, '0.5')

    expect(pushPullDistance(scene)).toBeLessThan(0)
  })

  it('explicit-negative typed beats an OUTWARD live drag (the typed sign wins)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    // The cursor is dragging OUTWARD (+Z), yet the explicit typed `-` must win.
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0.5, kind: 'on-axis' }), RAY)
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
    expect(tool.statusHint()).toContain('Click a face')
  })
})
