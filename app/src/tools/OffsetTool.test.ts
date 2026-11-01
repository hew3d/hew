/**
 * OffsetTool unit tests — the region under the cursor (a face's, a free
 * sketch's, the ground's) resolves through the plane's `plane_units_json`
 * listing and commits as ONE draft on that context
 * (`draft_begin_ctx` + `draft_offset_region`), drag-sign semantics, typed
 * VCB entry, retype, and Esc. Mirrors the fake-WasmScene pattern used by
 * PushPullTool.test.ts.
 */
import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import * as THREE from 'three'
import { OffsetTool } from './OffsetTool'
import type { Snap } from './types'
import { setLengthUnit, getLengthUnit, setPrecision, DEFAULT_PRECISION } from '../settings/units'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { ctxOfPlane, keyToId } from './planeUnits'
import { draftMock, type DraftLog } from './draftMock'
import { planeUnitsMock, ringUnit, hexKey, type MockListing, type MockRegion } from './planeUnitsMock'
import { SiblingPreviewMirror } from '../viewport/siblingPreview'
import { isSessionGhost } from './transformPreview'

/** Ray straight down at (x, y). */
function rayAt(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, preventDefault: () => {} } as unknown as KeyboardEvent
}

function makeFacePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    depth: () => 1,
    instance: () => instance,
    free: vi.fn(),
  }
}

function makeRegionPick(sketch: bigint, region: bigint) {
  return {
    sketch: () => sketch,
    region: () => region,
    free: vi.fn(),
  }
}

function makeEdgePick(sketch: bigint, edge: bigint) {
  return {
    sketch: () => sketch,
    edge: () => edge,
    free: vi.fn(),
  }
}

type P = [number, number, number]
/** A 2×2 square ring at height z. */
const square = (z: number): P[] => [[0, 0, z], [2, 0, z], [2, 2, z], [0, 2, z]]

/** The ground context and the context of the face at z = 1 (+Z). */
const CTX_GROUND = ctxOfPlane([0, 0, 0], [0, 0, 1]) as string
const CTX_FACE = ctxOfPlane([0, 0, 1], [0, 0, 1]) as string
/** The ground plane inside the entered instance 42 (same plane, instance-tagged context). */
const CTX_GROUND_IN_42 = ctxOfPlane([0, 0, 0], [0, 0, 1], 42n) as string
/** The key of the one region the default listings hold. */
const REGION = 7

/** A free profile `(sketch 9, region 7)` on the ground. */
const GROUND_LISTING: MockListing = {
  units: [ringUnit(1, 21, square(0))],
  regions: [{ key: REGION, outer: square(0), extrude: { sketch: 9, region: 7 } }],
}
/** A drawn square on a box top (object 3, face 4) at z = 1. */
const FACE_LISTING: MockListing = {
  units: [ringUnit(2, 31, square(1), { kind: 'topology', object: 3 })],
  regions: [{ key: REGION, outer: square(1), over: { object: 3, face: 4 } }],
}

