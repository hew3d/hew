/**
 * ProtractorTool — sketches on any plane (Phase 4, the sketch-planes design
 * §6 bullet 2): hover-adopting a non-ground sketch's plane before the apex
 * is placed. No idle arrow-key plane lock here — Protractor already owns
 * all four arrows for its own plane lock (see the module doc / Phase 4
 * report for the noted conflict).
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ProtractorTool } from './ProtractorTool'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** A tilted sketch: the y=0 plane, normal -Y (a vertical "wall" sketch). */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, -1, 0])
const GROUND_SKETCH = 9n
const GROUND_PLANE = new Float64Array([0, 0, 0, 0, 0, 1])

function makeWasmScene(opts: { sketchPick?: bigint } = {}): WasmScene {
  const planes = new Map<bigint, Float64Array>([
    [TILTED_SKETCH, TILTED_PLANE],
    [GROUND_SKETCH, GROUND_PLANE],
  ])
  return {
    face_normal: vi.fn(() => { throw new Error('not a live world-object face') }),
    add_guide_line: vi.fn(() => 1n),
    pick_sketch: vi.fn(() => opts.sketchPick),
    sketch_plane: vi.fn((h: bigint) => planes.get(h)),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const tool = new ProtractorTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn())
  return { tool }
}

describe('ProtractorTool — hover-adopt a non-ground sketch plane', () => {
  it('idle snapConstraint returns the hovered non-ground sketch plane', () => {
    const scene = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    expect(tool.snapConstraint(RAY)).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] },
    })
  })

  it('null with no sketch under the cursor', () => {
    const scene = makeWasmScene({ sketchPick: undefined })
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('null when the hovered sketch IS on the ground plane (today\'s unconstrained behavior)', () => {
    const scene = makeWasmScene({ sketchPick: GROUND_SKETCH })
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('null once the apex is placed — Protractor projects onto its own plane analytically past the first pick', () => {
    const scene = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerMove({ x: 1, y: 0, z: 2, kind: 'plane' }, RAY)
    tool.onPointerDown({ x: 1, y: 0, z: 2, kind: 'plane' }, RAY) // apex placed

    expect(tool.snapConstraint(RAY)).toBeNull()
  })
})

describe('ProtractorTool — apex adopts the hovered sketch plane (Blocker 1)', () => {
  /**
   * Mirrors the real call order (Viewport.tsx): `snapConstraint(ray)` runs
   * BEFORE `onPointerMove`/`onPointerDown` for every pointer event, so the
   * apex's `planeNormal` must come from the SAME hover-adopted plane
   * `snapConstraint()` just resolved, not fall back to world up because a
   * sketch hover carries no `elementKind === 'face'`.
   */
  it('places the apex with planeNormal from the hovered tilted sketch, not [0,0,1]', () => {
    const scene = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.snapConstraint(RAY) // hover-adopts TILTED_SKETCH's plane (normal [0,-1,0])
    tool.onPointerMove({ x: 1, y: 0, z: 2, kind: 'plane' }, RAY)
    tool.snapConstraint(RAY) // Viewport re-resolves the constraint right before onPointerDown too
    tool.onPointerDown({ x: 1, y: 0, z: 2, kind: 'plane' }, RAY) // apex placed

    const stage = (tool as unknown as {
      stage: { kind: string; planeNormal: [number, number, number] }
    }).stage
    expect(stage.kind).toBe('awaiting-baseline')
    expect(stage.planeNormal).toEqual([0, -1, 0])
  })

  it('still defaults to world up when nothing is hovered', () => {
    const scene = makeWasmScene({ sketchPick: undefined })
    const { tool } = makeTool(scene)

    tool.snapConstraint(RAY)
    tool.onPointerMove({ x: 1, y: 2, z: 0, kind: 'ground' }, RAY)
    tool.snapConstraint(RAY)
    tool.onPointerDown({ x: 1, y: 2, z: 0, kind: 'ground' }, RAY)

    const stage = (tool as unknown as {
      stage: { kind: string; planeNormal: [number, number, number] }
    }).stage
    expect(stage.kind).toBe('awaiting-baseline')
    expect(stage.planeNormal).toEqual([0, 0, 1])
  })
})
