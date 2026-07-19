/**
 * TapeMeasureTool — sketches on any plane (Phase 4, the sketch-planes design
 * §6 bullet 2): hover-adopting a non-ground sketch's plane, and the idle
 * arrow-key plane lock, both freeze `snapConstraint()`'s plane for the whole
 * gesture so a guide/measurement started on a tilted sketch stays in that
 * plane instead of resolving to the ground fallback and refusing.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { TapeMeasureTool } from './TapeMeasureTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'plane', ...overrides }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

/** A tilted sketch: the y=0 plane, normal -Y (a vertical "wall" sketch). */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, -1, 0])

function makeWasmScene(opts: { sketchPick?: bigint } = {}) {
  const guidePoints: number[][] = []
  const planes = new Map<bigint, Float64Array>([[TILTED_SKETCH, TILTED_PLANE]])
  const scene = {
    edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 2, 0, 0])),
    sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 2, 0, 0])),
    add_guide_line: vi.fn(),
    add_guide_point: vi.fn((x: number, y: number, z: number) => { guidePoints.push([x, y, z]) }),
    pick_sketch: vi.fn(() => opts.sketchPick),
    sketch_plane: vi.fn((h: bigint) => planes.get(h)),
  }
  return { scene: scene as unknown as WasmScene, guidePoints }
}

function makeTool(scene: WasmScene) {
  const onGuideCreated = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new TapeMeasureTool(scene, new THREE.Group(), onGuideCreated, onToast, onMeasurement)
  return { tool, onGuideCreated, onToast, onMeasurement }
}

describe('TapeMeasureTool — hover-adopt a non-ground sketch plane', () => {
  it('idle snapConstraint returns the hovered sketch plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    const constraint = tool.snapConstraint(RAY)
    expect(constraint).toEqual({ constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] } })
  })

  it('idle snapConstraint is null with no sketch under the cursor', () => {
    const { scene } = makeWasmScene({ sketchPick: undefined })
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('measuring between two points on the hovered sketch stays constrained to its plane through the second click', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    // First click over the tilted sketch (kind: 'plane' — a Phase 1 fallback
    // snap landing on the constraint plane still counts as "on the sketch",
    // not on-geometry, per snapOnGeometry's kind check).
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2, kind: 'plane' }), RAY)
    expect(tool.capturingInput()).toBe(true)

    // Mid-gesture: snapConstraint now returns the FROZEN plane regardless of
    // what's currently under the cursor.
    expect(tool.snapConstraint(RAY)).toEqual({ constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] } })

    tool.onPointerMove(makeSnap({ x: 3, y: 0, z: 5, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5, kind: 'plane' }), RAY) // second click commits

    // Both endpoints landed via the 'plane' fallback (not on real geometry)
    // — SketchUp/snapOnGeometry semantics: a guide POINT is dropped at the
    // second (measure-mode) endpoint, per _commitMeasure's onGeometry check.
    expect(scene.add_guide_point).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false) // gesture ended
  })

  it('the frozen plane clears once the gesture ends — the NEXT gesture re-resolves it', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5, kind: 'plane' }), RAY) // commit, back to idle

    // Idle again: snapConstraint re-derives from the CURRENT hover, not a
    // stale frozen plane.
    expect(tool.snapConstraint(RAY)).toEqual({ constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] } })
  })
})