function makeWasmScene(opts: {
  facePick?: ReturnType<typeof makeFacePick>
  regionPick?: ReturnType<typeof makeRegionPick>
  /** `pick_sketch_region_in_instance`'s own result, when it must differ
   *  from `regionPick` (defaults to `regionPick` otherwise) — mirrors
   *  PushPullTool.test.ts's `makeWasmScene`. */
  regionPickInInstance?: ReturnType<typeof makeRegionPick>
  edgePick?: ReturnType<typeof makeEdgePick>
  edgeEndpoints?: Float64Array
  /** `sketch_plane` result for the region's sketch — `[px,py,pz,nx,ny,nz]`
   *  (default: ground, normal +Z). `undefined` simulates a stale handle. */
  sketchPlane?: [number, number, number, number, number, number] | undefined
  /** `instance_pose` result (default identity). `undefined` simulates a
   *  stale instance. */
  instancePose?: Float64Array | undefined
  /** `node_parent` result (default none: a plain top-level object). */
  nodeParent?: unknown
  /** Plane listings by context (default: the ground square and the face square). */
  listings?: Record<string, MockListing>
  /** Objects the committed draft reports as changed. */
  ctxObjects?: bigint[]
} = {}): WasmScene & { draftLog: DraftLog } {
  const sketchPlane = 'sketchPlane' in opts ? opts.sketchPlane : [0, 0, 0, 0, 0, 1]
  const instancePose = 'instancePose' in opts
    ? opts.instancePose
    : new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])
  const dm = draftMock({ ctxObjects: opts.ctxObjects ?? (opts.facePick !== undefined ? [3n] : []) })
  const units = planeUnitsMock(opts.listings ?? { [CTX_GROUND]: GROUND_LISTING, [CTX_FACE]: FACE_LISTING, [CTX_GROUND_IN_42]: GROUND_LISTING })
  return {
    draftLog: dm.log,
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => opts.facePick),
    session_stack: vi.fn(() => []),
    pick_sketch_region: vi.fn(() => opts.regionPick),
    pick_sketch_region_in_instance: vi.fn(() => opts.regionPickInInstance ?? opts.regionPick),
    pick_sketch_edge: vi.fn(() => opts.edgePick),
    sketch_edge_endpoints: vi.fn(() => opts.edgeEndpoints),
    sketch_plane: vi.fn(() => (sketchPlane !== undefined ? new Float64Array(sketchPlane) : undefined)),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    instance_pose: vi.fn(() => instancePose),
    node_parent: vi.fn(() => opts.nodeParent),
    ...units,
    ...dm.members,
  } as unknown as WasmScene & { draftLog: DraftLog }
}

/** The `draft_offset_region` calls of a scene: `[draft, regionKey, distance]`. */
const offsetCalls = (scene: WasmScene) => (scene.draft_offset_region as ReturnType<typeof vi.fn>).mock.calls as [bigint, bigint, number][]
/** The context strings the scene's drafts opened on. */
const beganCtxs = (scene: WasmScene) => (scene as unknown as { draftLog: DraftLog }).draftLog.begins.map((b) => b.args[0] as string)

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new OffsetTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement }
}

describe('OffsetTool — Path B (sketch region)', () => {
  it('click, drag inside, click commits a negative (inward) offset in one draft', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, onCommit, onToast } = makeTool(scene)

    // Anchor on the region, then drag to a point 0.5 m inside the boundary.
    tool.onPointerDown(null, rayAt(1, 1))
    expect(tool.capturingInput()).toBe(true)
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))

    // ONE draft on the ground's context holds the whole offset.
    expect(beganCtxs(scene)).toEqual([CTX_GROUND])
    expect(offsetCalls(scene)).toHaveLength(1)
    const call = offsetCalls(scene)[0]
    expect(call[1]).toBe(keyToId(hexKey(REGION)))
    expect(call[2]).toBeCloseTo(-0.5)
    expect((scene as unknown as { draftLog: DraftLog }).draftLog.ended).toBe(1)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('dragging outside the boundary commits a positive (outward) offset', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, -0.75))
    tool.onPointerDown(null, rayAt(1, -0.75))

    const call = offsetCalls(scene)[0]
    expect(call[2]).toBeCloseTo(0.75)
  })

  it('renders the offset ghost loop while dragging', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))

    expect(preview.children.length).toBeGreaterThan(0)
    // Nothing reaches the kernel until the commit click.
    expect(scene.draft_begin_ctx).not.toHaveBeenCalled()
  })

  it('a collapsing distance draws no ghost but keeps the drag alive', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    expect(preview.children.length).toBeGreaterThan(0)
    // The centre of the 2 m square: an inward offset of 1 m closes it up.
    tool.onPointerMove(null, rayAt(1, 1))

    expect(preview.children).toHaveLength(0)
    expect(tool.capturingInput()).toBe(true)
  })

  it('a cursor with no in-plane position clears the preview and makes a click a no-op', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, preview, onMeasurement } = makeTool(scene)

    // Valid drag first: preview + live readout appear.
    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    expect(preview.children.length).toBeGreaterThan(0)

    // Sweep past the plane's horizon: the ray points away from the ground
    // plane (t < 0), so there is no in-plane cursor position. The stale
    // loop and readout must clear, not freeze.
    const awayRay: Ray = { origin: [1, 1, 5], direction: [0, 0, 1] }
    tool.onPointerMove(null, awayRay)
    expect(preview.children).toHaveLength(0)
    expect(onMeasurement).toHaveBeenLastCalledWith('')

    // A click in that state commits nothing — the memorized distance from
    // the earlier valid move must never be committed blind.
    tool.onPointerDown(null, awayRay)
    expect(offsetCalls(scene)).toHaveLength(0)
    expect(tool.capturingInput()).toBe(true)

    // Back over the plane, the drag resumes and commits normally.
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))
    expect(offsetCalls(scene)).toHaveLength(1)
  })

  it('is suppressed inside an editing context (region offset is a top-level act)', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 1n })

    tool.onPointerDown(null, rayAt(1, 1))

    expect(scene.pick_sketch_region).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
  })
})

