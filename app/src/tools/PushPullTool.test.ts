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
} = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.facePick),
    pick_sketch_region: vi.fn(() => opts.regionPick),
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
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
