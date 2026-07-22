/**
 * SectionPlaneTool unit tests. Mirrors the fake-WasmScene pattern used by
 * PushPullTool.test.ts/CircleTool.test.ts.
 *
 * The offsetting-gesture tests need rays that are NOT parallel to the
 * section plane's normal (`projectRayOntoAxis` is degenerate for a
 * perfectly axis-parallel ray — the same accepted limitation
 * PushPullTool's own axis-drag has). `rayThroughZ` below is a fixed-
 * direction family of rays, each of which passes exactly through the world
 * point (0, 0, z) — algebraically verified in its own comment — so a
 * section plane at origin [0,0,0]/normal [0,0,1] sees a clean, predictable
 * axis-projected distance of exactly `z` for `rayThroughZ(z)`.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { SectionPlaneTool, OFFSET_DRAG_THRESHOLD_M } from './SectionPlaneTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import type { SectionPlane } from '../viewport/sectionManager'

/** Fixed ray direction: 45° between -X and -Z, deliberately not parallel to
 * the [0,0,1] test plane's normal. */
const DIR: [number, number, number] = [-Math.SQRT1_2, 0, -Math.SQRT1_2]

/** A ray whose closest approach to the world Z axis (through the origin)
 * lands at exactly world Z = `z`, for the fixed `DIR` above. Also passes
 * exactly through the point (0, 0, z) itself, so it intersects the
 * `z=0`-normal test plane's own family predictably as `z` varies. */
function rayThroughZ(z: number): Ray {
  return { origin: [3, 0, z], direction: DIR }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeWasmScene(faceNormal: [number, number, number] = [0, 0, 1]): WasmScene {
  return {
    face_plane: vi.fn(() => new Float64Array([0, 0, 0, faceNormal[0], faceNormal[1], faceNormal[2]])),
  } as unknown as WasmScene
}

function makeTool(opts: {
  scene?: WasmScene
  currentPlane?: SectionPlane | null
  widgetHalfExtent?: number
} = {}) {
  const scene = opts.scene ?? makeWasmScene()
  const preview = new THREE.Group()
  let currentPlane = opts.currentPlane ?? null
  const onPlace = vi.fn()
  const onOffsetPreview = vi.fn()
  const onOffsetCommit = vi.fn()
  const onToggle = vi.fn()
  const onDelete = vi.fn()
  const onCancelOffset = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new SectionPlaneTool(
    scene,
    preview,
    () => currentPlane,
    opts.widgetHalfExtent ?? 10,
    onPlace,
    onOffsetPreview,
    onOffsetCommit,
    onToggle,
    onDelete,
    onCancelOffset,
    onToast,
    onMeasurement,
  )
  return {
    tool, preview, onPlace, onOffsetPreview, onOffsetCommit, onToggle, onDelete, onCancelOffset,
    onToast, onMeasurement,
    setCurrentPlane: (p: SectionPlane | null) => { currentPlane = p },
  }
}

const GROUND_RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

describe('SectionPlaneTool — placement', () => {
  it('clicking a face places a section coincident with + normal to it', () => {
    const scene = makeWasmScene([0, 1, 0])
    const { tool, onPlace } = makeTool({ scene })

    const snap = makeSnap({ x: 1, y: 2, z: 3, kind: 'on-face', elementKind: 'face', object: 5n, element: 6n })
    tool.onPointerDown(snap, GROUND_RAY)

    expect(onPlace).toHaveBeenCalledTimes(1)
    const [origin, normal] = onPlace.mock.calls[0]
    expect(origin).toEqual([1, 2, 3])
    expect(normal).toEqual([0, 1, 0])
  })

  it('clicking empty ground places an axis-aligned horizontal plane AT the ground (Z=0)', () => {
    const { tool, onPlace } = makeTool()

    const snap = makeSnap({ x: 4, y: -2, z: 1.5, kind: 'ground' })
    tool.onPointerDown(snap, GROUND_RAY)

    expect(onPlace).toHaveBeenCalledTimes(1)
    const [origin, normal] = onPlace.mock.calls[0]
    expect(origin).toEqual([4, -2, 0])
    expect(normal).toEqual([0, 0, 1])
  })

  it('a null snap places nothing', () => {
    const { tool, onPlace } = makeTool()
    tool.onPointerDown(null, GROUND_RAY)
    expect(onPlace).not.toHaveBeenCalled()
  })

  it('re-placing on a different face REPLACES rather than accumulating (one section at a time)', () => {
    const { tool, onPlace, setCurrentPlane } = makeTool()

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, elementKind: 'face', object: 1n, element: 1n }), GROUND_RAY)
    setCurrentPlane({ origin: [0, 0, 0], normal: [0, 0, 1], active: true })

    // A second face click, well away from the (small, origin-centered)
    // current widget, still places — it doesn't get swallowed as a
    // widget-drag arm.
    const farRay: Ray = { origin: [50, 50, 5], direction: [0, 0, -1] }
    tool.onPointerDown(makeSnap({ x: 50, y: 50, z: 0, elementKind: 'face', object: 2n, element: 2n }), farRay)

    expect(onPlace).toHaveBeenCalledTimes(2)
    expect(onPlace.mock.calls[1][0]).toEqual([50, 50, 0])
  })

  it('normalizes a non-unit face normal', () => {
    const scene = makeWasmScene([0, 0, 5])
    const { tool, onPlace } = makeTool({ scene })
    tool.onPointerDown(makeSnap({ elementKind: 'face', object: 1n, element: 1n }), GROUND_RAY)
    expect(onPlace.mock.calls[0][1]).toEqual([0, 0, 1])
  })

  it('a non-face, non-ground snap (e.g. an edge/vertex) PRESERVES its Z instead of flattening to the floor', () => {
    const { tool, onPlace } = makeTool()
    // Inference prefers a vertex/edge snap near a raised face's corner —
    // that snap has no elementKind 'face' and no kind 'ground', so its real
    // height must survive (regression: it used to collapse to Z=0).
    tool.onPointerDown(makeSnap({ x: 2, y: 3, z: 4, kind: 'endpoint' }), GROUND_RAY)
    expect(onPlace).toHaveBeenCalledTimes(1)
    const [origin, normal] = onPlace.mock.calls[0]
    expect(origin).toEqual([2, 3, 4])
    expect(normal).toEqual([0, 0, 1]) // horizontal default is acceptable for v0.3.0
  })

  it('only a genuine ground snap flattens Z to 0', () => {
    const { tool, onPlace } = makeTool()
    tool.onPointerDown(makeSnap({ x: 2, y: 3, z: 9, kind: 'ground' }), GROUND_RAY)
    expect(onPlace.mock.calls[0][0]).toEqual([2, 3, 0])
  })
})

