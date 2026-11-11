import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ScaleTool } from './ScaleTool'
import { DragCommitGate } from '../viewport/dragCommit'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { ctxGround, draftRef } from './planeUnits'
import { worldPerPixelPerspective, worldPerPixelOrtho } from '../viewport/math'
import { SiblingPreviewMirror } from '../viewport/siblingPreview'
import { isSessionGhost } from './transformPreview'

/** `updateGripScale` now takes a `worldPerPixel` callback (the CameraRig
 * form, docs/design/camera.md §1) instead of a raw `(camera, viewportH)`
 * pair — this is what a real Viewport would build from `rig.worldPerPixel`
 * for a perspective camera at the given fov/viewport. */
const wppPerspective = (fovDeg: number, viewportH: number) => (dist: number) =>
  worldPerPixelPerspective(dist, fovDeg, viewportH)

function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

/**
 * A ray that hits `target` EXACTLY (perpendicular distance 0), approached
 * from a generic oblique direction so it doesn't also graze another grip on
 * the same box — a straight-down or straight-across ray would pass through
 * BOTH grips of an axis-aligned pair (e.g. the +Z and -Z face grips share
 * the same x/y), which is genuinely ambiguous for picking.
 */
function rayAt(target: [number, number, number]): Ray {
  const offset: [number, number, number] = [5.3, 7.1, 11.7]
  const origin: [number, number, number] = [
    target[0] + offset[0], target[1] + offset[1], target[2] + offset[2],
  ]
  const len = Math.hypot(...offset)
  return { origin, direction: [-offset[0] / len, -offset[1] / len, -offset[2] / len] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeWasmScene() {
  return {
    history_generation: vi.fn(() => 1n),
    // World drawing axes (the dashed grip→anchor line is colored against them).
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    transform_selection: vi.fn(),
    transform_def_member: vi.fn(),
    transform_def_selection: vi.fn(),
    instance_pose: vi.fn(() => new Float64Array([
      1, 0, 0, 0,
      0, 1, 0, 0,
      0, 0, 1, 0,
    ])),
  }
}

/** An objectsGroup with one named box mesh spanning [min, max] — the fixture
 * the gizmo's world-AABB is computed from (mirrors how the real Viewport
 * names an object's rendered group `Object_<id>`). */
function makeBoxObjectsGroup(
  id: bigint,
  min: [number, number, number],
  max: [number, number, number],
): THREE.Group {
  const objectsGroup = new THREE.Group()
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(max[0] - min[0], max[1] - min[1], max[2] - min[2]),
  )
  mesh.name = `Object_${id}`
  mesh.position.set((min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2)
  objectsGroup.add(mesh)
  return objectsGroup
}

function makeTool(
  selection: NodeRef[] = [],
  objectsGroup: THREE.Group | null = null,
  instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const wasmScene = makeWasmScene()
  const tool = new ScaleTool(
    wasmScene as never,
    preview,
    objectsGroup,
    selection,
    onCommit,
    onToast,
    instanceGroupGetter,
    onMeasurement,
  )
  return { tool, onCommit, onToast, onMeasurement, wasmScene, preview }
}

/** The affine `Float64Array` passed to `transform_selection`'s 4th arg on
 * its most recent call, as a plain number array for easy assertions. */
function lastAffine(wasmScene: ReturnType<typeof makeWasmScene>): number[] {
  const calls = wasmScene.transform_selection.mock.calls
  const affine = calls[calls.length - 1][3] as Float64Array
  return Array.from(affine)
}

describe('ScaleTool — auto-select on click', () => {
  // Contract change from the free-drag gesture (bounding-box grip gizmo,
  // nonuniform-scale effort): the first click that auto-acquires a selection
  // only REVEALS the gizmo — it can't also grab a grip, because grip
  // positions depend on the box, which doesn't exist until the selection
  // does. A drag starts only once a click lands within pick tolerance of an
  // actual grip (see the "grip gizmo" describe block below).
  it('empty selection with no renderable geometry: the click acquires the node but starts no drag', () => {
    const { tool, onToast } = makeTool([])
    const acquire = vi.fn(() => [{ kind: 'object', id: 9n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0))

    expect(acquire).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false) // gizmo reveal only — not a grab
  })

  it('empty selection, click auto-selects and lands on a grip of the freshly-revealed box: it only selects, the next click grabs', () => {
    const objectsGroup = makeBoxObjectsGroup(9n, [0, 0, 0], [2, 2, 1])
    const { tool } = makeTool([], objectsGroup)
    const acquire = vi.fn(() => [{ kind: 'object', id: 9n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // the +Z face grip
    expect(acquire).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1]))
    expect(acquire).toHaveBeenCalledTimes(1) // on a grip: no re-target
    expect(tool.capturingInput()).toBe(true)
  })

  it('a genuine miss (acquirer returns null) toasts and stays idle', () => {
    const { tool, onToast } = makeTool([])
    tool.setSelectionAcquirer(() => null)
    tool.onPointerDown(makeSnap(), rayThrough(0, 0))
    expect(onToast).toHaveBeenCalledWith('Click an object to scale it')
    expect(tool.capturingInput()).toBe(false)
  })

  it('idle status hint matches the selection state', () => {
    expect(makeTool([]).tool.statusHint()).toBe('Click the object you want to scale.')
    expect(makeTool([{ kind: 'object', id: 1n }]).tool.statusHint()).toBe(
      'Drag a grip to scale — a face stretches one axis, an edge two, a corner all three. Ctrl anchors at the center. Click another object to scale it instead.',
    )
  })
})

describe('ScaleTool — grip gizmo', () => {
  const ID = 9n
  const MIN: [number, number, number] = [0, 0, 0]
  const MAX: [number, number, number] = [2, 2, 1]

  function makeBoxTool(min: [number, number, number] = MIN, max: [number, number, number] = MAX) {
    const objectsGroup = makeBoxObjectsGroup(ID, min, max)
    return makeTool([{ kind: 'object', id: ID } as NodeRef], objectsGroup)
  }

  it('grabbing the +Z face grip and dragging doubles height only', () => {
    const { tool, wasmScene } = makeBoxTool()

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face
    expect(tool.capturingInput()).toBe(true)

    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2])) // drag to z=2 (double)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2])) // commit

    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(lastAffine(wasmScene)).toEqual([
      1, 0, 0, 0,
      0, 1, 0, 0,
      0, 0, 2, 0,
    ])
    expect(tool.capturingInput()).toBe(false) // back to idle, gizmo redrawn
  })

  it('grabbing a face grip drives ONLY its axis — a diagonal drag past the grip still leaves X/Y untouched', () => {
    const { tool, wasmScene } = makeBoxTool()

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face
    // A cursor that also drifted off in X/Y must not leak into those axes —
    // the +Z face grip's driven axis is Z alone.
    tool.onPointerDown(makeSnap({ x: 50, y: -30, z: 3 }), rayThrough(50, -30))

    const affine = lastAffine(wasmScene)
    expect(affine[0]).toBeCloseTo(1) // X untouched
    expect(affine[5]).toBeCloseTo(1) // Y untouched
    expect(affine[10]).toBeCloseTo(3) // Z: (3-0)/(1-0) = 3
  })

  it('typed target dimension: 50mm on a 25mm-tall (0.025m) box gives factor 2 on Z', () => {
    const { tool, wasmScene } = makeBoxTool([0, 0, 0], [2, 2, 0.025])

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0.025 }), rayAt([1, 1, 0.025])) // grab +Z face
    expect(tool.capturingInput()).toBe(true)

    for (const ch of '50mm') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    const affine = lastAffine(wasmScene)
    expect(affine[10]).toBeCloseTo(2) // 50mm / 25mm = factor 2
    expect(affine[0]).toBeCloseTo(1)
    expect(affine[5]).toBeCloseTo(1)
  })

  it('a bare typed number is a FACTOR, not a target dimension', () => {
    const { tool, wasmScene } = makeBoxTool()

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face (extent 1m)
    for (const ch of '3') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(lastAffine(wasmScene)[10]).toBeCloseTo(3)
  })

  it('center anchor (toggleCenterAnchor): both Z faces move symmetrically', () => {
    const { tool, wasmScene } = makeBoxTool()

    // toggleCenterAnchor() is the public entry the Viewport's dedicated Ctrl
    // listener calls — NOT onKey (a bare Control keydown never reaches onKey
    // through the real key pipeline; see ScaleTool.onKey's note and the Ctrl
    // e2e in tools.spec.ts).
    tool.toggleCenterAnchor() // anchor at box center
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face; center z = 0.5
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 1.5 }), rayAt([1, 1, 1.5])) // 2x from center
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1.5 }), rayAt([1, 1, 1.5])) // commit

    const affine = lastAffine(wasmScene)
    // Scale about center z=0.5 by 2: tz = 0.5*(1-2) = -0.5, so z=0 -> -0.5
    // and z=1 -> 1.5 — both faces moved (symmetric about the center).
    expect(affine[10]).toBeCloseTo(2)
    expect(affine[11]).toBeCloseTo(-0.5)
  })

  it('toggling the center anchor mid-drag re-anchors immediately from the last-known cursor', () => {
    const { tool, wasmScene } = makeBoxTool()

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 1.5 }), rayAt([1, 1, 1.5]))
    tool.toggleCenterAnchor() // flip to center-anchored mid-drag
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1.5 }), rayAt([1, 1, 1.5])) // commit at the same cursor

    const affine = lastAffine(wasmScene)
    // Same cursor (z=1.5), now anchored at center (z=0.5): factor = (1.5-0.5)/(1-0.5) = 2.
    expect(affine[10]).toBeCloseTo(2)
    expect(affine[11]).toBeCloseTo(-0.5)
  })

  it('click-drag-release: a Ctrl tap while the grab is armed (before any move) center-anchors the drag the release commits', () => {
    const { tool, wasmScene } = makeBoxTool()
    const gate = new DragCommitGate()
    // The grab arms the gate (idle before, dragging after).
    expect(gate.press(tool, 1, 100, 100, () => tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])))).toBe(true)
    // The Viewport's clean Ctrl tap lands here, before the pointer moves.
    tool.toggleCenterAnchor()
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 1.5 }), rayAt([1, 1, 1.5]))
    gate.move(1, 100, 40)
    expect(gate.release(tool, 1, 100, 40, () => tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1.5 }), rayAt([1, 1, 1.5])))).toBe(true)
    const affine = lastAffine(wasmScene)
    expect(affine[10]).toBeCloseTo(2) // (1.5 - 0.5) / (1 - 0.5), about the center
    expect(affine[11]).toBeCloseTo(-0.5)
    expect(tool.capturingInput()).toBe(false) // committed by the release, back to idle
  })

  it('dragging a grip past its anchor clamps to MIN_SCALE, never reflects', () => {
    const { tool, wasmScene } = makeBoxTool()

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face, anchor z=0
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: -3 }), rayAt([1, 1, -3])) // past the anchor
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: -3 }), rayAt([1, 1, -3])) // commit

    const affine = lastAffine(wasmScene)
    expect(affine[10]).toBeCloseTo(0.01) // MIN_SCALE, never negative/zero
    expect(affine[10]).toBeGreaterThan(0)
  })

  it('a corner grip stays uniform: all three axes scale by the same factor', () => {
    const { tool, wasmScene } = makeBoxTool()

    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 1 }), rayAt([2, 2, 1])) // grab the max corner
    // Drag exactly along the pivot(0,0,0)->grab(2,2,1) diagonal, doubled.
    tool.onPointerMove(makeSnap({ x: 4, y: 4, z: 2 }), rayAt([4, 4, 2]))
    tool.onPointerDown(makeSnap({ x: 4, y: 4, z: 2 }), rayAt([4, 4, 2])) // commit

    const affine = lastAffine(wasmScene)
    expect(affine[0]).toBeCloseTo(2)
    expect(affine[5]).toBeCloseTo(2)
    expect(affine[10]).toBeCloseTo(2)
  })

  it('an edge grip scales its two axes INDEPENDENTLY (dragging one leaves the other fixed)', () => {
    // Box 2x4x2; the +X/+Z edge grip (fixed axis = Y) sits at (2, 2, 2), its
    // opposite/anchor at (0, 2, 0). Drag only in X to (4, 2, 2): X doubles,
    // Z must NOT move — the failure the shared-diagonal ratio produced.
    const { tool, wasmScene } = makeBoxTool([0, 0, 0], [2, 4, 2])

    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 2 }), rayAt([2, 2, 2])) // grab +X/+Z edge grip
    expect(tool.capturingInput()).toBe(true)
    tool.onPointerMove(makeSnap({ x: 4, y: 2, z: 2 }), rayAt([4, 2, 2])) // drag X only
    tool.onPointerDown(makeSnap({ x: 4, y: 2, z: 2 }), rayAt([4, 2, 2])) // commit

    const affine = lastAffine(wasmScene)
    expect(affine[0]).toBeCloseTo(2) // X: (4-0)/(2-0) = 2
    expect(affine[5]).toBeCloseTo(1) // Y: not driven
    expect(affine[10]).toBeCloseTo(1) // Z: (2-0)/(2-0) = 1 — unchanged, NOT scaled by the X ratio
  })

  it('an edge grip drags each axis by its own ratio (both axes move, by different amounts)', () => {
    const { tool, wasmScene } = makeBoxTool([0, 0, 0], [2, 4, 2])

    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 2 }), rayAt([2, 2, 2])) // +X/+Z edge grip, anchor (0,2,0)
    // Drag to (4, 2, 3): X 2->4 (factor 2), Z 2->3 (factor 1.5).
    tool.onPointerDown(makeSnap({ x: 4, y: 2, z: 3 }), rayAt([4, 2, 3])) // commit

    const affine = lastAffine(wasmScene)
    expect(affine[0]).toBeCloseTo(2) // X
    expect(affine[5]).toBeCloseTo(1) // Y untouched
    expect(affine[10]).toBeCloseTo(1.5) // Z, independent
  })

  it('a single typed value on an edge grip scales both its axes proportionally (target = the 2D diagonal)', () => {
    // Box X-extent 3, Z-extent 4 -> XZ diagonal 5. Grab the +X/+Z edge grip;
    // type "10m" -> factor 10/5 = 2 applied to BOTH driven axes (comma VCB is
    // out of scope), Y untouched.
    const { tool, wasmScene } = makeBoxTool([0, 0, 0], [3, 6, 4])

    tool.onPointerDown(makeSnap({ x: 3, y: 3, z: 4 }), rayAt([3, 3, 4])) // +X/+Z edge grip
    expect(tool.capturingInput()).toBe(true)
    for (const ch of '10m') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    const affine = lastAffine(wasmScene)
    expect(affine[0]).toBeCloseTo(2) // X
    expect(affine[5]).toBeCloseTo(1) // Y untouched
    expect(affine[10]).toBeCloseTo(2) // Z
  })

  it('Esc cancels: the ghost clears, nothing commits, and the gizmo is ready again', () => {
    const { tool, wasmScene } = makeBoxTool()
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1]))
    expect(tool.capturingInput()).toBe(true)

    tool.onKey({ key: 'Escape' } as KeyboardEvent)

    expect(tool.capturingInput()).toBe(false)
    expect(wasmScene.transform_selection).not.toHaveBeenCalled()

    // The gizmo is ready again — grabbing the same grip still works.
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1]))
    expect(tool.capturingInput()).toBe(true)
  })

  it('a click far from any grip is a silent miss — no drag, no toast', () => {
    const { tool, onToast } = makeBoxTool()
    tool.onPointerDown(makeSnap({ x: 50, y: 50, z: 50 }), rayAt([50, 50, 50]))
    expect(tool.capturingInput()).toBe(false)
    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('ScaleTool — drag constrained to the grip axis/plane', () => {
  const ID = 9n
  function makeBoxTool(min: [number, number, number], max: [number, number, number]) {
    return makeTool([{ kind: 'object', id: ID } as NodeRef], makeBoxObjectsGroup(ID, min, max))
  }
  type Constraint = {
    anchor?: [number, number, number]
    lockAxis?: 0 | 1 | 2
    constraintPlane?: { point: [number, number, number]; normal: [number, number, number] }
  } | null

  it('idle: no constraint (the grip-grab click uses ordinary snapping)', () => {
    const { tool } = makeBoxTool([0, 0, 0], [2, 2, 1])
    expect((tool as unknown as { snapConstraint(): Constraint }).snapConstraint()).toBeNull()
  })

  it('face grip: locks the cursor to the driven world axis (like Move) so a +Z grip stretches, not snaps to ground', () => {
    const { tool } = makeBoxTool([0, 0, 0], [2, 2, 1])
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face at (1,1,1)
    const c = (tool as unknown as { snapConstraint(): Constraint }).snapConstraint()
    expect(c).not.toBeNull()
    expect(c!.lockAxis).toBe(2) // Z
    expect(c!.anchor).toEqual([1, 1, 1])
    expect(c!.constraintPlane).toBeUndefined()
  })

  it('edge grip: constrains the cursor to the two-axis plane (normal = the fixed third axis)', () => {
    const { tool } = makeBoxTool([0, 0, 0], [2, 4, 2])
    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 2 }), rayAt([2, 2, 2])) // grab +X/+Z edge grip
    const c = (tool as unknown as { snapConstraint(): Constraint }).snapConstraint()
    expect(c).not.toBeNull()
    expect(c!.lockAxis).toBeUndefined()
    expect(c!.constraintPlane!.normal).toEqual([0, 1, 0]) // fixed axis = Y
    expect(c!.constraintPlane!.point).toEqual([2, 2, 2])
  })

  it('corner grip: no constraint (uniform-via-diagonal keeps the ground/view resolution)', () => {
    const { tool } = makeBoxTool([0, 0, 0], [2, 2, 1])
    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 1 }), rayAt([2, 2, 1])) // grab the max corner
    expect((tool as unknown as { snapConstraint(): Constraint }).snapConstraint()).toBeNull()
  })

  it('edge grip in empty space: a ground-fallback snap is projected onto the plane, keeping the off-ground axis live', () => {
    // Box 2x4x2; grab the +X/+Z edge grip (fixed axis Y, plane y=2). A ground
    // fallback snap (kind:"ground") is OFF the plane, so the drag must project
    // the RAY onto the plane instead of using the ground point's z=0 (which
    // would collapse Z). The ray points at (4, 2, 3) on the plane.
    const { tool, wasmScene } = makeBoxTool([0, 0, 0], [2, 4, 2])
    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 2 }), rayAt([2, 2, 2])) // grab edge grip

    // A misleading ground snap far off the plane; the ray, not the snap, wins.
    tool.onPointerMove(makeSnap({ x: 99, y: 99, z: 0, kind: 'ground' }), rayAt([4, 2, 3]))
    tool.onPointerDown(makeSnap({ x: 99, y: 99, z: 0, kind: 'ground' }), rayAt([4, 2, 3])) // commit

    const calls = wasmScene.transform_selection.mock.calls
    const affine = Array.from(calls[calls.length - 1][3] as Float64Array)
    expect(affine[0]).toBeCloseTo(2) // X: (4-0)/(2-0)
    expect(affine[5]).toBeCloseTo(1) // Y untouched
    expect(affine[10]).toBeCloseTo(1.5) // Z: (3-0)/(2-0), from the RAY, not z=0
  })

  it('edge grip: a real on-plane inference snap (kind !== "ground") is used as-is', () => {
    // A genuine geometry snap is already constraint-plane-filtered onto the
    // plane, so the tool trusts it rather than re-projecting the ray.
    const { tool, wasmScene } = makeBoxTool([0, 0, 0], [2, 4, 2])
    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 2 }), rayAt([2, 2, 2])) // grab edge grip
    tool.onPointerDown(
      makeSnap({ x: 3, y: 2, z: 4, kind: 'endpoint' }), // on-plane (y=2) inference snap
      rayAt([3, 2, 4]),
    )
    const calls = wasmScene.transform_selection.mock.calls
    const affine = Array.from(calls[calls.length - 1][3] as Float64Array)
    expect(affine[0]).toBeCloseTo(1.5) // X: (3-0)/(2-0)
    expect(affine[10]).toBeCloseTo(2) // Z: (4-0)/(2-0)
  })
})