// Sketches on any plane (Phase 1, the sketch-planes design §3): the
// region branch's plane normal must come from `sketch_plane`, not a
// hardcoded [0,0,1] — otherwise a rotated sketch's offset distance would be
// measured against the wrong plane.
describe('OffsetTool — Path B on a rotated sketch (sketch_plane normal)', () => {
  it('measures the offset in the sketch\'s OWN plane (XZ, normal +Y), not the ground XY plane', () => {
    const regionPick = makeRegionPick(9n, 7n)
    // A 2x2 square lying in the XZ plane (y = 0), listed under that plane's context.
    const xz: P[] = [[0, 0, 0], [2, 0, 0], [2, 0, 2], [0, 0, 2]]
    const ctxXZ = ctxOfPlane([0, 0, 0], [0, 1, 0]) as string
    const scene = makeWasmScene({
      regionPick,
      sketchPlane: [0, 0, 0, 0, 1, 0],
      listings: { [ctxXZ]: { units: [ringUnit(1, 21, xz)], regions: [{ key: REGION, outer: xz, extrude: { sketch: 9, region: 7 } }] } },
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    // A ray straight down -Y — parallel to the ground (XY) plane, so a
    // ground-normal fallback would never intersect anything; only the
    // sketch's true plane (Y-normal) makes this drag/commit work at all.
    const rayXZ = (x: number, z: number): Ray => ({ origin: [x, 5, z], direction: [0, -1, 0] })

    tool.onPointerDown(null, rayXZ(1, 1))
    expect(scene.sketch_plane).toHaveBeenCalledWith(9n)
    expect(tool.capturingInput()).toBe(true)
    tool.onPointerMove(null, rayXZ(1, 0.5))
    tool.onPointerDown(null, rayXZ(1, 0.5))

    expect(offsetCalls(scene)).toHaveLength(1)
    const call = offsetCalls(scene)[0]
    expect(call[1]).toBe(keyToId(hexKey(REGION)))
    expect(call[2]).toBeCloseTo(-0.5) // 0.5 m inside the boundary along Z
    expect(beganCtxs(scene)).toEqual([ctxXZ])
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a stale sketch handle (sketch_plane undefined) leaves the tool idle instead of falling back to ground', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick, sketchPlane: undefined })
    const { tool } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))

    expect(scene.sketch_plane).toHaveBeenCalledWith(9n)
    expect(tool.capturingInput()).toBe(false)
  })
})

