import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { SliceTool } from './SliceTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** Minimal WasmScene stub — only the members SliceTool calls. No face under
 * the ray and no object under the cursor, so the plane follows the locked
 * (default Z) axis and no commit is attempted. */
function makeWasmScene(): WasmScene {
  return {
    face_plane: vi.fn(() => {
      throw new Error('not a live world-object face')
    }),
    pick_face: vi.fn(() => undefined),
  } as unknown as WasmScene
}

function makeTool() {
  const preview = new THREE.Group()
  const onSliceCommitted = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const wasmScene = makeWasmScene()
  const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast, onMeasurement)
  return { tool, preview, onSliceCommitted, onToast, onMeasurement, wasmScene }
}

describe('SliceTool — screen-constant preview-plane scaling', () => {
  // The preview plane's screen size at the app's reference fov/viewport (45°,
  // 720px tall) — carried over from the old PLANE_SCREEN_K = 0.06 constant so
  // the migration doesn't change how big the cut-plane preview looks at that
  // baseline.
  const REF_FOV_DEG = 45
  const REF_VIEWPORT_H = 720
  const tanHalf = (fovDeg: number) => Math.tan((fovDeg * Math.PI) / 360)
  const expectedScale = (dist: number, fovDeg: number, viewportH: number) => {
    const desiredPixels = (0.06 * REF_VIEWPORT_H) / tanHalf(REF_FOV_DEG)
    return (desiredPixels * dist * tanHalf(fovDeg)) / viewportH
  }

  function hoverPreview(tool: SliceTool, preview: THREE.Group) {
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY)
    return preview.children[0] as THREE.Group
  }

  it('shows a preview plane on hover, centered at the snap point', () => {
    const { tool, preview } = makeTool()
    const quad = hoverPreview(tool, preview)
    expect(quad.position.toArray()).toEqual([0, 0, 0])
  })

  it('updateDiskScale matches the old PLANE_SCREEN_K * dist size at the reference fov/viewport', () => {
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

function makeFacePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    free: vi.fn(),
  }
}

describe('SliceTool — commit routing', () => {
  it('a plain world object slices via slice_object', () => {
    const preview = new THREE.Group()
    const onSliceCommitted = vi.fn()
    const onToast = vi.fn()
    const wasmScene = {
      face_plane: vi.fn(() => { throw new Error('not a face') }),
      pick_face: vi.fn(() => makeFacePick(3n, 4n)),
      slice_object: vi.fn(() => new BigUint64Array([10n, 11n])),
    } as unknown as WasmScene
    const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(wasmScene.slice_object).toHaveBeenCalledTimes(1)
    expect((wasmScene.slice_object as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(3n)
    expect(onSliceCommitted).toHaveBeenCalledWith(10n)
  })

  it('an instanced object is out of scope at the top level (unchanged)', () => {
    const preview = new THREE.Group()
    const onSliceCommitted = vi.fn()
    const onToast = vi.fn()
    const wasmScene = {
      face_plane: vi.fn(() => { throw new Error('not a face') }),
      pick_face: vi.fn(() => makeFacePick(3n, 4n, 99n)),
      slice_object: vi.fn(),
      slice_def_member: vi.fn(),
    } as unknown as WasmScene
    const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(wasmScene.slice_object).not.toHaveBeenCalled()
    expect(wasmScene.slice_def_member).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onSliceCommitted).not.toHaveBeenCalled()
  })

  // component-edit-parity.md phase A2.
  describe('inside a component instance context', () => {
    const INSTANCE = 42n
    const COMPONENT = 5n

    it('a member face slices via slice_def_member, never the world slice_object', () => {
      const preview = new THREE.Group()
      const onSliceCommitted = vi.fn()
      const onToast = vi.fn()
      const wasmScene = {
        face_plane: vi.fn(() => { throw new Error('not a face') }),
        pick_face: vi.fn(() => makeFacePick(3n, 4n, INSTANCE)),
        slice_object: vi.fn(),
        slice_def_member: vi.fn(() => new BigUint64Array([10n, 11n])),
      } as unknown as WasmScene
      const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast)
      tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

      expect(wasmScene.slice_def_member).toHaveBeenCalledTimes(1)
      const call = (wasmScene.slice_def_member as ReturnType<typeof vi.fn>).mock.calls[0]
      expect(call[0]).toBe(INSTANCE)
      expect(call[1]).toBe(3n)
      expect(wasmScene.slice_object).not.toHaveBeenCalled()
      expect(onSliceCommitted).toHaveBeenCalledWith(10n)
    })

    it('a face outside the entered instance is out of scope', () => {
      const preview = new THREE.Group()
      const onSliceCommitted = vi.fn()
      const onToast = vi.fn()
      const wasmScene = {
        face_plane: vi.fn(() => { throw new Error('not a face') }),
        pick_face: vi.fn(() => makeFacePick(3n, 4n, 99n)), // a different instance
        slice_object: vi.fn(),
        slice_def_member: vi.fn(),
      } as unknown as WasmScene
      const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast)
      tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

      expect(wasmScene.slice_object).not.toHaveBeenCalled()
      expect(wasmScene.slice_def_member).not.toHaveBeenCalled()
      expect(onSliceCommitted).not.toHaveBeenCalled()
    })

    it('a plain WORLD object is out of scope while editing an instance', () => {
      const preview = new THREE.Group()
      const onSliceCommitted = vi.fn()
      const onToast = vi.fn()
      const wasmScene = {
        face_plane: vi.fn(() => { throw new Error('not a face') }),
        pick_face: vi.fn(() => makeFacePick(3n, 4n, undefined)), // plain world object
        slice_object: vi.fn(),
        slice_def_member: vi.fn(),
      } as unknown as WasmScene
      const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast)
      tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

      expect(wasmScene.slice_object).not.toHaveBeenCalled()
      expect(wasmScene.slice_def_member).not.toHaveBeenCalled()
    })
  })
})

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