describe('ScaleTool — gizmo lifecycle (no viewport leak)', () => {
  const ID = 9n

  function makeBoxTool() {
    const objectsGroup = makeBoxObjectsGroup(ID, [0, 0, 0], [2, 2, 1])
    return makeTool([{ kind: 'object', id: ID } as NodeRef], objectsGroup)
  }

  it('activate() draws the gizmo into the shared preview group (NOT the constructor)', () => {
    // The gizmo is drawn on activate(), which ToolController.setTool() calls
    // AFTER the outgoing tool's cancel() clears the shared preview group. A
    // constructor draw would be wiped by that outgoing cancel (the invisible-
    // gizmo bug), so construction alone must NOT populate the group.
    const { tool, preview } = makeBoxTool()
    expect(preview.children.length).toBe(0) // construction draws nothing
    tool.activate()
    expect(preview.children.length).toBeGreaterThan(0) // now the gizmo is up
  })

  it('cancel() clears the gizmo from the preview group and does NOT redraw it', () => {
    // cancel() is the ONLY hook ToolController.setTool() calls on the outgoing
    // tool when switching away — so it must leave nothing stranded in the
    // shared viewport preview group (the tool-switch leak this guards).
    const { tool, preview } = makeBoxTool()
    tool.activate()
    expect(preview.children.length).toBeGreaterThan(0)

    tool.cancel()
    expect(preview.children.length).toBe(0)
  })

  it('after cancel(), the next idle pointer move re-shows the gizmo (Esc keeps the tool usable)', () => {
    const { tool, preview } = makeBoxTool()
    tool.activate()
    tool.cancel()
    expect(preview.children.length).toBe(0)

    tool.onPointerMove(makeSnap({ x: 5, y: 5, z: 5 }), rayAt([5, 5, 5]))
    expect(preview.children.length).toBeGreaterThan(0)
  })

  it('after an idle cancel(), updateGripScale is a no-op (no stale rescaling of the disposed grips)', () => {
    // cancel() must drop the grip-mesh list along with the gizmo group —
    // otherwise the render loop (which keeps calling updateGripScale while
    // the tool stays active after an idle Esc) would rescale up-to-26
    // disposed, detached meshes every frame until the next pointer move.
    const { tool, preview } = makeBoxTool()
    tool.activate()
    const meshes = preview.children[0].children.filter(
      (c): c is THREE.Mesh => c instanceof THREE.Mesh,
    )
    tool.cancel()

    const before = meshes.map((m) => m.scale.x)
    tool.updateGripScale(new THREE.PerspectiveCamera(50, 1, 0.01, 100), wppPerspective(50, 800))
    expect(meshes.map((m) => m.scale.x)).toEqual(before) // untouched
  })

  it('Esc mid-drag empties the preview group (drag ghost + gizmo both cleared)', () => {
    const { tool, preview } = makeBoxTool()
    tool.activate()
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab a grip
    expect(tool.capturingInput()).toBe(true)

    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.capturingInput()).toBe(false)
    expect(preview.children.length).toBe(0)
  })
})