describe('OffsetTool — Path B, click ON a boundary edge (edge fallback)', () => {
  // The maintainer's repro: F, click a 5 cm square's edge — nothing. The
  // region pick is interior-only, so a click landing on the visible edge
  // line (which is exactly where Offset invites you to click) always
  // missed. The fallback resolves the picked edge to the region whose
  // outer boundary contains it — one click on an edge OR the interior
  // must pick the region.
  it('one click on a region edge starts the drag on that region', () => {
    const scene = makeWasmScene({
      edgePick: makeEdgePick(9n, 33n),
      // The bottom edge of squareBoundary(0).
      edgeEndpoints: new Float64Array([0, 0, 0, 2, 0, 0]),
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 0)) // ON the edge — region pick misses
    expect(tool.capturingInput()).toBe(true)

    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))

    expect(offsetCalls(scene)).toHaveLength(1)
    const call = offsetCalls(scene)[0]
    expect(call[1]).toBe(keyToId(hexKey(REGION)))
    expect(call[2]).toBeCloseTo(-0.5)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('reversed endpoint order still resolves the owning region', () => {
    const scene = makeWasmScene({
      edgePick: makeEdgePick(9n, 33n),
      edgeEndpoints: new Float64Array([2, 0, 0, 0, 0, 0]),
    })
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, rayAt(1, 0))
    expect(tool.capturingInput()).toBe(true)
  })

  it('the wrap-around boundary segment (last vertex back to first) matches too', () => {
    const scene = makeWasmScene({
      edgePick: makeEdgePick(9n, 34n),
      // squareBoundary(0) closes (0,2) -> (0,0).
      edgeEndpoints: new Float64Array([0, 2, 0, 0, 0, 0]),
    })
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, rayAt(0, 1))
    expect(tool.capturingInput()).toBe(true)
  })

  it('an edge on no region boundary leaves the tool idle', () => {
    const scene = makeWasmScene({
      edgePick: makeEdgePick(9n, 35n),
      edgeEndpoints: new Float64Array([5, 5, 0, 6, 5, 0]), // a stray line
    })
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, rayAt(5.5, 5))
    expect(tool.capturingInput()).toBe(false)
    expect(offsetCalls(scene)).toHaveLength(0)
  })

  it('a stale edge (endpoints gone) leaves the tool idle instead of throwing', () => {
    const scene = makeWasmScene({
      edgePick: makeEdgePick(9n, 36n),
      edgeEndpoints: undefined,
    })
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, rayAt(1, 0))
    expect(tool.capturingInput()).toBe(false)
  })

  it('stays out of editing contexts (region offset is a top-level act)', () => {
    const scene = makeWasmScene({
      edgePick: makeEdgePick(9n, 33n),
      edgeEndpoints: new Float64Array([0, 0, 0, 2, 0, 0]),
    })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 1n })
    tool.onPointerDown(null, rayAt(1, 0))
    expect(scene.pick_sketch_edge).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
  })
})

describe('OffsetTool — Path A (object face)', () => {
  it('click, drag, click commits ONE draft on the face\'s plane context', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool, onFaceImprint, onToast } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.25))
    tool.onPointerDown(null, rayAt(1, 0.25))

    expect(offsetCalls(scene)).toHaveLength(1)
    const call = offsetCalls(scene)[0]
    expect(beganCtxs(scene)).toEqual([CTX_FACE])
    expect(call[1]).toBe(keyToId(hexKey(REGION)))
    expect(call[2]).toBeCloseTo(-0.25)
    expect(onFaceImprint).toHaveBeenCalledWith(3n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a face of an object inside a group (not entered) shows the ineligible-face hint, never a silent no-op', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick, nodeParent: { kind: 1, id: 9n } })
    const { tool, onToast, onFaceImprint } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/inside a group/i)
    expect(tool.capturingInput()).toBe(false)
    expect(beganCtxs(scene)).toEqual([])
    expect(onFaceImprint).not.toHaveBeenCalled()
  })

  it('surfaces a kernel refusal as a toast, back at idle', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    ;(scene.draft_end as ReturnType<typeof vi.fn>).mockImplementation(() => {
      throw new Error('OffsetCollapsed: the offset distance collapses the boundary')
    })
    const { tool, onToast, onFaceImprint } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.25))
    tool.onPointerDown(null, rayAt(1, 0.25))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('OffsetCollapsed')
    expect(onFaceImprint).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
    // The refused draft was cancelled, leaving the document untouched.
    expect((scene as unknown as { draftLog: DraftLog }).draftLog.cancelled).toBe(1)
  })

  it('an OUTWARD offset past the face edge is accepted (the kernel splits it into face and free parts)', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick, ctxObjects: [3n] })
    const { tool, onToast, onFaceImprint, onCommit } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, -0.5))
    tool.onPointerDown(null, rayAt(1, -0.5))

    expect(onToast).not.toHaveBeenCalled()
    expect(beganCtxs(scene)).toEqual([CTX_FACE])
    expect(offsetCalls(scene)[0][2]).toBeCloseTo(0.5)
    expect(onFaceImprint).toHaveBeenCalledWith(3n)
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('a report naming objects AND sketches refreshes both (a straddling offset)', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick, ctxObjects: [3n] })
    ;(scene.draft_end as ReturnType<typeof vi.fn>).mockReturnValue(
      JSON.stringify({ objects: [3], sketches: [9], kept_as_sketch: [] }),
    )
    const { tool, onFaceImprint, onCommit } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, -0.5))
    tool.onPointerDown(null, rayAt(1, -0.5))

    expect(onFaceImprint).toHaveBeenCalledWith(3n)
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('a click on the boundary line of a drawn shape on the face picks that shape, not the face around it', () => {
    const inner: P[] = [[0.5, 0.5, 1], [1.5, 0.5, 1], [1.5, 1.5, 1], [0.5, 1.5, 1]]
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      listings: {
        [CTX_FACE]: {
          units: [ringUnit(2, 31, inner, { kind: 'topology', object: 3 })],
          regions: [
            { key: 8, outer: inner, over: { object: 3, face: 5 } },
            { key: 9, outer: square(1), holes: [inner], over: { object: 3, face: 4 } },
          ],
        },
      },
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 0.5)) // ON the inner square's bottom edge
    tool.onPointerMove(null, rayAt(1, 0.25))
    tool.onPointerDown(null, rayAt(1, 0.25))

    expect(offsetCalls(scene)[0][1]).toBe(keyToId(hexKey(8)))
  })
})