describe('SectionPlaneTool — widget offset-drag', () => {
  const PLANE: SectionPlane = { origin: [0, 0, 0], normal: [0, 0, 1], active: true }

  it('a click on the widget arms an offset drag instead of placing', () => {
    const { tool, onPlace } = makeTool({ currentPlane: PLANE })
    tool.onPointerDown(null, rayThroughZ(0))
    expect(onPlace).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)
  })

  it('moving past the threshold previews the offset plane live, in place', () => {
    const { tool, onOffsetPreview } = makeTool({ currentPlane: PLANE })
    tool.onPointerDown(null, rayThroughZ(0)) // grab at z=0
    tool.onPointerMove(null, rayThroughZ(2)) // cursor moves to z=2

    expect(onOffsetPreview).toHaveBeenCalledTimes(1)
    const preview = onOffsetPreview.mock.calls[0][0] as SectionPlane
    expect(preview.origin[2]).toBeCloseTo(2, 6)
    expect(preview.normal).toEqual([0, 0, 1])
  })

  it('a second click past the threshold commits the sweep (not a toggle)', () => {
    const { tool, onOffsetCommit, onToggle } = makeTool({ currentPlane: PLANE })
    tool.onPointerDown(null, rayThroughZ(0))
    tool.onPointerMove(null, rayThroughZ(2))
    tool.onPointerDown(null, rayThroughZ(2))

    expect(onToggle).not.toHaveBeenCalled()
    expect(onOffsetCommit).toHaveBeenCalledTimes(1)
    const committed = onOffsetCommit.mock.calls[0][0] as SectionPlane
    expect(committed.origin[2]).toBeCloseTo(2, 6)
  })

  it('a second click UNDER the threshold toggles active instead of committing an offset', () => {
    const { tool, onOffsetCommit, onToggle } = makeTool({ currentPlane: PLANE })
    tool.onPointerDown(null, rayThroughZ(0))
    // Released at (almost) the same spot — net movement well under threshold.
    tool.onPointerDown(null, rayThroughZ(OFFSET_DRAG_THRESHOLD_M / 10))

    expect(onOffsetCommit).not.toHaveBeenCalled()
    expect(onToggle).toHaveBeenCalledTimes(1)
  })

  it('exiting the drag (Esc) reverts via onCancelOffset and returns to idle', () => {
    const { tool, onCancelOffset, onOffsetCommit, onPlace } = makeTool({ currentPlane: PLANE })
    tool.onPointerDown(null, rayThroughZ(0))
    tool.onPointerMove(null, rayThroughZ(5))
    tool.onKey({ key: 'Escape' } as KeyboardEvent)

    expect(onCancelOffset).toHaveBeenCalledTimes(1)
    expect(onOffsetCommit).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)

    // Back to idle: a plain click well away from the (still-existing)
    // widget places again rather than resuming the drag.
    const farRay: Ray = { origin: [50, 50, 5], direction: [0, 0, -1] }
    tool.onPointerDown(makeSnap({ x: 50, y: 50, elementKind: 'face', object: 1n, element: 1n }), farRay)
    expect(onPlace).toHaveBeenCalledTimes(1)
  })

  it('switching tools mid-drag (cancel()) also reverts via onCancelOffset', () => {
    const { tool, onCancelOffset } = makeTool({ currentPlane: PLANE })
    tool.onPointerDown(null, rayThroughZ(0))
    tool.onPointerMove(null, rayThroughZ(5))
    tool.cancel()
    expect(onCancelOffset).toHaveBeenCalledTimes(1)
  })

  it('typing an exact offset and pressing Enter commits that value', () => {
    const { tool, onOffsetCommit } = makeTool({ currentPlane: PLANE })
    tool.onPointerDown(null, rayThroughZ(0))
    for (const ch of '3') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(onOffsetCommit).toHaveBeenCalledTimes(1)
    const committed = onOffsetCommit.mock.calls[0][0] as SectionPlane
    expect(committed.origin[2]).toBeCloseTo(3, 6)
  })

  it('a ray missing the widget rectangle falls through to placement', () => {
    const { tool, onPlace } = makeTool({ currentPlane: PLANE, widgetHalfExtent: 0.01 })
    // rayThroughZ(0) hits the plane at world (0,0,0) — comfortably inside a
    // half-extent of 10 but OUTSIDE a half-extent of 0.01 only if the hit
    // point itself is off-center; use a ray that hits well away from the
    // origin instead.
    const missRay: Ray = { origin: [50, 0, 3], direction: DIR }
    tool.onPointerDown(makeSnap({ x: 50, y: 0, z: 0, elementKind: 'face', object: 9n, element: 9n }), missRay)
    expect(onPlace).toHaveBeenCalledTimes(1)
  })

  it('a ray parallel to the plane (no intersection) falls through to placement', () => {
    const { tool, onPlace } = makeTool({ currentPlane: PLANE })
    const parallelRay: Ray = { origin: [0, 0, 5], direction: [1, 0, 0] } // ⊥ to normal [0,0,1]
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, elementKind: 'face', object: 9n, element: 9n }), parallelRay)
    expect(onPlace).toHaveBeenCalledTimes(1)
  })

  it('an armed offset whose SECOND click MISSES the widget re-places instead of offset-jumping', () => {
    // Widget half-extent 10, plane at origin. Arm on the widget (rayThroughZ(0)
    // hits the z=0 plane at x=3, inside the rect), then the second click lands
    // on a distant face at x=50 — well outside the widget rect at its
    // previewed position — so it abandons the offset and re-places, NOT
    // commits a surprise offset.
    const { tool, onPlace, onOffsetCommit, onCancelOffset } = makeTool({
      currentPlane: PLANE,
      widgetHalfExtent: 10,
    })
    tool.onPointerDown(null, rayThroughZ(0)) // arm — hits widget at (3,0,0)
    expect(tool.capturingInput()).toBe(true) // armed
    // Second click straight down onto a distant face at (50,0,0).
    const farFaceRay: Ray = { origin: [50, 0, 3], direction: [0, 0, -1] }
    tool.onPointerDown(
      makeSnap({ x: 50, y: 0, z: 0, elementKind: 'face', object: 9n, element: 9n }),
      farFaceRay,
    )
    expect(onOffsetCommit).not.toHaveBeenCalled()
    expect(onCancelOffset).toHaveBeenCalledTimes(1) // arm abandoned
    expect(onPlace).toHaveBeenCalledTimes(1) // re-placed on the face
    expect(onPlace.mock.calls[0][0]).toEqual([50, 0, 0])
    expect(tool.capturingInput()).toBe(false) // back to idle
  })
})