describe('ScaleTool — screen-constant grip markers (updateGripScale)', () => {
  const ID = 9n
  // Mirrors ScaleTool's GRIP_SCREEN_PX — the target on-screen edge length in
  // CSS pixels every grip marker is held to.
  const SCREEN_PX = 9
  // Mirrors FALLBACK_GRIP_HALF_M — the pre-first-tick placeholder half-size.
  const FALLBACK_HALF = 0.02

  function makeBoxTool() {
    const objectsGroup = makeBoxObjectsGroup(ID, [0, 0, 0], [2, 2, 1])
    return makeTool([{ kind: 'object', id: ID } as NodeRef], objectsGroup)
  }

  /** The grip marker meshes of the currently-drawn gizmo (its group's Mesh
   * children — the outline is a Box3Helper/LineSegments, not a Mesh). */
  function gripMeshes(preview: THREE.Group): THREE.Mesh[] {
    return preview.children[0].children.filter(
      (c): c is THREE.Mesh => c instanceof THREE.Mesh,
    )
  }

  /** A ray whose nearest approach to `target` is exactly `missBy` meters —
   * built by aiming rayAt's oblique direction at a point displaced
   * perpendicular to that direction. */
  function rayMissing(target: [number, number, number], missBy: number): Ray {
    const base = rayAt(target)
    const dir = new THREE.Vector3(...base.direction)
    const perp = new THREE.Vector3().crossVectors(dir, new THREE.Vector3(0, 0, 1)).normalize()
    const aim: [number, number, number] = [
      target[0] + perp.x * missBy,
      target[1] + perp.y * missBy,
      target[2] + perp.z * missBy,
    ]
    return rayAt(aim)
  }

  it('each grip is scaled so its WORLD size projects to the same on-screen pixel size at its own distance', () => {
    const { tool, preview } = makeBoxTool()
    tool.activate()
    const meshes = gripMeshes(preview)
    expect(meshes.length).toBe(26) // 6 face + 8 corner + 12 edge grips

    const camera = new THREE.PerspectiveCamera(50, 1, 0.01, 100)
    camera.position.set(0, -6, 3)
    const height = 800
    tool.updateGripScale(camera, wppPerspective(50, height))

    const tanHalf = Math.tan((50 * Math.PI) / 360)
    const halves = meshes.map((m) => {
      const dist = camera.position.distanceTo(m.position)
      const expected = (SCREEN_PX * dist * tanHalf) / height
      expect(m.scale.x).toBeCloseTo(expected, 10)
      expect(m.scale.y).toBeCloseTo(expected, 10) // uniform
      expect(m.scale.z).toBeCloseTo(expected, 10)
      return m.scale.x
    })
    // Grips sit at different camera distances, so their WORLD sizes differ —
    // that per-grip variation is what keeps the SCREEN size constant.
    expect(Math.max(...halves)).toBeGreaterThan(Math.min(...halves))
  })

  it('pick tolerance follows the rendered screen size: tight when grips render small, forgiving when they render large', () => {
    const missBy = 0.05 // under the 0.02·3 fallback tolerance, so it hits pre-tick
    const grip: [number, number, number] = [1, 1, 1] // the +Z face grip

    // Small-rendering setup (narrow FOV, tall viewport): tolerance shrinks
    // below 0.05 → the same near-miss ray now misses.
    const tight = makeBoxTool()
    tight.tool.activate()
    tight.tool.updateGripScale(new THREE.PerspectiveCamera(10, 1, 0.01, 100), wppPerspective(10, 4000))
    tight.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayMissing(grip, missBy))
    expect(tight.tool.capturingInput()).toBe(false)

    // Large-rendering setup (wide FOV, short viewport): tolerance grows well
    // past 0.05 → the near-miss ray grabs the grip.
    const loose = makeBoxTool()
    loose.tool.activate()
    loose.tool.updateGripScale(new THREE.PerspectiveCamera(90, 1, 0.01, 100), wppPerspective(90, 200))
    loose.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayMissing(grip, missBy))
    expect(loose.tool.capturingInput()).toBe(true)
  })

  it('a click inside a FAR grip\'s tolerance is not swallowed by a nearer-to-camera grip failing its own tighter tolerance', () => {
    // A deep box viewed from one end: grips near the camera have tiny
    // (screen-tracking) tolerances, grips at the far end much larger ones. A
    // ray that passes fairly close (in world units) to the near grips but
    // well inside the FAR face grip's own tolerance must grab the far grip —
    // picking the raw-world-nearest grip first and then testing only ITS
    // tolerance would return a miss (the regression this pins).
    const objectsGroup = makeBoxObjectsGroup(ID, [0, 0, 0], [0.04, 10, 0.04])
    const { tool } = makeTool([{ kind: 'object', id: ID } as NodeRef], objectsGroup)
    tool.activate()

    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100)
    camera.position.set(0.06, -0.5, 0.02)
    tool.updateGripScale(camera, wppPerspective(45, 900))

    // From just past the near end, aiming down the box's length with a tiny
    // outward drift: every near-end grip is missed by ~0.02–0.04 m (far
    // outside its ~0.006 m tolerance) while the far +Y face grip at
    // (0.02, 10, 0.02) is missed by ~0.045 m — comfortably inside its
    // ~0.13 m tolerance.
    const dir = new THREE.Vector3(0.0005, 1, 0).normalize()
    const ray: Ray = {
      origin: [0.06, -0.5, 0.02],
      direction: [dir.x, dir.y, dir.z],
    }
    tool.onPointerDown(makeSnap({ x: 0.06, y: 0, z: 0.02 }), ray)
    expect(tool.capturingInput()).toBe(true)
  })

  it('before the first tick, the placeholder size and fallback tolerance hold', () => {
    const { tool, preview } = makeBoxTool()
    tool.activate()
    const meshes = gripMeshes(preview)
    for (const m of meshes) expect(m.scale.x).toBeCloseTo(FALLBACK_HALF, 10)

    // The fallback pick tolerance (0.02·3 = 0.06) still governs: a 0.05-off
    // ray grabs the grip exactly as the pre-rework tests rely on.
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayMissing([1, 1, 1], 0.05))
    expect(tool.capturingInput()).toBe(true)
  })

  it('DESIGN CHANGE: an orthographic (parallel-projection) camera now scales the grips too — no longer a silent no-op', () => {
    // Before Phase 1, an `instanceof PerspectiveCamera` guard made this a
    // no-op under ortho — every grip would silently vanish (freeze at its
    // pre-tick placeholder size) the moment the user toggled Parallel
    // Projection. The tool now takes a `worldPerPixel` callback instead of a
    // camera+viewportHeight pair, so it has no projection-specific branch at
    // all; the caller (Viewport.tsx, via CameraRig.worldPerPixel) is what
    // makes ortho behave correctly.
    const { tool, preview } = makeBoxTool()
    tool.activate()
    const meshes = gripMeshes(preview)

    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 10)
    ortho.position.set(0, -6, 3)
    const frustumHeight = 4 // top(1) - bottom(-1)
    const wppOrtho = worldPerPixelOrtho(frustumHeight, 1, 800)
    tool.updateGripScale(ortho, () => wppOrtho) // ortho: distance-independent

    for (const m of meshes) {
      const expected = (SCREEN_PX * wppOrtho) / 2
      expect(m.scale.x).toBeCloseTo(expected, 10)
    }
  })
})