describe('OffsetTool — typed distance (VCB)', () => {
  it('Enter commits the typed magnitude with the live drag sign', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.9)) // inward drag (negative)
    for (const key of ['0', '.', '7', '5']) {
      tool.onKey(makeKeyEvent(key))
    }
    tool.onKey(makeKeyEvent('Enter'))

    const call = offsetCalls(scene)[0]
    expect(call[2]).toBeCloseTo(-0.75)
  })

  it('with no drag movement a face offset defaults inward', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    for (const key of ['0', '.', '2']) {
      tool.onKey(makeKeyEvent(key))
    }
    tool.onKey(makeKeyEvent('Enter'))

    const call = offsetCalls(scene)[0]
    expect(call[2]).toBeCloseTo(-0.2)
  })

  it('an off-plane excursion keeps an inward region drag inward for typed entry', () => {
    // Drag INWARD (cursor inside the boundary — opposite the region
    // default of outward), sweep past the plane's horizon, then type: the
    // commit must keep the inward sign the user established, not fall back
    // to the default.
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool } = makeTool(scene)
    const awayRay: Ray = { origin: [1, 1, 5], direction: [0, 0, 1] }

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5)) // inward (negative)
    tool.onPointerMove(null, awayRay) // off the plane
    for (const key of ['0', '.', '3']) {
      tool.onKey(makeKeyEvent(key))
    }
    tool.onKey(makeKeyEvent('Enter'))

    const call = offsetCalls(scene)[0]
    expect(call[2]).toBeCloseTo(-0.3)
  })

  it('an off-plane excursion keeps an outward face drag outward for typed entry', () => {
    // The mirror case on a face, against the face default of inward: an
    // OUTWARD drag stays outward across the excursion (the kernel then
    // refuses it with its own typed error — the sign must still be the
    // user's, never silently flipped to the default).
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool } = makeTool(scene)
    const awayRay: Ray = { origin: [1, 1, 5], direction: [0, 0, 1] }

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, -0.25)) // outward (positive)
    tool.onPointerMove(null, awayRay) // off the plane
    for (const key of ['0', '.', '4']) {
      tool.onKey(makeKeyEvent(key))
    }
    tool.onKey(makeKeyEvent('Enter'))

    const call = offsetCalls(scene)[0]
    expect(call[2]).toBeCloseTo(0.4)
  })
})

describe('OffsetTool — hover hint (Tool.hoverHint)', () => {
  const RAY: Ray = { origin: [0.5, 0.5, 5], direction: [0, 0, -1] }
  it('hovering a component member\'s face explains the refusal before the click, and the click toasts the same line as guidance', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 12n) })
    const { tool, onToast } = makeTool(scene)
    expect(tool.hoverHint()).toBeNull()
    tool.onPointerMove(null, RAY)
    expect(tool.hoverHint()).toContain('component')
    expect(tool.statusHint()).toContain('component')
    expect(onToast).not.toHaveBeenCalled()
    tool.onPointerDown(null, RAY)
    expect(String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])).toContain('component')
    expect((onToast as ReturnType<typeof vi.fn>).mock.calls[0][1]).toBe('EditContextHint')
    tool.onPointerLeave()
    expect(tool.hoverHint()).toBeNull()
  })

  it('a plain face hovers with no hint', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool } = makeTool(scene)
    tool.onPointerMove(null, RAY)
    expect(tool.hoverHint()).toBeNull()
  })
})

