/**
 * Soft axis inference — the preview visual (tool-parity playtest2 §2c):
 * "the line turns green [...] and locks in on that axis" is a preview
 * feature, not a resolution one — the kernel already decides WHETHER/what
 * to snap (crates/inference); this file is entirely about how `LineTool`
 * PAINTS the result. `snap.direction` is set whenever the kernel's answer
 * is axis-relevant — the kernel's own soft-axis inference (no lock held)
 * OR an active `lockAxis` hold — and both cases route through the same
 * `_previewStyle` helper, mirroring ProtractorTool.test.ts's established
 * "compute the expected color via the same axisColorForDirection call,
 * don't hardcode a hex" pattern so this file doesn't depend on which theme
 * happens to resolve in the vitest environment.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import type { DrawingAxes } from './drawingAxes'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** Matches `LINE_AXIS_PREVIEW_TOL_DOT` in LineTool.ts (10 degrees). */
const TOL = Math.cos((10 * Math.PI) / 180)

const WORLD_FRAME_FLAT = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]
const MOVED_FRAME_FLAT = [5, 0, 0, 0, 1, 0, -1, 0, 0, 0, 0, 1] // red = world +Y

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'plane', ...overrides }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

function makeWasmScene(frame: number[] = WORLD_FRAME_FLAT) {
  const planes = new Map<bigint, Float64Array>()
  let sketchCounter = 90n
  const scene = {
    axes: vi.fn(() => new Float64Array(frame)),
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      planes.set(sketchCounter, new Float64Array([0, 0, 0, 0, 0, 1]))
      return sketchCounter
    }),
    begin_sketch_on_plane: vi.fn(
      (px: number, py: number, pz: number, nx: number, ny: number, nz: number) => {
        sketchCounter += 1n
        planes.set(sketchCounter, new Float64Array([px, py, pz, nx, ny, nz]))
        return sketchCounter
      },
    ),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn((h: bigint) => planes.get(h)),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_add_segment: vi.fn(() => ({
      new_edges: () => new BigUint64Array([]),
      regions_created: () => new BigUint64Array([]),
      regions_removed: () => new BigUint64Array([]),
      free: vi.fn(),
    })),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return scene as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const tool = new LineTool(scene, preview, vi.fn(), vi.fn(), vi.fn(), vi.fn())
  return { tool, preview }
}

/** The single rubber-band preview segment, or undefined if none is drawn. */
function previewLine(preview: THREE.Group): LineSegments2 | undefined {
  return preview.children.find((c) => c instanceof LineSegments2) as LineSegments2 | undefined
}

/** The color LineTool's own preview path would assign a direction, computed
 *  the same way ProtractorTool.test.ts does — not hardcoded, so this file
 *  doesn't depend on which theme resolves in the vitest environment. */
function expectedAxisColorHex(dir: [number, number, number], frame?: DrawingAxes): number {
  const match = axisColorForDirection(dir, TOL, axisColorsForTheme(getResolvedTheme()), frame)
  expect(match, `${dir} should map to an axis color`).not.toBeNull()
  return new THREE.LineBasicMaterial({ color: match!.color }).color.getHex()
}