describe('ScaleTool — setEditContext aborts an armed gesture on a genuine change (component-edit-parity.md phase A2)', () => {
  const ID = 9n
  function makeBoxTool(min: [number, number, number], max: [number, number, number]) {
    return makeTool([{ kind: 'object', id: ID } as NodeRef], makeBoxObjectsGroup(ID, min, max))
  }

  it('builds and commits from a definition member rendered through the active instance', () => {
    const instance = 77n
    const member = 9n
    const instanceGroup = new THREE.Group()
    instanceGroup.name = `Instance_${instance}`
    const memberMesh = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 1))
    memberMesh.name = `InstanceFace_${instance}_${member}_0`
    memberMesh.position.set(1, 1, 0.5)
    instanceGroup.add(memberMesh)
    const { tool, wasmScene } = makeTool(
      [{ kind: 'object', id: member }],
      null,
      (id) => id === instance ? instanceGroup : null,
    )
    tool.setEditContext({ kind: 'instance', id: instance, component: 90n })
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1]))
    expect(tool.capturingInput()).toBe(true)
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2]))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2]))
    expect(wasmScene.transform_def_selection).toHaveBeenCalledTimes(1)
    expect(wasmScene.transform_selection).not.toHaveBeenCalled()
  })

  it('a genuine context change cancels an armed grip drag instead of silently retargeting its eventual commit', () => {
    const { tool } = makeBoxTool([0, 0, 0], [2, 2, 1])
    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face grip
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'top' })

    expect(tool.capturingInput()).toBe(false)
  })

  it('re-pushing the SAME context is a no-op — an armed grip drag survives it untouched', () => {
    const { tool } = makeBoxTool([0, 0, 0], [2, 2, 1])
    const ctx = { kind: 'instance' as const, id: 9n, component: 90n }
    tool.setEditContext(ctx)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face grip
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })

    expect(tool.capturingInput()).toBe(true)
  })
})

