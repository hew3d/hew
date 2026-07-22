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