describe('OffsetTool — cancel and status hint', () => {
  it('Esc cancels the drag and clears the preview', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, preview, onMeasurement } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onKey(makeKeyEvent('Escape'))

    expect(tool.capturingInput()).toBe(false)
    expect(preview.children).toHaveLength(0)
    expect(onMeasurement).toHaveBeenLastCalledWith('')
    expect(offsetCalls(scene)).toHaveLength(0)
  })

  it('guidance follows the stage', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool } = makeTool(scene)

    expect(tool.statusHint()).toContain('Click a face')
    tool.onPointerDown(null, rayAt(1, 1))
    expect(tool.statusHint()).toContain('click to commit')
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))
    // Committed: the retype window offers to redo the offset at a typed
    // distance; Escape closes it and the pick guidance returns.
    expect(tool.statusHint()).toContain('redo the offset')
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.statusHint()).toContain('Click a face')
  })
})

describe('OffsetTool — instance editing context (component-edit-parity.md phase A2)', () => {
  const INSTANCE = 42n
  const COMPONENT = 5n

  it('a component member face is refused with a clear hint', () => {
    const facePick = makeFacePick(3n, 4n, INSTANCE)
    const scene = makeWasmScene({ facePick })
    const { tool, onToast } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(null, rayAt(1, 1))

    expect(tool.capturingInput()).toBe(false)
    expect(scene.draft_begin_ctx).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(String(onToast.mock.calls[0][0])).toContain('sketch region')
  })

  it('a def-owned sketch region is reachable via pick_sketch_region_in_instance, not the world-only pick_sketch_region', () => {
    // pick_sketch_region only ever walks Document::sketch_ids() (world-tree
    // sketches) — it can NEVER find a region drawn inside a component's own
    // definition. Seed it with a hit anyway (the false-confidence mock this
    // finding calls out) to prove the REAL pick that fires is the
    // `_in_instance` sibling, exactly like PushPullTool's identical Path B.
    const regionPickInInstance = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({
      regionPick: makeRegionPick(999n, 999n), // world-only pick — must be ignored
      regionPickInInstance,
    })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))

    expect(scene.pick_sketch_region_in_instance).toHaveBeenCalled()
    const call = (scene.pick_sketch_region_in_instance as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(INSTANCE)
    expect(scene.pick_sketch_region).not.toHaveBeenCalled()

    expect(offsetCalls(scene)).toHaveLength(1)
    // The draft opened on the instance-tagged context of the def-owned sketch's plane.
    expect(beganCtxs(scene)).toEqual([CTX_GROUND_IN_42])
  })

  it('a miss from pick_sketch_region_in_instance (no def-owned region under the cursor) offsets nothing', () => {
    const scene = makeWasmScene({ facePick: undefined, regionPickInInstance: undefined, edgePick: undefined })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(null, rayAt(1, 1))

    expect(scene.pick_sketch_region_in_instance).toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
    expect(offsetCalls(scene)).toHaveLength(0)
  })

  // The boundary/normal this drag measures against (`region_boundary`/
  // `sketch_plane`) answer in DEFINITION-LOCAL space for a def-owned sketch
  // — `_beginRegionDrag` must pose-map them into WORLD space (same fix
  // class as the region PICK itself) or the drag's signed-distance math
  // silently measures against the wrong (unposed) boundary location.
  it('a def-owned region drag is posed into WORLD space under a translated instance', () => {
    const regionPickInInstance = makeRegionPick(99n, 7n)
    const shifted = square(0).map(([x, y, z]): P => [x + 10, y, z])
    const scene = makeWasmScene({
      regionPickInInstance,
      instancePose: new Float64Array([1, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 0]), // +10 in X
      listings: {
        [CTX_GROUND_IN_42]: {
          units: [ringUnit(1, 21, shifted)],
          regions: [{ key: REGION, outer: shifted, extrude: { sketch: 99, region: 7 } }],
        },
      },
    })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    // The LOCAL square (0,0)-(2,2) sits at WORLD x∈[10,12], y∈[0,2] under
    // this pose — anchor/drag at the WORLD position, mirroring the
    // identity-pose sibling test in "Path B (sketch region)" exactly,
    // shifted by the pose's +10 in X.
    tool.onPointerDown(null, rayAt(11, 1))
    tool.onPointerMove(null, rayAt(11, 0.5))
    tool.onPointerDown(null, rayAt(11, 0.5))

    expect(offsetCalls(scene)).toHaveLength(1)
    const call = offsetCalls(scene)[0]
    expect(call[2]).toBeCloseTo(-0.5)
  })

  // The edge-pick fallback (`_pickRegionViaEdge`) has no `_in_instance`
  // sibling — it can ONLY ever find a WORLD sketch, even while editing an
  // instance. `_beginRegionDrag` must NOT re-map a region found this way
  // through the active instance's pose, or an already-correct world
  // boundary gets silently corrupted.
  it('a WORLD sketch region found via the edge-pick fallback is NOT re-mapped by the active instance pose', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const scene = makeWasmScene({
      regionPickInInstance: undefined, // interior pick misses — falls to the edge fallback
      edgePick,
      edgeEndpoints: new Float64Array([0, 0, 0, 2, 0, 0]),
      instancePose: new Float64Array([1, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 0]), // +10 in X
    })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    // The WORLD square sits at its own raw (0,0)-(2,2) — no instance pose
    // applies to it — so anchor/drag at the SAME coordinates the identity-
    // pose sibling test uses, not shifted by the active instance's +10.
    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))

    expect(offsetCalls(scene)).toHaveLength(1)
    const call = offsetCalls(scene)[0]
    expect(call[2]).toBeCloseTo(-0.5)
  })
})

