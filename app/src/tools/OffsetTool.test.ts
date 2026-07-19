/**
 * OffsetTool unit tests — Path A (object face → `offset_face`) and Path B
 * (sketch region → gesture-bracketed `sketch_offset_region`), drag-sign
 * semantics, typed VCB entry, and Esc. Mirrors the fake-WasmScene pattern
 * used by PushPullTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { OffsetTool } from './OffsetTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

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

function makeFacePick(object: bigint, face: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => undefined,
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

/** A 2×2 square boundary on the ground (region) or at z=1 (face). */
function squareBoundary(z: number): Float32Array {
  return new Float32Array([0, 0, z, 2, 0, z, 2, 2, z, 0, 2, z])
}

function makeEdgePick(sketch: bigint, edge: bigint) {
  return {
    sketch: () => sketch,
    edge: () => edge,
    free: vi.fn(),
  }
}

function makeWasmScene(opts: {
  facePick?: ReturnType<typeof makeFacePick>
  regionPick?: ReturnType<typeof makeRegionPick>
  edgePick?: ReturnType<typeof makeEdgePick>
  edgeEndpoints?: Float64Array
  /** `sketch_plane` result for the region's sketch — `[px,py,pz,nx,ny,nz]`
   *  (default: ground, normal +Z). `undefined` simulates a stale handle. */
  sketchPlane?: [number, number, number, number, number, number] | undefined
} = {}): WasmScene {
  const offsetReport = {
    new_edges: () => new BigUint64Array([]),
    new_curves: () => new BigUint64Array([]),
    regions_created: () => new BigUint64Array([11n]),
    regions_removed: () => new BigUint64Array([]),
    free: vi.fn(),
  }
  const sketchPlane = 'sketchPlane' in opts ? opts.sketchPlane : [0, 0, 0, 0, 0, 1]
  return {
    pick_face: vi.fn(() => opts.facePick),
    pick_sketch_region: vi.fn(() => opts.regionPick),
    pick_sketch_edge: vi.fn(() => opts.edgePick),
    sketch_edge_endpoints: vi.fn(() => opts.edgeEndpoints),
    sketch_regions: vi.fn(() => BigUint64Array.from([7n])),
    sketch_plane: vi.fn(() => (sketchPlane !== undefined ? new Float64Array(sketchPlane) : undefined)),
    face_boundary: vi.fn(() => squareBoundary(1)),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    region_boundary: vi.fn(() => squareBoundary(0)),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_offset_region: vi.fn(() => offsetReport),
    sketch_offset_region_preview: vi.fn(() => new Float64Array([1, 3, 0.5, 0.5, 0, 1.5, 0.5, 0, 1, 1.5, 0])),
    offset_face: vi.fn(() => 77n),
    offset_face_preview: vi.fn(() => new Float64Array([0.5, 0.5, 1, 1.5, 0.5, 1, 1, 1.5, 1])),
  } as unknown as WasmScene
}

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
  it('click, drag inside, click commits a negative (inward) gesture-bracketed offset', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, onCommit, onToast } = makeTool(scene)

    // Anchor on the region, then drag to a point 0.5 m inside the boundary.
    tool.onPointerDown(null, rayAt(1, 1))
    expect(tool.capturingInput()).toBe(true)
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))

    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(9n)
    expect(scene.sketch_offset_region).toHaveBeenCalledTimes(1)
    const call = (scene.sketch_offset_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(9n)
    expect(call[1]).toBe(7n)
    expect(call[2]).toBeCloseTo(-0.5)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(9n)
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

    const call = (scene.sketch_offset_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[2]).toBeCloseTo(0.75)
  })

  it('renders the kernel-computed preview loops while dragging', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.5))

    expect(scene.sketch_offset_region_preview).toHaveBeenCalled()
    expect(preview.children.length).toBeGreaterThan(0)
  })

  it('a preview the kernel refuses (collapse) draws nothing but keeps the drag alive', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    ;(scene.sketch_offset_region_preview as ReturnType<typeof vi.fn>).mockImplementation(() => {
      throw new Error('OffsetCollapsed: the offset distance collapses the boundary')
    })
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.999))

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
    expect(scene.sketch_offset_region).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)

    // Back over the plane, the drag resumes and commits normally.
    tool.onPointerMove(null, rayAt(1, 0.5))
    tool.onPointerDown(null, rayAt(1, 0.5))
    expect(scene.sketch_offset_region).toHaveBeenCalledTimes(1)
  })

  it('is suppressed inside an editing context (region offset is a top-level act)', () => {
    const regionPick = makeRegionPick(9n, 7n)
    const scene = makeWasmScene({ regionPick })
    const { tool } = makeTool(scene)
    tool.setActiveContext(1n)

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
    const scene = makeWasmScene({ regionPick, sketchPlane: [0, 0, 0, 0, 1, 0] })
    // A 2x2 square boundary lying in the XZ plane (y = 0).
    ;(scene.region_boundary as ReturnType<typeof vi.fn>).mockReturnValue(
      new Float32Array([0, 0, 0, 2, 0, 0, 2, 0, 2, 0, 0, 2]),
    )
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

    expect(scene.sketch_offset_region).toHaveBeenCalledTimes(1)
    const call = (scene.sketch_offset_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(9n)
    expect(call[1]).toBe(7n)
    expect(call[2]).toBeCloseTo(-0.5) // 0.5 m inside the boundary along Z
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

    expect(scene.sketch_offset_region).toHaveBeenCalledTimes(1)
    const call = (scene.sketch_offset_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(9n)
    expect(call[1]).toBe(7n)
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
    expect(scene.sketch_offset_region).not.toHaveBeenCalled()
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
    tool.setActiveContext(1n)
    tool.onPointerDown(null, rayAt(1, 0))
    expect(scene.pick_sketch_edge).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
  })
})

describe('OffsetTool — Path A (object face)', () => {
  it('click, drag, click commits offset_face with the picked handles', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool, onFaceImprint, onToast } = makeTool(scene)

    tool.onPointerDown(null, rayAt(1, 1))
    tool.onPointerMove(null, rayAt(1, 0.25))
    tool.onPointerDown(null, rayAt(1, 0.25))

    expect(scene.offset_face).toHaveBeenCalledTimes(1)
    const call = (scene.offset_face as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(4n)
    expect(call[2]).toBeCloseTo(-0.25)
    expect(onFaceImprint).toHaveBeenCalledWith(3n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('surfaces a kernel refusal as a toast, back at idle', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    ;(scene.offset_face as ReturnType<typeof vi.fn>).mockImplementation(() => {
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

    const call = (scene.sketch_offset_region as ReturnType<typeof vi.fn>).mock.calls[0]
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

    const call = (scene.offset_face as ReturnType<typeof vi.fn>).mock.calls[0]
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

    const call = (scene.sketch_offset_region as ReturnType<typeof vi.fn>).mock.calls[0]
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

    const call = (scene.offset_face as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[2]).toBeCloseTo(0.4)
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
    expect(scene.sketch_offset_region).not.toHaveBeenCalled()
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
    expect(tool.statusHint()).toContain('Click a face')
  })
})
