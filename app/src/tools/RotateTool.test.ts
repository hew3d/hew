import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RotateTool } from './RotateTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { worldPerPixelPerspective, worldPerPixelOrtho } from '../viewport/math'

/** ~2° axis tolerance, matching RotateTool's own AXIS_SNAP_TOL_DOT. */
const TOL = Math.cos((2 * Math.PI) / 180)

/** A ray straight down (−Z) through world (x, y): hits the z=0 plane at (x,y,0),
 * so a sweep move at (x,y) lands the cursor there in the ground rotation plane. */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** Minimal KeyboardEvent-shaped fake — onKey only reads .key/.repeat and calls
 * .preventDefault(). */
function makeKeyEvent(key: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  let defaultPrevented = false
  return {
    key,
    repeat: opts.repeat ?? false,
    get defaultPrevented() { return defaultPrevented },
    preventDefault: () => { defaultPrevented = true },
  } as unknown as KeyboardEvent
}

/** Minimal WasmScene stub — only the members RotateTool calls. */
function makeWasmScene(faceNormal?: [number, number, number]) {
  return {
    face_normal: vi.fn((..._args: unknown[]) => {
      if (faceNormal === undefined) throw new Error('not a live world-object face')
      return new Float64Array(faceNormal)
    }),
    // No face under the ray → RotateTool's fallback returns world +Z.
    pick_face: vi.fn(() => undefined),
    transform_selection: vi.fn(),
  }
}

function makeTool(opts: {
  faceNormal?: [number, number, number]
  selection?: NodeRef[]
  instance?: { id: bigint; group: THREE.Group }
} = {}) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const wasmScene = makeWasmScene(opts.faceNormal)
  const selection: NodeRef[] = opts.selection ?? [{ kind: 'object', id: 1n }]
  const tool = new RotateTool(
    wasmScene as never,
    preview,
    null, // world objectsGroup
    selection,
    onCommit,
    onToast,
    opts.instance === undefined
      ? null
      : (id) => id === opts.instance!.id ? opts.instance!.group : null,
    onMeasurement,
  )
  return { tool, preview, onCommit, onToast, onMeasurement, wasmScene }
}

// ── disk inspection helpers ──────────────────────────────────────────────────

function diskGroup(preview: THREE.Group): THREE.Group {
  const g = preview.children.find((c) => c instanceof THREE.Group)
  expect(g, 'a protractor disk group should exist').toBeDefined()
  return g as THREE.Group
}

function ringColorHex(preview: THREE.Group): number {
  const ring = diskGroup(preview).children.find((c) => c instanceof THREE.LineLoop) as THREE.LineLoop
  expect(ring, 'the disk should have a ring (LineLoop)').toBeDefined()
  return (ring.material as THREE.LineBasicMaterial).color.getHex()
}

/** Count of LineSegments in the disk group: the lock tick and/or sweep arms. In
 * the idle phase (no arms) this is 1 exactly when the axis is locked. */
function lineSegmentCount(preview: THREE.Group): number {
  return diskGroup(preview).children.filter((c) => c instanceof THREE.LineSegments).length
}

/** The color RotateTool assigns a ring whose normal points along `dir`, run
 * through a LineBasicMaterial so color-management matches the tool's own path. */
function axisColorHex(dir: [number, number, number]): number {
  const match = axisColorForDirection(dir, TOL, axisColorsForTheme(getResolvedTheme()))
  expect(match, `${dir} should map to a world axis color`).not.toBeNull()
  return new THREE.LineBasicMaterial({ color: match!.color }).color.getHex()
}

// ── tests ────────────────────────────────────────────────────────────────────

describe('RotateTool — protractor widget (idle/hover)', () => {
  it('shows a blue (Z) protractor on the ground by default, centered at the cursor', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))
    expect(diskGroup(preview).position.toArray()).toEqual([1, 2, 0])
    expect(ringColorHex(preview)).toBe(axisColorHex([0, 0, 1]))
    expect(lineSegmentCount(preview)).toBe(0) // unlocked → no tick
  })

  it('recolors the disk to the hovered face axis (side face → red X)', () => {
    const { tool, preview } = makeTool({ faceNormal: [1, 0, 0] })
    tool.onPointerMove(makeSnap({ kind: 'face', elementKind: 'face', object: 1n, element: 2n }), rayThrough(0, 0))
    expect(ringColorHex(preview)).toBe(axisColorHex([1, 0, 0]))
    expect(ringColorHex(preview)).not.toBe(axisColorHex([0, 0, 1]))
  })
})