// Post-commit retype (retypeWindow.ts): type a factor or a dimension after a
// grip drag commits and the scale is redone on the same grip axes.
describe('ScaleTool — retype the factor or dimension after the commit', () => {
  const ID = 9n
  const MIN: [number, number, number] = [0, 0, 0]
  const MAX: [number, number, number] = [2, 2, 1]
  function makeBoxTool() {
    const t = makeTool([{ kind: 'object', id: ID } as NodeRef], makeBoxObjectsGroup(ID, MIN, MAX))
    // A stateful history for the window: transform_selection records a step.
    let gen = 1n
    t.wasmScene.history_generation.mockImplementation(() => gen)
    t.wasmScene.transform_selection.mockImplementation(() => { gen += 1n })
    const undo = vi.fn(() => { gen += 1n; return { free: vi.fn() } })
    Object.assign(t.wasmScene, { scene_undo: undo, scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }) })
    return { ...t, undo }
  }
  const typeIn = (tool: ScaleTool, text: string) => { for (const ch of text) tool.onKey({ key: ch } as KeyboardEvent); tool.onKey({ key: 'Enter' } as KeyboardEvent) }

  it('the +Z face drag doubled the height; typing 3 redoes it ×3 on Z only, then 0.5m sets the height to 0.5 m', () => {
    const t = makeBoxTool()
    const top: [number, number, number] = [(MIN[0] + MAX[0]) / 2, (MIN[1] + MAX[1]) / 2, MAX[2]]
    const height = MAX[2] - MIN[2]
    t.tool.onPointerDown(makeSnap({ x: top[0], y: top[1], z: top[2] }), rayAt(top))
    const dragged: [number, number, number] = [top[0], top[1], MIN[2] + 2 * height]
    t.tool.onPointerMove(makeSnap({ x: dragged[0], y: dragged[1], z: dragged[2] }), rayAt(dragged))
    t.tool.onPointerDown(makeSnap({ x: dragged[0], y: dragged[1], z: dragged[2] }), rayAt(dragged))
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(lastAffine(t.wasmScene)[10]).toBeCloseTo(2, 9)
    expect(t.tool.statusHint()).toContain('redo the scale')
    expect(t.tool.capturesKey('3')).toBe(true)
    expect(t.tool.capturesKey('s')).toBe(false)

    typeIn(t.tool, '3')
    expect(t.undo).toHaveBeenCalledTimes(1)
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(2)
    const a = lastAffine(t.wasmScene)
    expect(a[0]).toBeCloseTo(1, 9)
    expect(a[5]).toBeCloseTo(1, 9)
    expect(a[10]).toBeCloseTo(3, 9)

    // A length is a target dimension measured against the PRE-scale height.
    typeIn(t.tool, '0.5m')
    expect(t.undo).toHaveBeenCalledTimes(2)
    expect(lastAffine(t.wasmScene)[10]).toBeCloseTo(0.5 / height, 9)
  })
})

