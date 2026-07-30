/**
 * TextTool unit tests — click-drag-click leader placement. The tool itself
 * never calls into the kernel (text entry is the in-viewport editor's job,
 * wired by Viewport.tsx) — these assert the `onPlaceLeader` handoff shape
 * and the gesture stages (Escape cancels mid-drag).
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { TextTool, type PlacedLeader } from './TextTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

function makeTool() {
  const onPlace = vi.fn<(leader: PlacedLeader) => void>()
  const tool = new TextTool(new THREE.Group(), onPlace)
  return { tool, onPlace }
}

describe('TextTool — leader placement gesture', () => {
  it('click anchors, drag sets the offset, second click hands off to onPlaceLeader', () => {
    const { tool, onPlace } = makeTool()

    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0, kind: 'on-face', object: 9n, element: 4n }), RAY)
    expect(tool.statusHint()).toMatch(/drag/i)

    tool.onPointerMove(makeSnap({ x: 1.5, y: 2.5, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2.5, z: 0 }), RAY)

    expect(onPlace).toHaveBeenCalledTimes(1)
    const leader = onPlace.mock.calls[0][0]
    expect(leader.anchorNode).toEqual({ kind: 0, id: 9n })
    expect(leader.anchorPoint).toEqual([1, 2, 0])
    expect(leader.offset[0]).toBeCloseTo(0.5, 9)
    expect(leader.offset[1]).toBeCloseTo(0.5, 9)

    // Tool returns to idle after handing off.
    expect(tool.statusHint()).toMatch(/anchor the leader/i)
  })

  it('a free-space anchor (no object/instance) encodes as null', () => {
    const { tool, onPlace } = makeTool()

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind: 'ground' }), RAY)

    expect(onPlace.mock.calls[0][0].anchorNode).toBeNull()
  })

  it('an instance-placed anchor encodes as kind 2 (Instance)', () => {
    const { tool, onPlace } = makeTool()

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'on-face', object: 3n, instance: 55n }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0, kind: 'ground' }), RAY)

    expect(onPlace.mock.calls[0][0].anchorNode).toEqual({ kind: 2, id: 55n })
  })

  it('Escape cancels a drag in progress without ever calling onPlaceLeader', () => {
    const { tool, onPlace } = makeTool()

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onKey(makeKeyEvent('Escape'))
    // A subsequent click starts a brand-new gesture (idle again).
    expect(tool.statusHint()).toMatch(/anchor the leader/i)
    tool.onPointerDown(makeSnap({ x: 5, y: 5, z: 0, kind: 'ground' }), RAY)
    expect(onPlace).not.toHaveBeenCalled()
    expect(tool.statusHint()).toMatch(/drag/i)
  })

  it('a pointer move before the first click is a no-op (idle)', () => {
    const { tool, onPlace } = makeTool()
    tool.onPointerMove(makeSnap({ x: 9, y: 9, z: 9 }), RAY)
    expect(onPlace).not.toHaveBeenCalled()
  })
})

describe('TextTool — camera-facing offset plane (dimensions-playtest2.md §3)', () => {
  // RED-CHECK: the unfixed code computes `offset` directly from the
  // resolved snap point with no camera involvement at all — `updateCamera`
  // doesn't exist yet, so these tests fail to even compile/construct
  // meaningfully different behavior against it; conceptually, a cursor ray
  // aimed at a point far from the anchor's own depth (see the test below)
  // would offset the leader by that far point's FULL 3D displacement
  // instead of landing it at the anchor's depth in the ray's direction.

  function perspCamera(pos: [number, number, number], target: [number, number, number]): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    cam.position.set(...pos)
    cam.lookAt(...target)
    cam.updateMatrixWorld(true)
    return cam
  }

  it('without updateCamera, behaves exactly as before (plain cursor - anchor)', () => {
    const { tool, onPlace } = makeTool()
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0, kind: 'on-face', object: 9n, element: 4n }), RAY)
    tool.onPointerMove(makeSnap({ x: 1.5, y: 2.5, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2.5, z: 0 }), RAY)
    const leader = onPlace.mock.calls[0][0]
    expect(leader.offset[0]).toBeCloseTo(0.5, 9)
    expect(leader.offset[1]).toBeCloseTo(0.5, 9)
  })

  it('with a registered camera, the offset lands the leader at the ANCHOR depth along the cursor ray, not the raw snap point', () => {
    const { tool, onPlace } = makeTool()
    // Camera looking straight down the -Z axis from above; anchor sits at
    // z=3 (well above the ground the raw snap below reports).
    const camera = perspCamera([0, 0, 10], [0, 0, 0])
    tool.updateCamera(camera)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'on-face', object: 9n, element: 4n }), RAY)
    // The raw resolved snap point is on the GROUND (z=0) at (2,1,0) — the
    // exact "collapses to the ground" symptom findings 4 targets. A ray
    // straight down through that (x,y) column, from the camera at z=10,
    // hits the anchor's own camera-facing plane (z=3) at the SAME (x,y).
    const ray: Ray = { origin: [2, 1, 10], direction: [0, 0, -1] }
    tool.onPointerMove(makeSnap({ x: 2, y: 1, z: 0 }), ray)
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 0 }), ray)

    const leader = onPlace.mock.calls[0][0]
    // Offset lands the leader at (2,1,3) -> offset (2,1,0) from the anchor
    // (0,0,3) — NOT (2,1,-3), which is what a raw ground-plane snap point
    // minus the anchor would have produced.
    expect(leader.offset[0]).toBeCloseTo(2, 9)
    expect(leader.offset[1]).toBeCloseTo(1, 9)
    expect(leader.offset[2]).toBeCloseTo(0, 9)
  })

  it('falls back to the plain snap point when the ray is parallel to the camera-facing plane', () => {
    const { tool, onPlace } = makeTool()
    const camera = perspCamera([0, 0, 10], [0, 0, 0]) // view dir = -Z
    tool.updateCamera(camera)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'on-face', object: 9n, element: 4n }), RAY)
    // A ray running perpendicular to the view direction never crosses the
    // (z=3) camera-facing plane.
    const parallelRay: Ray = { origin: [0, 0, 5], direction: [1, 0, 0] }
    tool.onPointerMove(makeSnap({ x: 4, y: 4, z: 4 }), parallelRay)
    tool.onPointerDown(makeSnap({ x: 4, y: 4, z: 4 }), parallelRay)
    const leader = onPlace.mock.calls[0][0]
    expect(leader.offset).toEqual([4, 4, 1])
  })
})