describe('RotateTool — axis locking', () => {
  it('ArrowRight locks X: recolors to red and emphasizes the disk with a normal tick', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    expect(lineSegmentCount(preview)).toBe(0)

    const ev = makeKeyEvent('ArrowRight')
    tool.onKey(ev)
    expect(ev.defaultPrevented).toBe(true)
    expect(ringColorHex(preview)).toBe(axisColorHex([1, 0, 0]))
    expect(lineSegmentCount(preview)).toBe(1) // locked → normal tick added
  })

  it('ArrowDown clears the lock, returning to the inferred (ground Z) axis', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('ArrowRight'))
    tool.onKey(makeKeyEvent('ArrowDown'))
    expect(ringColorHex(preview)).toBe(axisColorHex([0, 0, 1]))
    expect(lineSegmentCount(preview)).toBe(0)
  })

  it('Shift toggles the lock on and off', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('Shift'))
    expect(lineSegmentCount(preview)).toBe(1)
    tool.onKey(makeKeyEvent('Shift'))
    expect(lineSegmentCount(preview)).toBe(0)
  })

  it('ignores Shift keydown autorepeat', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onKey(makeKeyEvent('Shift', { repeat: true }))
    expect(lineSegmentCount(preview)).toBe(0) // no lock, no tick
  })
})

describe('RotateTool — gesture', () => {
  // Deliberate contract change (selection-UX overhaul): an empty-selection
  // click no longer demands a prior Select step — the Viewport injects an
  // acquirer that picks the node under the cursor. The toast survives only
  // for a genuine miss (nothing under the cursor / no acquirer injected).
  it('empty selection with no acquirer (or a miss): toasts and stays idle', () => {
    const { tool, onToast } = makeTool({ selection: [] })
    tool.onPointerMove(makeSnap(), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(), rayThrough(0, 0))
    expect(onToast).toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false) // never advanced past idle

    const missTool = makeTool({ selection: [] })
    missTool.tool.setSelectionAcquirer(() => null)
    missTool.tool.onPointerDown(makeSnap(), rayThrough(0, 0))
    expect(missTool.onToast).toHaveBeenCalled()
    expect(missTool.tool.capturingInput()).toBe(false)
  })

  it('idle status hint matches the selection state (empty → "click the object")', () => {
    expect(makeTool({ selection: [] }).tool.statusHint())
      .toBe('Click the object you want to rotate.')
    expect(makeTool().tool.statusHint()).toContain('center of rotation')
  })

  it('empty selection auto-selects via the injected acquirer and starts the gesture in one click', () => {
    const { tool, onToast, wasmScene, onCommit } = makeTool({ selection: [] })
    const acquire = vi.fn(() => [{ kind: 'object', id: 5n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot + auto-select
    expect(acquire).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true) // advanced past idle in the same click

    // The rest of the gesture proceeds on the acquired node.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep 90°
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit
    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith([{ kind: 'object', id: 5n }])
  })

  it('commits a rotation from a full three-click gesture (pivot → reference → sweep)', () => {
    const { tool, wasmScene, onCommit } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, 0°)
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep to +Y (90°)
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit

    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false) // reset to idle after commit
  })

  it('ignores a reference point coincident with the pivot', () => {
    const { tool, onMeasurement, wasmScene } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // ref == pivot → ignored
    // Entering the sweep stage sets the '0.0°' readout; if the coincident click
    // was ignored we never entered it.
    expect(onMeasurement).not.toHaveBeenCalledWith('0.0°')

    // A usable reference then still works — exactly one rotation commits.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // real reference
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // sweep 90°
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1)) // commit
    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
  })
})

describe('RotateTool — snapConstraint (axis-lock plane)', () => {
  it('is absent while idle, and absent while a pivot/ref stage has no lock engaged', () => {
    const { tool } = makeTool()
    // Idle — no lock, no pivot.
    expect(tool.snapConstraint()).toBeNull()

    // Pivot placed, still unlocked: axis is merely inferred (ground Z), so
    // it must stay free — an inferred axis can change on the next hover.
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    expect(tool.snapConstraint()).toBeNull()

    // Ref stage, still unlocked.
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    expect(tool.snapConstraint()).toBeNull()
  })

  it('is {pivot, lockedNormal} once an axis lock is engaged with a pivot placed', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 2, y: 3, z: 0 }), rayThrough(2, 3))
    tool.onPointerDown(makeSnap({ x: 2, y: 3, z: 0 }), rayThrough(2, 3)) // pivot at (2,3,0)
    tool.onKey(makeKeyEvent('ArrowRight')) // lock X

    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [2, 3, 0], normal: [1, 0, 0] },
    })

    // Still constrained once the reference is placed (ref stage).
    tool.onPointerDown(makeSnap({ x: 2, y: 3, z: 1 }), rayThrough(2, 3)) // reference (in the X-locked plane)
    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [2, 3, 0], normal: [1, 0, 0] },
    })
  })

  it('stores the reference point projected into the locked plane for an out-of-plane snap', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    tool.onKey(makeKeyEvent('ArrowRight')) // lock X — rotation plane is x=0 (Y-Z plane)

    // The resolved snap for the reference click carries a nonzero X (as if
    // an off-plane candidate slipped through despite the constraint) — the
    // tool must still store the reference point IN the x=0 plane, not at
    // this raw 3D point.
    tool.onPointerDown(makeSnap({ x: 5, y: 2, z: 3 }), rayThrough(2, 3))

    const stage = (tool as unknown as { stage: { kind: string; refPoint: [number, number, number] } }).stage
    expect(stage.kind).toBe('ref')
    expect(stage.refPoint).toEqual([0, 2, 3])
  })

  it('an arrow-lock change mid-gesture (ref stage) moves the constraint plane', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    tool.onKey(makeKeyEvent('ArrowRight')) // lock X
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(1, 0)) // reference, in-plane

    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [1, 0, 0] },
    })

    tool.onKey(makeKeyEvent('ArrowUp')) // relock to Z mid-sweep

    expect(tool.snapConstraint()).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [0, 0, 1] },
    })
  })
})