describe('ScaleTool — retype after an arranging drawn-shape scale', () => {
  it('re-commits with the PRE-commit shape refs (the undo restores the original keys), while the visible selection is the re-resolved unit', () => {
    const ctx = ctxGround()
    const hex = (n: bigint): string => n.toString(16).padStart(16, '0')
    const di = (key: bigint): NodeRef => draftRef('draft-island', ctx, hex(key))
    const sel: NodeRef[] = [di(40n)]
    const t = makeTool(sel)
    let gen = 1n
    let ids: bigint[] = [40n, 41n]
    // The listing of the ground plane: unit 40 is a free shape (so the
    // z-scale that leaves the plane DETACHES it through the island op).
    let units: { key: bigint; edge: bigint; a: number[]; b: number[] }[] = [
      { key: 40n, edge: 10n, a: [0, 0, 0], b: [2, 2, 1] },
      { key: 41n, edge: 20n, a: [5, 5, 0], b: [6, 5, 0] },
    ]
    const scene = t.wasmScene as unknown as Record<string, ReturnType<typeof vi.fn>>
    Object.assign(scene, {
      history_generation: vi.fn(() => gen),
      plane_units_json: vi.fn((_c: string) =>
        JSON.stringify({
          units: units.map((u) => ({
            key: hex(u.key),
            edges: [{ key: hex(u.edge), a: u.a, b: u.b, store: { kind: 'free', sketch: 3 }, curve: null }],
          })),
          regions: [],
        }),
      ),
      sketch_island_ids: vi.fn(() => ids),
      // The shape's island handle, found by its edge's own position.
      sketch_island_at_edge_positions: vi.fn(() => 40n),
      // The first landing arranges: island 40 merges into fresh island 99
      // (its edge re-keyed, found by geometry afterwards).
      transform_sketch_islands: vi.fn((_s: bigint, _i: BigUint64Array, affine: Float64Array) => {
        gen += 1n
        ids = [99n, 41n]
        units = [
          { key: 99n, edge: 60n, a: [0, 0, 0], b: [2, 2, affine[10]] },
          units[1],
        ]
      }),
      scene_undo: vi.fn(() => {
        gen += 1n
        ids = [40n, 41n]
        units = [
          { key: 40n, edge: 10n, a: [0, 0, 0], b: [2, 2, 1] },
          units[1],
        ]
        return { free: vi.fn() }
      }),
      scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    })

    // Drag the +Z face grip of the shape's box to double its height.
    const top: [number, number, number] = [1, 1, 1]
    t.tool.onPointerDown(makeSnap({ x: top[0], y: top[1], z: top[2] }), rayAt(top))
    const dragged: [number, number, number] = [1, 1, 2]
    t.tool.onPointerMove(makeSnap({ x: dragged[0], y: dragged[1], z: dragged[2] }), rayAt(dragged))
    t.tool.onPointerDown(makeSnap({ x: dragged[0], y: dragged[1], z: dragged[2] }), rayAt(dragged))
    expect(scene.transform_sketch_islands).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenLastCalledWith([di(99n)])

    for (const ch of '3') t.tool.onKey({ key: ch } as KeyboardEvent)
    t.tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(scene.transform_sketch_islands).toHaveBeenCalledTimes(2)
    expect([...(scene.transform_sketch_islands.mock.calls[1][1] as BigUint64Array)]).toEqual([40n])
    expect((scene.transform_sketch_islands.mock.calls[1][2] as Float64Array)[10]).toBeCloseTo(3, 9)
    expect(t.onCommit).toHaveBeenLastCalledWith([di(99n)])
  })

  it('an in-plane scale of a drawn unit drafts it over ALL its edge keys, and the gizmo is built from the unit\'s own geometry', () => {
    const ctx = ctxGround()
    const hex = (n: bigint): string => n.toString(16).padStart(16, '0')
    const t = makeTool([draftRef('draft-island', ctx, hex(40n))])
    const scene = t.wasmScene as unknown as Record<string, ReturnType<typeof vi.fn>>
    let gen = 1n
    Object.assign(scene, {
      history_generation: vi.fn(() => gen),
      plane_units_json: vi.fn((_c: string) =>
        JSON.stringify({
          units: [{
            key: hex(40n),
            edges: [
              { key: hex(10n), a: [0, 0, 0], b: [2, 0, 0], store: { kind: 'free', sketch: 3 }, curve: null },
              { key: hex(11n), a: [2, 0, 0], b: [2, 2, 0], store: { kind: 'free', sketch: 3 }, curve: null },
            ],
          }],
          regions: [],
        }),
      ),
      draft_begin_ctx: vi.fn(() => 1n),
      draft_transform_edges: vi.fn(),
      draft_end: vi.fn(() => { gen += 1n; return '{"objects":[],"sketches":[],"kept_as_sketch":[]}' }),
      draft_cancel: vi.fn(),
    })
    // The flat unit's +X face grip (at the box's far X side) dragged 2 → 4:
    // an in-plane scale along X.
    const grip: [number, number, number] = [2, 1, 0]
    t.tool.onPointerDown(makeSnap({ x: grip[0], y: grip[1], z: grip[2] }), rayAt(grip))
    const dragged: [number, number, number] = [4, 1, 0]
    t.tool.onPointerMove(makeSnap({ x: dragged[0], y: dragged[1], z: dragged[2] }), rayAt(dragged))
    t.tool.onPointerDown(makeSnap({ x: dragged[0], y: dragged[1], z: dragged[2] }), rayAt(dragged))
    expect(scene.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_ctx).toHaveBeenCalledWith(ctx, 'Scale') // the single draft IS the history entry: labeled for the tool
    expect([...(scene.draft_transform_edges.mock.calls[0][1] as BigUint64Array)]).toEqual([10n, 11n])
    const a = scene.draft_transform_edges.mock.calls[0][2] as Float64Array
    expect(a[0]).toBeCloseTo(2, 9)
    expect(scene.transform_selection).not.toHaveBeenCalled()
  })
})

