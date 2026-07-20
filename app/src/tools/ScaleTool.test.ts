import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ScaleTool } from './ScaleTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

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
    transform_selection: vi.fn(),
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

function makeTool(selection: NodeRef[] = [], objectsGroup: THREE.Group | null = null) {
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
    null,
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

  it('empty selection, click auto-selects AND lands on a grip of the freshly-revealed box: the same click starts dragging', () => {
    const objectsGroup = makeBoxObjectsGroup(9n, [0, 0, 0], [2, 2, 1])
    const { tool } = makeTool([], objectsGroup)
    const acquire = vi.fn(() => [{ kind: 'object', id: 9n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayAt([1, 1, 1])) // the +Z face grip
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
      'Drag a grip to scale — a face stretches one axis, an edge two, a corner all three. Ctrl anchors at the center.',
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
    tool.updateGripScale(new THREE.PerspectiveCamera(50, 1, 0.01, 100), 800)
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
    tool.updateGripScale(camera, height)

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
    tight.tool.updateGripScale(new THREE.PerspectiveCamera(10, 1, 0.01, 100), 4000)
    tight.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayMissing(grip, missBy))
    expect(tight.tool.capturingInput()).toBe(false)

    // Large-rendering setup (wide FOV, short viewport): tolerance grows well
    // past 0.05 → the near-miss ray grabs the grip.
    const loose = makeBoxTool()
    loose.tool.activate()
    loose.tool.updateGripScale(new THREE.PerspectiveCamera(90, 1, 0.01, 100), 200)
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
    tool.updateGripScale(camera, 900)

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

  it('before the first tick (and for a non-perspective camera) the placeholder size and fallback tolerance hold', () => {
    const { tool, preview } = makeBoxTool()
    tool.activate()
    const meshes = gripMeshes(preview)
    for (const m of meshes) expect(m.scale.x).toBeCloseTo(FALLBACK_HALF, 10)

    // A non-perspective camera is a no-op: sizes unchanged, nothing cached.
    tool.updateGripScale(new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 10), 800)
    for (const m of meshes) expect(m.scale.x).toBeCloseTo(FALLBACK_HALF, 10)

    // The fallback pick tolerance (0.02·3 = 0.06) still governs: a 0.05-off
    // ray grabs the grip exactly as the pre-rework tests rely on.
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1 }), rayMissing([1, 1, 1], 0.05))
    expect(tool.capturingInput()).toBe(true)
  })
})