describe('RotateTool — locked sweep consumes the constrained snap', () => {
  it('the live sweep target is the resolved snap point (not a raw ray∩plane recompute) once locked', () => {
    const { tool, onMeasurement, wasmScene } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot at origin
    tool.onKey(makeKeyEvent('ArrowUp')) // lock Z — ground plane, matches the ray helper's plane
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, 0°)

    // Feed a snap whose (x, y, z) disagrees with what ray∩plane would compute
    // for this ray (the ray itself points straight through (0, 1, 0) on
    // z=0) — a snapped vertex sitting exactly at 90° but slightly off the
    // ray, as a sticky kernel candidate would be. The committed angle must
    // follow the SNAP, proving the tool consumed it rather than recomputing
    // ray∩plane and ignoring `snap`.
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0, kind: 'endpoint' }), rayThrough(0.3, 0.9))
    expect(onMeasurement).toHaveBeenCalledWith('Z 90.0°')

    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0, kind: 'endpoint' }), rayThrough(0.3, 0.9)) // commit
    expect(wasmScene.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('unlocked rotation keeps pure ray∩plane intersection (snap.xyz is ignored)', () => {
    const { tool, onMeasurement } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot (unlocked, inferred Z)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference (+X, 0°)

    // The snap's own (x,y,z) says 45°, but the ray itself intersects the
    // ground plane at (0,1,0) → 90°. Unlocked must follow the ray, not the
    // snap payload.
    tool.onPointerMove(makeSnap({ x: 0.70710678, y: 0.70710678, z: 0 }), rayThrough(0, 1))
    // "Z " tags the readout because the default INFERRED axis (no face/edge
    // hit) is world +Z, same as the locked case below — the tag reflects
    // world-alignment, not lock state. The 90° (not 45°) is the assertion
    // that matters: it comes from ray∩plane, proving `snap.xyz` was ignored.
    expect(onMeasurement).toHaveBeenCalledWith('Z 90.0°')
  })
})