describe('TapeMeasureTool — idle arrow-key plane lock (design §6 bullet 2)', () => {
  it('an arrow key locks the plane while idle, named in statusHint', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    expect(tool.statusHint()).not.toContain('Locked')

    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.statusHint()).toContain('Locked to the red plane')
  })

  it('pressing the same arrow again clears the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.statusHint()).not.toContain('Locked')
  })

  it('idle snapConstraint is FREE (unconstrained) while locked — the plane derives from the click', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.snapConstraint(RAY)).toBeNull()
    expect(scene.pick_sketch).not.toHaveBeenCalled() // lock beats sketch-hover adoption
  })

  it('the first click freezes the locked axis plane through the clicked point', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // red/X lock
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3 }), RAY)

    expect(tool.snapConstraint(RAY)).toEqual({ constraintPlane: { point: [2, 1, 3], normal: [1, 0, 0] } })
  })

  it('Escape while idle-locked clears the lock first; a second Escape does not throw', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    expect(tool.statusHint()).toContain('Locked')
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.statusHint()).not.toContain('Locked')
    expect(() => tool.onKey(makeKeyEvent('Escape'))).not.toThrow()
  })

  it('Escape aborting an anchored-but-uncommitted gesture preserves the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3 }), RAY) // anchor only
    expect(tool.capturingInput()).toBe(true)
    tool.onKey(makeKeyEvent('Escape')) // abort the gesture, keep the aim
    expect(tool.capturingInput()).toBe(false)
    expect(tool.statusHint()).toContain('Locked to the red plane')
  })

  it('cancel() clears the lock', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.cancel()
    expect(tool.statusHint()).not.toContain('Locked')
  })

  it('the lock survives a completed gesture', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 3 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 4, z: 6 }), RAY) // commit (measure, drops a guide point)

    expect(tool.statusHint()).toContain('Locked to the red plane')
  })
})

describe('TapeMeasureTool — parallel-guide vs. frozen plane, the edge wins (Blocker 2)', () => {
  it('edge IN the hover-adopted tilted-sketch plane: constraint kept, committed origin stays on-plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    // The default edge_endpoints mock is [0,0,0, 2,0,0] — direction [1,0,0],
    // which IS perpendicular to TILTED_SKETCH's normal [0,-1,0], and the
    // picked point (y=0) sits ON that plane too — the hover-adopted case.
    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    expect(tool.capturingInput()).toBe(true)

    // Constraint survives — the edge agrees with the frozen plane.
    expect(tool.snapConstraint(RAY)).toEqual({ constraintPlane: { point: [0, 0, 0], normal: [0, -1, 0] } })

    // Drag the cursor to another point that itself stays on the plane
    // (y=0) — the resulting guide origin must too.
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 3, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 3, kind: 'plane' }), RAY) // commit

    const wasm = scene as unknown as { add_guide_line: ReturnType<typeof vi.fn> }
    expect(wasm.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = wasm.add_guide_line.mock.calls[0] as number[]
    expect(oy).toBe(0) // origin stayed on the y=0 plane
    expect([ox, oz]).toEqual([1, 3])
    expect([dx, dy, dz]).toEqual([1, 0, 0]) // parallel to the source edge
  })

  it('idle-lock plane + edge NOT in that plane: the edge wins, gesture is unconstrained (legacy behavior)', () => {
    const { scene } = makeWasmScene() // no sketch under the cursor
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // idle-locks the red/X plane (normal [1,0,0])

    // The default edge_endpoints mock is [0,0,0, 2,0,0] — direction [1,0,0],
    // which is NOT perpendicular to the locked plane's normal [1,0,0]
    // (dot = 1) — the edge disagrees with the idle lock.
    tool.onPointerDown(
      makeSnap({ x: 2, y: 1, z: 3, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    expect(tool.capturingInput()).toBe(true)

    // The edge wins: the frozen plane is dropped for this gesture — no
    // constraintPlane at all (legacy unconstrained parallel-guide behavior).
    expect(tool.snapConstraint(RAY)).toBeNull()

    // Legacy offset behavior still works normally: perpComponent of the
    // cursor relative to the edge direction, unconstrained by any plane.
    tool.onPointerMove(makeSnap({ x: 2, y: 5, z: 3, kind: 'plane' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 5, z: 3, kind: 'plane' }), RAY) // commit

    const wasm = scene as unknown as { add_guide_line: ReturnType<typeof vi.fn> }
    expect(wasm.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = wasm.add_guide_line.mock.calls[0] as number[]
    // edgePoint (2,1,3) + perp((cursor-edgePoint), edgeDir=[1,0,0]) = (2,1,3) + (0,4,0) = (2,5,3)
    expect([ox, oy, oz]).toEqual([2, 5, 3])
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })
})
