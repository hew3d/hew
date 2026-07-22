import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RotateTool } from './RotateTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'

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

function makeTool(opts: { faceNormal?: [number, number, number]; selection?: NodeRef[] } = {}) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const wasmScene = makeWasmScene(opts.faceNormal)
  const selection: NodeRef[] = opts.selection ?? [{ kind: 'object', id: 1n }]
  const tool = new RotateTool(
    wasmScene as never,
    preview,
    null, // objectsGroup — null means no ghost mesh is cloned (fine for logic tests)
    selection,
    onCommit,
    onToast,
    null,
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

  it('updateDiskScale matches the old DISK_SCREEN_K * dist size at the reference fov/viewport', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))
    const disk = diskGroup(preview)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(1, 2, 10) // 10 m straight up from the disk center

    tool.updateDiskScale(camera, REF_VIEWPORT_H)

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
      tool.updateDiskScale(camera, REF_VIEWPORT_H)
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
      tool.updateDiskScale(camera, viewportH)
      const dist = camera.position.distanceTo(disk.position)
      expect(disk.scale.x).toBeCloseTo(expectedScale(dist, REF_FOV_DEG, viewportH), 9)
    }
  })

  it('is a no-op for a non-perspective camera or a degenerate viewport height', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    const disk = diskGroup(preview)
    const before = disk.scale.x

    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 10)
    ortho.position.set(0, 0, 10)
    tool.updateDiskScale(ortho, REF_VIEWPORT_H)
    expect(disk.scale.x).toBe(before)

    const camera = new THREE.PerspectiveCamera(REF_FOV_DEG)
    camera.position.set(0, 0, 10)
    tool.updateDiskScale(camera, 0)
    expect(disk.scale.x).toBe(before)
  })
})