describe('ScaleTool — an identity retype retracts the scale', () => {
  it('typing 1 after a ×2 undoes the scale and closes the window instead of redoing the old one', () => {
    const ID = 9n
    const t = makeTool([{ kind: 'object', id: ID } as NodeRef], makeBoxObjectsGroup(ID, [0, 0, 0], [2, 2, 1]))
    let gen = 1n
    t.wasmScene.history_generation.mockImplementation(() => gen)
    t.wasmScene.transform_selection.mockImplementation(() => { gen += 1n })
    const undo = vi.fn(() => { gen += 1n; return { free: vi.fn() } })
    const redo = vi.fn(() => { gen += 1n; return { free: vi.fn() } })
    Object.assign(t.wasmScene, { scene_undo: undo, scene_redo: redo })
    t.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1]))
    t.tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2]))
    t.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2]))
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    for (const ch of '1') t.tool.onKey({ key: ch } as KeyboardEvent)
    t.tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(undo).toHaveBeenCalledTimes(1)
    expect(redo).not.toHaveBeenCalled()
    expect(t.wasmScene.transform_selection).toHaveBeenCalledTimes(1) // no identity re-commit
    expect(t.tool.capturesKey('2')).toBe(false) // window closed
    expect(t.onCommit).toHaveBeenCalledTimes(2) // the retract refreshed the host
  })
})


describe('ScaleTool — a clicked factor is exactly the displayed factor', () => {
  const ID = 9n
  function drag(kind: string) {
    const objectsGroup = makeBoxObjectsGroup(ID, [0, 0, 0], [2, 2, 1])
    const { tool, wasmScene, onMeasurement } = makeTool([{ kind: 'object', id: ID } as NodeRef], objectsGroup)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab +Z face, anchor z=0
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 1.61374, kind }), rayAt([1, 1, 1.61374]))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1.61374, kind }), rayAt([1, 1, 1.61374]))
    return { z: lastAffine(wasmScene)[10], onMeasurement }
  }

  it('a free click commits the shown ×1.61, not the raw ratio', () => {
    const r = drag('ground')
    expect(r.onMeasurement).toHaveBeenCalledWith(expect.stringContaining('×1.61'))
    expect(r.z).toBe(1.61)
  })

  it('a hard-snapped click keeps the raw ratio', () => {
    expect(drag('endpoint').z).toBeCloseTo(1.61374, 12)
    expect(drag('endpoint').z).not.toBe(1.61)
  })
})

describe('ScaleTool — the drag ghost is session geometry, mirrored onto sibling placements', () => {
  it('grabbing a grip puts a marked ghost in the preview (the gizmo is not one); the mirror shows it and clears with Esc', () => {
    const ID = 9n
    const objectsGroup = makeBoxObjectsGroup(ID, [0, 0, 0], [2, 2, 1])
    const { tool, preview } = makeTool([{ kind: 'object', id: ID } as NodeRef], objectsGroup)
    const root = new THREE.Group()
    const mirror = new SiblingPreviewMirror({ source: preview, root, frames: () => [1, 0, 0, 4, 0, 1, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 7, 0, 0, 1, 0] })

    tool.activate()
    expect(preview.children.filter(isSessionGhost)).toHaveLength(0) // gizmo only
    mirror.sync()
    expect(mirror.mirrorCount).toBe(0)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // grab a grip
    expect(tool.capturingInput()).toBe(true)
    const ghosts = preview.children.filter(isSessionGhost)
    expect(ghosts).toHaveLength(1)
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)
    expect((root.children[1] as THREE.Group).children).toHaveLength(1)

    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    mirror.sync()
    expect(mirror.mirrorCount).toBe(0)
  })
})