describe('SliceTool — hasArmedGesture (distinct from the unconditional capturingInput, component-edit-parity.md phase A2)', () => {
  function makeTool() {
    const preview = new THREE.Group()
    const onSliceCommitted = vi.fn()
    const onToast = vi.fn()
    const wasmScene = {
      face_plane: vi.fn(() => { throw new Error('not a face') }),
      pick_face: vi.fn(() => undefined),
      slice_object: vi.fn(),
    } as unknown as WasmScene
    const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast)
    return { tool }
  }

  it('capturingInput() is unconditionally true even with no lock engaged', () => {
    const { tool } = makeTool()
    expect(tool.capturingInput()).toBe(true)
    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('hasArmedGesture() becomes true only once a plane lock is engaged (Shift), and false again once lifted', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // establish a hover normal
    tool.onKey(makeKeyEvent('Shift')) // engage the lock
    expect(tool.hasArmedGesture()).toBe(true)

    tool.onKey(makeKeyEvent('Escape')) // first Escape lifts the lock, per the tool's own semantics
    expect(tool.hasArmedGesture()).toBe(false)
  })
})

describe('SliceTool — setEditContext aborts an armed gesture on a genuine change (component-edit-parity.md phase A2)', () => {
  function makeTool() {
    const preview = new THREE.Group()
    const onSliceCommitted = vi.fn()
    const onToast = vi.fn()
    const wasmScene = {
      face_plane: vi.fn(() => { throw new Error('not a face') }),
      pick_face: vi.fn(() => undefined),
      slice_object: vi.fn(),
    } as unknown as WasmScene
    const tool = new SliceTool(wasmScene, preview, onSliceCommitted, onToast)
    return { tool }
  }

  it('a genuine context change drops an engaged plane lock instead of silently carrying a stale-frame normal into the new context', () => {
    const { tool } = makeTool()
    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('Shift'))
    expect(tool.hasArmedGesture()).toBe(true)

    tool.setEditContext({ kind: 'top' })

    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('re-pushing the SAME context is a no-op — an engaged lock survives it untouched', () => {
    const { tool } = makeTool()
    const ctx = { kind: 'instance' as const, id: 9n, component: 90n }
    tool.setEditContext(ctx)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('Shift'))
    expect(tool.hasArmedGesture()).toBe(true)

    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })

    expect(tool.hasArmedGesture()).toBe(true)
  })
})