describe('SectionPlaneTool — delete', () => {
  it('Delete removes the section when idle and one exists', () => {
    const { tool, onDelete } = makeTool({ currentPlane: { origin: [0, 0, 0], normal: [0, 0, 1], active: true } })
    tool.onKey({ key: 'Delete' } as KeyboardEvent)
    expect(onDelete).toHaveBeenCalledTimes(1)
  })

  it('Backspace also removes the section when idle', () => {
    const { tool, onDelete } = makeTool({ currentPlane: { origin: [0, 0, 0], normal: [0, 0, 1], active: true } })
    tool.onKey({ key: 'Backspace' } as KeyboardEvent)
    expect(onDelete).toHaveBeenCalledTimes(1)
  })

  it('Delete is a no-op when no section exists', () => {
    const { tool, onDelete } = makeTool({ currentPlane: null })
    tool.onKey({ key: 'Delete' } as KeyboardEvent)
    expect(onDelete).not.toHaveBeenCalled()
  })

  it('Backspace mid-drag edits the VCB buffer instead of deleting', () => {
    const { tool, onDelete, onMeasurement } = makeTool({
      currentPlane: { origin: [0, 0, 0], normal: [0, 0, 1], active: true },
    })
    tool.onPointerDown(null, rayThroughZ(0))
    tool.onKey({ key: '5' } as KeyboardEvent)
    tool.onKey({ key: 'Backspace' } as KeyboardEvent)
    expect(onDelete).not.toHaveBeenCalled()
    expect(onMeasurement).toHaveBeenCalled()
  })
})