describe('OffsetTool — top-level click on an instance face (component-edit-parity.md phase A2)', () => {
  it('an ordinary top-level click on a placed component instance\'s face toasts, instead of silently doing nothing', () => {
    // The far more common case than the "inside its own edit context" test
    // above: no edit context entered at all (_activeInstance === null), the
    // ordinary way a user encounters a placed component. Previously the
    // early-return only toasted when _activeInstance !== null, so this
    // exact click armed no drag, showed no toast, and gave zero feedback.
    const facePick = makeFacePick(3n, 4n, 77n) // instanced geometry, no context entered
    const scene = makeWasmScene({ facePick })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))

    expect(tool.capturingInput()).toBe(false)
    expect(scene.draft_begin_ctx).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(String(onToast.mock.calls[0][0])).toContain('sketch region')
  })
})

describe('OffsetTool — setEditContext aborts an armed gesture on a genuine change (component-edit-parity.md phase A2)', () => {
  it('a genuine context change cancels an armed drag instead of silently retargeting its eventual commit', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })
    tool.onPointerDown(null, rayAt(1, 1))
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'top' })

    expect(tool.capturingInput()).toBe(false)
    tool.onPointerDown(null, rayAt(1, 0.5))
    expect(offsetCalls(scene)).toHaveLength(0) // the cancelled gesture never commits
  })

  it('re-pushing the SAME context is a no-op — an armed drag survives it untouched', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool } = makeTool(scene)
    const ctx = { kind: 'instance' as const, id: 42n, component: 5n }
    tool.setEditContext(ctx)
    tool.onPointerDown(null, rayAt(1, 1))
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })

    expect(tool.capturingInput()).toBe(true)
  })
})