describe('LineTool — soft/hard axis preview colour and weight (design §2c)', () => {
  it('a free (non-axis) rubber band stays the default blue, default width', () => {
    const { tool, preview } = makeTool(makeWasmScene())
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerMove(makeSnap({ x: 3, y: 5, z: 0 }), RAY) // no `direction` on this snap
    const line = previewLine(preview)
    expect(line).toBeDefined()
    expect(line!.material.color.getHex()).toBe(
      new THREE.LineBasicMaterial({ color: PREVIEW_LINE_STYLE.color }).color.getHex(),
    )
    expect(line!.material.linewidth).toBe(PREVIEW_LINE_STYLE.widthPx)
  })

  it('a direction that reads as no particular axis (45 degrees off) also stays the default blue', () => {
    const { tool, preview } = makeTool(makeWasmScene())
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    // A diagonal direction, comfortably outside the 10-degree label tolerance.
    const diagonal: [number, number, number] = [Math.SQRT1_2, Math.SQRT1_2, 0]
    tool.onPointerMove(makeSnap({ x: 3, y: 3, z: 0, direction: diagonal }), RAY)
    const line = previewLine(preview)
    expect(line!.material.color.getHex()).toBe(
      new THREE.LineBasicMaterial({ color: PREVIEW_LINE_STYLE.color }).color.getHex(),
    )
  })

  it('the KERNEL\'s own soft-axis inference (no lockAxis held) colours the preview but keeps the default width', () => {
    const { tool, preview } = makeTool(makeWasmScene())
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    // A soft snap: kind on-axis, direction set, but the TOOL's own lockAxis
    // is null (no arrow key was pressed) — exactly what the kernel returns
    // for its own soft-axis candidate.
    tool.onPointerMove(makeSnap({ x: 5, y: 0.3, z: 0, kind: 'on-axis', direction: [1, 0, 0] }), RAY)
    const line = previewLine(preview)
    expect(line!.material.color.getHex()).toBe(expectedAxisColorHex([1, 0, 0]))
    expect(line!.material.linewidth).toBe(PREVIEW_LINE_STYLE.widthPx) // NOT bold — soft, not hard
  })

  it('an ACTIVE lockAxis hold colours the preview AND makes it bold — soft and hard read differently', () => {
    const { tool, preview } = makeTool(makeWasmScene())
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('ArrowRight')) // mid-chain lockAxis: red/X
    // The resolved snap while locked: direction carries the lock's axis.
    tool.onPointerMove(makeSnap({ x: 5, y: 0, z: 0, direction: [1, 0, 0] }), RAY)
    const line = previewLine(preview)
    expect(line!.material.color.getHex()).toBe(expectedAxisColorHex([1, 0, 0]))
    expect(line!.material.linewidth).toBeGreaterThan(PREVIEW_LINE_STYLE.widthPx) // bold
  })

  it('the preview follows a MOVED drawing-axes frame, not literal world axes', () => {
    const scene = makeWasmScene(MOVED_FRAME_FLAT) // red = world +Y
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    // The kernel reports the direction it actually resolved: world +Y (the
    // moved frame's red axis).
    tool.onPointerMove(makeSnap({ x: 0, y: 5, z: 0, kind: 'on-axis', direction: [0, 1, 0] }), RAY)
    const line = previewLine(preview)
    const frame: DrawingAxes = {
      origin: [5, 0, 0],
      x: [0, 1, 0],
      y: [-1, 0, 0],
      z: [0, 0, 1],
    }
    expect(line!.material.color.getHex()).toBe(expectedAxisColorHex([0, 1, 0], frame))
    // Under the moved frame, world +Y IS red — not the world-identity
    // green a frame-blind implementation would wrongly assign.
    expect(line!.material.color.getHex()).not.toBe(expectedAxisColorHex([0, 1, 0], undefined))
  })

  it('face-mode chains ALSO colour their rubber band by axis direction', () => {
    const scene = makeWasmScene()
    ;(scene as unknown as { pick_face: ReturnType<typeof vi.fn> }).pick_face = vi.fn(() => ({
      object: () => 1n,
      face: () => 2n,
      instance: () => undefined,
      free: vi.fn(),
    }))
    ;(scene as unknown as { face_normal: ReturnType<typeof vi.fn> }).face_normal = vi.fn(
      () => new Float64Array([0, 0, 1]),
    )
    ;(scene as unknown as { node_parent: ReturnType<typeof vi.fn> }).node_parent = vi.fn(() => undefined)
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), RAY) // face-mode anchor
    tool.onPointerMove(makeSnap({ x: 5, y: 0, z: 1, kind: 'on-axis', direction: [1, 0, 0] }), RAY)
    const line = previewLine(preview)
    expect(line!.material.color.getHex()).toBe(expectedAxisColorHex([1, 0, 0]))
  })
})