describe('SectionPlaneTool — capturingInput', () => {
  it('is false at idle and true only while offsetting', () => {
    const { tool } = makeTool({ currentPlane: { origin: [0, 0, 0], normal: [0, 0, 1], active: true } })
    expect(tool.capturingInput()).toBe(false)
    tool.onPointerDown(null, rayThroughZ(0))
    expect(tool.capturingInput()).toBe(true)
    tool.cancel()
    expect(tool.capturingInput()).toBe(false)
  })
})

describe('SectionPlaneTool — capturesKey (destructive-Delete guard)', () => {
  // The App-level Delete/Backspace handler backs off exactly when
  // capturesKey(key) is true, so this gating is what keeps Delete from ALSO
  // running a real kernel delete of the document selection while the section
  // tool removes its own section.
  it('captures Delete/Backspace at idle ONLY when a section exists to remove', () => {
    const withSection = makeTool({ currentPlane: { origin: [0, 0, 0], normal: [0, 0, 1], active: true } })
    expect(withSection.tool.capturesKey('Delete')).toBe(true)
    expect(withSection.tool.capturesKey('Backspace')).toBe(true)

    const noSection = makeTool({ currentPlane: null })
    // With nothing of its own to delete, Delete falls through to its normal
    // meaning (delete the document selection) — the tool must NOT capture it.
    expect(noSection.tool.capturesKey('Delete')).toBe(false)
    expect(noSection.tool.capturesKey('Backspace')).toBe(false)
  })

  it('does not capture unrelated keys at idle (tool-switch letters fall through)', () => {
    const { tool } = makeTool({ currentPlane: { origin: [0, 0, 0], normal: [0, 0, 1], active: true } })
    expect(tool.capturesKey('r')).toBe(false)
    expect(tool.capturesKey('m')).toBe(false)
  })

  it('captures the whole keyboard while offsetting (VCB eats letters/space)', () => {
    const { tool } = makeTool({ currentPlane: { origin: [0, 0, 0], normal: [0, 0, 1], active: true } })
    tool.onPointerDown(null, rayThroughZ(0)) // arm offsetting
    expect(tool.capturesKey('5')).toBe(true)
    expect(tool.capturesKey('m')).toBe(true)
    expect(tool.capturesKey(' ')).toBe(true)
    expect(tool.capturesKey('Delete')).toBe(true)
  })
})