// Post-commit distance retype (retypeWindow.ts): type a distance after the
// commit click and the offset just made is redone at that distance.
describe('OffsetTool — retype the distance after the commit', () => {
  function makeRetypeScene(opts: Parameters<typeof makeWasmScene>[0] = {}) {
    const base = makeWasmScene(opts) as unknown as Record<string, unknown>
    let gen = 1n
    const baseEnd = base.draft_end as (d: bigint) => string
    const scene = {
      ...base,
      history_generation: vi.fn(() => gen),
      draft_end: vi.fn((d: bigint) => { const r = baseEnd(d); gen += 1n; return r }),
      scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
      scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    }
    return scene as unknown as WasmScene
  }
  const typeIn = (tool: OffsetTool, text: string) => { for (const ch of text) tool.onKey(makeKeyEvent(ch)); tool.onKey(makeKeyEvent('Enter')) }
  const regionDistances = (scene: WasmScene) => offsetCalls(scene).map((c) => c[2])

  it('a profile offset redone at a typed distance keeps its inward direction; a negative value flips it', () => {
    const scene = makeRetypeScene({ regionPick: makeRegionPick(9n, 7n) })
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))
    const first = regionDistances(scene)
    expect(first.length).toBe(1)
    expect(first[0]).toBeLessThan(0) // inward
    expect(tool.statusHint()).toContain('redo the offset')
    typeIn(tool, '0.2')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(regionDistances(scene)[1]).toBeCloseTo(-0.2, 9)
    expect(offsetCalls(scene)[1][1]).toBe(keyToId(hexKey(REGION)))
    typeIn(tool, '-0.2')
    expect(regionDistances(scene)[2]).toBeCloseTo(0.2, 9)
    expect(onCommit).toHaveBeenCalledTimes(3)
  })

  it('a face offset retypes through a fresh draft on the face context', () => {
    const scene = makeRetypeScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onFaceImprint } = makeTool(scene)
    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))
    expect(offsetCalls(scene)).toHaveLength(1)
    typeIn(tool, '0.3')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    const calls = offsetCalls(scene)
    expect(calls.length).toBe(2)
    expect(Math.abs(calls[1][2] as number)).toBeCloseTo(0.3, 9)
    expect(Math.sign(calls[1][2] as number)).toBe(Math.sign(calls[0][2] as number))
    expect(onFaceImprint).toHaveBeenCalledTimes(2)
  })

  it('a zero typed distance is refused with the usual toast; a new pick closes the window', () => {
    const scene = makeRetypeScene({ regionPick: makeRegionPick(9n, 7n) })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))
    typeIn(tool, '0')
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledWith('Move more before committing the offset')
    tool.onPointerDown(null, rayAt(1, 1)) // picks again → window closed
    expect(tool.capturingInput()).toBe(true)
  })
})


describe('OffsetTool — a clicked distance is exactly the displayed distance', () => {
  // These pin the Exact digits (9.399 mm); the default mm rounding is coarser.
  beforeAll(() => setPrecision('exact', 'mm'))
  afterAll(() => setPrecision(DEFAULT_PRECISION.mm, 'mm'))
  const prev = getLengthUnit()
  function drag(snap: Snap | null, y: number) {
    const scene = makeWasmScene({ regionPick: makeRegionPick(9n, 7n) })
    const { tool, onMeasurement } = makeTool(scene)
    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(snap, rayAt(1, y))
    tool.onPointerDown(snap, rayAt(1, y))
    return {
      d: offsetCalls(scene)[0][2] as number,
      onMeasurement,
    }
  }

  it('a free click outside the boundary commits the shown +12.346 mm', () => {
    setLengthUnit('mm')
    try {
      const r = drag(null, -0.0123456)
      expect(r.onMeasurement).toHaveBeenCalledWith('12.346 mm')
      expect(r.d).toBe(12.346 * 0.001)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('a free click inside commits the shown negative distance', () => {
    setLengthUnit('mm')
    try {
      expect(drag(makeSnap({ kind: 'on-face', x: 1, y: 0.0123456 }), 0.0123456).d).toBe(-12.346 * 0.001)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('a hard-snapped (endpoint) click keeps the exact projected distance', () => {
    setLengthUnit('mm')
    try {
      const r = drag(makeSnap({ kind: 'endpoint', x: 1, y: -0.0123456, z: 0 }), -0.0123456)
      expect(r.d).toBeCloseTo(0.0123456, 15)
      expect(r.d).not.toBe(12.346 * 0.001)
    } finally {
      setLengthUnit(prev)
    }
  })
})

describe('OffsetTool — the offset loops are session geometry, mirrored onto sibling placements', () => {
  it('the dragged loops are marked ghosts; the mirror shows one clone per loop per frame and clears with Esc', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, preview } = makeTool(scene)
    const root = new THREE.Group()
    const mirror = new SiblingPreviewMirror({ source: preview, root, frames: () => [1, 0, 0, 4, 0, 1, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 7, 0, 0, 1, 0] })

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))
    const ghosts = preview.children.filter(isSessionGhost)
    expect(ghosts.length).toBeGreaterThan(0)
    expect(ghosts).toHaveLength(preview.children.length) // every loop, nothing else
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)
    expect((root.children[0] as THREE.Group).children).toHaveLength(ghosts.length)

    tool.onKey({ key: 'Escape', preventDefault: () => {} } as unknown as KeyboardEvent)
    mirror.sync()
    expect(mirror.mirrorCount).toBe(0)
  })
})