describe('RotateTool — screen-constant disk scaling', () => {
  // The disk's screen size at the app's reference fov/viewport (45°, 720px
  // tall) — carried over from the old DISK_SCREEN_K = 0.06 constant so the
  // migration doesn't change how big the protractor looks at that baseline.
  const REF_FOV_DEG = 45
  const REF_VIEWPORT_H = 720
  const tanHalf = (fovDeg: number) => Math.tan((fovDeg * Math.PI) / 360)
  const expectedScale = (dist: number, fovDeg: number, viewportH: number) => {
    const desiredPixels = (0.06 * REF_VIEWPORT_H) / tanHalf(REF_FOV_DEG)
    return (desiredPixels * dist * tanHalf(fovDeg)) / viewportH
  }
  /** `updateDiskScale` now takes a `worldPerPixel` callback (the CameraRig
   * form, docs/design/camera.md §1) instead of a raw `(camera, viewportH)`
   * pair — this is what a real Viewport would build from `rig.worldPerPixel`
   * for a perspective camera at the given fov/viewport. */
  const wppPerspective = (fovDeg: number, viewportH: number) => (dist: number) =>
    worldPerPixelPerspective(dist, fovDeg, viewportH)

  it('updateDiskScale matches the old DISK_SCREEN_K * dist size at the reference fov/viewport', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))
    const disk = diskGroup(preview)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(1, 2, 10) // 10 m straight up from the disk center

    tool.updateDiskScale(camera, wppPerspective(REF_FOV_DEG, REF_VIEWPORT_H))

    const dist = camera.position.distanceTo(disk.position)
    expect(dist).toBeCloseTo(10, 9)
    const expected = expectedScale(dist, REF_FOV_DEG, REF_VIEWPORT_H)
    expect(expected).toBeCloseTo(0.06 * dist, 9) // old K * dist, sanity cross-check
    expect(disk.scale.x).toBeCloseTo(expected, 9)
  })

  it('holds its on-screen size across a FOV change, unlike the old K * dist form', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)

    for (const fov of [20, 45, 70, 100]) {
      const camera = new THREE.PerspectiveCamera(fov)
      camera.position.set(0, 0, 10)
      tool.updateDiskScale(camera, wppPerspective(fov, REF_VIEWPORT_H))
      const dist = camera.position.distanceTo(disk.position)
      expect(disk.scale.x).toBeCloseTo(expectedScale(dist, fov, REF_VIEWPORT_H), 9)
    }
  })

  it('holds its on-screen size across a viewport resize, unlike the old K * dist form', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)
    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)

    for (const viewportH of [400, 720, 1200]) {
      tool.updateDiskScale(camera, wppPerspective(REF_FOV_DEG, viewportH))
      const dist = camera.position.distanceTo(disk.position)
      expect(disk.scale.x).toBeCloseTo(expectedScale(dist, REF_FOV_DEG, viewportH), 9)
    }
  })

  it('is a no-op only when no disk is shown — a degenerate worldPerPixel still yields a defined (zero) size rather than leaving the old scale untouched', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)
    // A degenerate (zero) viewport height's worldPerPixel is 0 (see
    // worldPerPixelPerspective) — the disk collapses to size 0 deterministically,
    // rather than the old guard's "leave the previous scale untouched" (which
    // could leave an arbitrary stale size on screen).
    tool.updateDiskScale(camera, wppPerspective(REF_FOV_DEG, 0))
    expect(disk.scale.x).toBe(0)
  })

  it('DESIGN CHANGE: works under an orthographic (parallel-projection) camera too — no longer silently hidden', () => {
    // Before Phase 1, an `instanceof PerspectiveCamera` guard made this a
    // no-op under ortho — the protractor disk would silently vanish the
    // moment the user toggled Parallel Projection. `worldPerPixel` is
    // supplied by the caller (CameraRig.worldPerPixel under the hood), so the
    // tool itself no longer has (or needs) any projection-specific branch.
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)

    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 10)
    ortho.position.set(0, 0, 10)
    const frustumHeight = 4 // top(1) - bottom(-1), pre-scaled for this test
    const wpp = (_dist: number) => worldPerPixelOrtho(frustumHeight, 1, REF_VIEWPORT_H)
    tool.updateDiskScale(ortho, wpp)

    const expected = (0.06 * REF_VIEWPORT_H) / tanHalf(REF_FOV_DEG) * wpp(0) / 2
    expect(disk.scale.x).toBeCloseTo(expected, 9)
    expect(disk.scale.x).toBeGreaterThan(0)
  })
})

describe('RotateTool — setEditContext aborts an armed gesture on a genuine change (component-edit-parity.md phase A2)', () => {
  it('previews and commits a selected definition member through its active instance', () => {
    const instance = 77n
    const member = 9n
    const group = new THREE.Group()
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1))
    mesh.name = `InstanceFace_${instance}_${member}`
    group.add(mesh)
    const { tool, preview, wasmScene } = makeTool({
      selection: [{ kind: 'object', id: member }],
      instance: { id: instance, group },
    })
    ;(wasmScene as unknown as { transform_def_selection: ReturnType<typeof vi.fn> })
      .transform_def_selection = vi.fn()
    tool.setEditContext({ kind: 'instance', id: instance, component: 90n })
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0))
    tool.onPointerMove(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1))
    expect(preview.getObjectByName('InstanceMemberPreview')).toBeDefined()
    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), rayThrough(0, 1))
    expect(
      (wasmScene as unknown as { transform_def_selection: ReturnType<typeof vi.fn> })
        .transform_def_selection,
    ).toHaveBeenCalledTimes(1)
  })

  it('a genuine context change cancels an armed pivot/reference gesture instead of silently retargeting its eventual commit', () => {
    const { tool } = makeTool()
    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'top' })

    expect(tool.capturingInput()).toBe(false)
  })

  it('re-pushing the SAME context is a no-op — an armed gesture survives it untouched', () => {
    const { tool } = makeTool()
    const ctx = { kind: 'instance' as const, id: 9n, component: 90n }
    tool.setEditContext(ctx)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // pivot
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0)) // reference
    expect(tool.capturingInput()).toBe(true)

    tool.setEditContext({ kind: 'instance', id: 9n, component: 90n })

    expect(tool.capturingInput()).toBe(true)
  })
})