describe('SectionPlaneTool — screen-constant preview-quad scaling', () => {
  // The preview quad's screen size at the app's reference fov/viewport (45°,
  // 720px tall) — carried over from the old PREVIEW_SCREEN_K = 0.06 constant
  // so the migration doesn't change how big the hover preview looks at that
  // baseline (same source K as SliceTool's PLANE_SCREEN_PX).
  const REF_FOV_DEG = 45
  const REF_VIEWPORT_H = 720
  const tanHalf = (fovDeg: number) => Math.tan((fovDeg * Math.PI) / 360)
  const expectedScale = (dist: number, fovDeg: number, viewportH: number) => {
    const desiredPixels = (0.06 * REF_VIEWPORT_H) / tanHalf(REF_FOV_DEG)
    return (desiredPixels * dist * tanHalf(fovDeg)) / viewportH
  }

  function hoverPreview(tool: SectionPlaneTool, preview: THREE.Group) {
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), rayThroughZ(0))
    return preview.children[0] as THREE.Group
  }

  it('updateDiskScale matches the old PREVIEW_SCREEN_K * dist size at the reference fov/viewport', () => {
    const { tool, preview } = makeTool()
    const quad = hoverPreview(tool, preview)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)
    tool.updateDiskScale(camera, REF_VIEWPORT_H)

    const dist = camera.position.distanceTo(quad.position)
    const expected = expectedScale(dist, REF_FOV_DEG, REF_VIEWPORT_H)
    expect(expected).toBeCloseTo(0.06 * dist, 9) // old K * dist, sanity cross-check
    expect(quad.scale.x).toBeCloseTo(expected, 9)
  })

  it('holds its on-screen size across a FOV change, unlike the old K * dist form', () => {
    const { tool, preview } = makeTool()
    const quad = hoverPreview(tool, preview)

    for (const fov of [20, 45, 70, 100]) {
      const camera = new THREE.PerspectiveCamera(fov)
      camera.position.set(0, 0, 10)
      tool.updateDiskScale(camera, REF_VIEWPORT_H)
      const dist = camera.position.distanceTo(quad.position)
      expect(quad.scale.x).toBeCloseTo(expectedScale(dist, fov, REF_VIEWPORT_H), 9)
    }
  })

  it('holds its on-screen size across a viewport resize, unlike the old K * dist form', () => {
    const { tool, preview } = makeTool()
    const quad = hoverPreview(tool, preview)
    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)

    for (const viewportH of [400, 720, 1200]) {
      tool.updateDiskScale(camera, viewportH)
      const dist = camera.position.distanceTo(quad.position)
      expect(quad.scale.x).toBeCloseTo(expectedScale(dist, REF_FOV_DEG, viewportH), 9)
    }
  })

  it('is a no-op when no preview is shown, for a non-perspective camera, or a degenerate viewport height', () => {
    const { tool } = makeTool()
    expect(() => tool.updateDiskScale(new THREE.PerspectiveCamera(REF_FOV_DEG), REF_VIEWPORT_H)).not.toThrow()

    const { tool: tool2, preview } = makeTool()
    const quad = hoverPreview(tool2, preview)
    const before = quad.scale.x

    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 10)
    ortho.position.set(0, 0, 10)
    tool2.updateDiskScale(ortho, REF_VIEWPORT_H)
    expect(quad.scale.x).toBe(before)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)
    tool2.updateDiskScale(camera, 0)
    expect(quad.scale.x).toBe(before)
  })
})