describe('ScaleTool — the hot pair (hovered grip and the point it scales about)', () => {
  const ID = 9n
  const MIN: [number, number, number] = [0, 0, 0]
  const MAX: [number, number, number] = [2, 2, 1]
  const FACE_PZ: [number, number, number] = [1, 1, 1] // +Z face grip; opposite (1,1,0)

  function makeBoxTool() {
    return makeTool([{ kind: 'object', id: ID } as NodeRef], makeBoxObjectsGroup(ID, MIN, MAX))
  }

  /** [grip, anchor] marker positions and the joining line's color, or null. */
  function hotPair(preview: THREE.Group): { grip: number[]; anchor: number[]; lineColor: number } | null {
    const group = preview.getObjectByName('ScaleHotGrips')
    if (group === undefined) return null
    const [grip, anchor] = group.children.filter((c) => c instanceof THREE.Mesh) as THREE.Mesh[]
    const line = group.children.find((c) => c instanceof THREE.LineSegments) as THREE.LineSegments
    return {
      grip: grip.position.toArray(),
      anchor: anchor.position.toArray(),
      lineColor: (line.material as THREE.LineDashedMaterial).color.getHex(),
    }
  }

  it('hovering a grip lights it and its opposite, joined by a line in the axis color; off the grips, nothing', () => {
    const { tool, preview } = makeBoxTool()
    tool.onPointerMove(makeSnap(), rayAt(FACE_PZ))
    const pair = hotPair(preview)!
    expect(pair.grip).toEqual([1, 1, 1])
    expect(pair.anchor).toEqual([1, 1, 0])
    expect(pair.lineColor).not.toBe(0x000000) // on the Z axis: blue

    tool.onPointerMove(makeSnap(), rayThrough(10, 10))
    expect(hotPair(preview)).toBeNull()
  })

  it('a corner grip\'s line runs diagonally, in black', () => {
    const { tool, preview } = makeBoxTool()
    tool.onPointerMove(makeSnap(), rayAt([2, 2, 1]))
    const pair = hotPair(preview)!
    expect(pair.anchor).toEqual([0, 0, 0])
    expect(pair.lineColor).toBe(0x000000)
  })

  it('Ctrl\'s center anchor moves the second point to the box center at once', () => {
    const { tool, preview } = makeBoxTool()
    tool.onPointerMove(makeSnap(), rayAt(FACE_PZ))
    tool.toggleCenterAnchor()
    expect(hotPair(preview)!.anchor).toEqual([1, 1, 0.5])
  })

  it('stays through the drag — the grip riding the scale, the anchor fixed — and lights the new box\'s grip after', () => {
    const objects = makeBoxObjectsGroup(ID, MIN, MAX)
    const { tool, preview, onCommit } = makeTool([{ kind: 'object', id: ID } as NodeRef], objects)
    // The commit rebuilds the object the way the app does: twice as tall.
    onCommit.mockImplementation(() => {
      objects.remove(objects.children[0])
      objects.add(makeBoxObjectsGroup(ID, MIN, [2, 2, 2]).children[0])
    })
    tool.onPointerMove(makeSnap(), rayAt(FACE_PZ))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt(FACE_PZ)) // grab
    expect(preview.getObjectByName('ScaleGizmo')).toBeUndefined()

    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2]))
    const pair = hotPair(preview)!
    expect(pair.grip[2]).toBeCloseTo(2)
    expect(pair.anchor).toEqual([1, 1, 0])

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 2 }), rayAt([1, 1, 2])) // commit
    expect(tool.capturingInput()).toBe(false)
    // The cursor rests on the new box's top grip: lit with no further move.
    expect(hotPair(preview)!.grip).toEqual([1, 1, 2])
    expect(hotPair(preview)!.anchor).toEqual([1, 1, 0])
    // And again after the app's selection echo redraws the gizmo.
    tool.setSelection([{ kind: 'object', id: ID } as NodeRef])
    expect(hotPair(preview)!.grip).toEqual([1, 1, 2])
  })

  it('comes back at once when the gizmo is redrawn under a parked cursor (the app\'s selection echo)', () => {
    const { tool, preview } = makeBoxTool()
    tool.onPointerMove(makeSnap(), rayAt(FACE_PZ))
    expect(hotPair(preview)).not.toBeNull()
    tool.setSelection([{ kind: 'object', id: ID } as NodeRef])
    expect(hotPair(preview)!.grip).toEqual([1, 1, 1])
  })

  it('once the pointer has left the canvas, a redraw lights nothing', () => {
    const { tool, preview } = makeBoxTool()
    tool.onPointerMove(makeSnap(), rayAt(FACE_PZ))
    tool.onPointerLeave()
    tool.setSelection([{ kind: 'object', id: ID } as NodeRef])
    expect(hotPair(preview)).toBeNull()
    tool.onPointerMove(makeSnap(), rayAt(FACE_PZ))
    expect(hotPair(preview)!.grip).toEqual([1, 1, 1])
  })

  it('Esc clears it with the rest of the preview', () => {
    const { tool, preview } = makeBoxTool()
    tool.onPointerMove(makeSnap(), rayAt(FACE_PZ))
    tool.onKey({ key: 'Escape', repeat: false, preventDefault: () => { /* no-op */ } } as KeyboardEvent)
    expect(hotPair(preview)).toBeNull()
  })
})

describe('ScaleTool — a click off the grips re-targets (better than SketchUp)', () => {
  const ID = 9n
  const OTHER = 8n

  function makeTwoBoxTool() {
    const objectsGroup = makeBoxObjectsGroup(ID, [0, 0, 0], [2, 2, 1])
    objectsGroup.add(makeBoxObjectsGroup(OTHER, [5, 5, 0], [6, 6, 1]).children[0])
    const t = makeTool([{ kind: 'object', id: ID } as NodeRef], objectsGroup)
    const release = vi.fn()
    return { ...t, release }
  }

  it('on another object: selects it and moves the gizmo there, grabbing nothing', () => {
    const t = makeTwoBoxTool()
    const acquire = vi.fn(() => [{ kind: 'object', id: OTHER } as NodeRef])
    t.tool.setSelectionAcquirer(acquire, t.release)

    t.tool.onPointerDown(makeSnap({ x: 5.5, y: 5.2, z: 1 }), rayThrough(5.5, 5.2))
    expect(acquire).toHaveBeenCalledTimes(1)
    expect(t.tool.capturingInput()).toBe(false)
    expect(t.release).not.toHaveBeenCalled()

    // The gizmo is the other box's now: its +Z face grip grabs.
    t.tool.onPointerDown(makeSnap({ x: 5.5, y: 5.5, z: 1 }), rayAt([5.5, 5.5, 1]))
    expect(t.tool.capturingInput()).toBe(true)
  })

  it('on nothing: clears the selection and the gizmo', () => {
    const t = makeTwoBoxTool()
    t.tool.setSelectionAcquirer(() => null, t.release)
    t.tool.onPointerDown(makeSnap({ x: 20, y: 20, z: 0 }), rayThrough(20, 20))
    expect(t.release).toHaveBeenCalledTimes(1)
    expect(t.preview.getObjectByName('ScaleGizmo')).toBeUndefined()
    expect(t.tool.statusHint()).toBe('Click the object you want to scale.')
  })

  it('on a grip it grabs, never re-targets', () => {
    const t = makeTwoBoxTool()
    const acquire = vi.fn(() => [{ kind: 'object', id: OTHER } as NodeRef])
    t.tool.setSelectionAcquirer(acquire, t.release)
    t.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1]))
    expect(acquire).not.toHaveBeenCalled()
    expect(t.tool.capturingInput()).toBe(true)
  })

  it('what a click would take (clickAcquire): a node off the grips, nothing over one or mid-drag', () => {
    const t = makeTwoBoxTool()
    t.tool.setSelectionAcquirer(() => null, t.release)
    t.tool.onPointerMove(makeSnap(), rayThrough(20, 20))
    expect(t.tool.clickAcquire()).toEqual({ guides: false, nodes: true })
    t.tool.onPointerMove(makeSnap(), rayAt([1, 1, 1]))
    expect(t.tool.clickAcquire()).toBeNull()
    t.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1]))
    expect(t.tool.clickAcquire()).toBeNull()
  })
})
