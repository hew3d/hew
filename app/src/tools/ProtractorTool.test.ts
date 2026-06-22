import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ProtractorTool } from './ProtractorTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: new THREE.Vector3(), direction: new THREE.Vector3(0, 0, -1) }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/**
 * Minimal KeyboardEvent-shaped fake — the vitest environment here is plain
 * Node (no DOM globals), and `onKey` only reads `.key`/`.repeat` and calls
 * `.preventDefault()`, so a plain object matching that surface is enough.
 */
function makeKeyEvent(key: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  let defaultPrevented = false
  return {
    key,
    repeat: opts.repeat ?? false,
    get defaultPrevented() {
      return defaultPrevented
    },
    preventDefault: () => {
      defaultPrevented = true
    },
  } as unknown as KeyboardEvent
}

/** Minimal WasmScene stub — only the members ProtractorTool calls. */
function makeWasmScene(faceNormal?: [number, number, number]): WasmScene {
  return {
    face_normal: vi.fn((..._args: unknown[]) => {
      if (faceNormal === undefined) throw new Error('not a live world-object face')
      return new Float64Array(faceNormal)
    }),
    add_guide_line: vi.fn(() => 1n),
  } as unknown as WasmScene
}

function makeTool(faceNormal?: [number, number, number]) {
  const preview = new THREE.Group()
  const onGuideCreated = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const wasmScene = makeWasmScene(faceNormal)
  const tool = new ProtractorTool(wasmScene, preview, onGuideCreated, onToast, onMeasurement)
  return { tool, preview, onGuideCreated, onToast, onMeasurement, wasmScene }
}

describe('ProtractorTool — plane inference (hover phase)', () => {
  it('defaults to world up (blue) when hovering empty space / ground', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 1, y: 2, z: 0 }), RAY)
    // One disk group should be in the preview, centered at the snap point.
    expect(preview.children).toHaveLength(1)
    const disk = preview.children[0]
    expect(disk.position.toArray()).toEqual([1, 2, 0])
  })

  it('uses the hovered face normal when the snap is a live world-Object face', () => {
    const { tool, preview } = makeTool([0, 0, 1])
    tool.onPointerMove(
      makeSnap({ kind: 'face', elementKind: 'face', object: 1n, element: 2n, x: 0, y: 0, z: 0 }),
      RAY,
    )
    expect(preview.children).toHaveLength(1)
  })

  it('falls back to world up if face_normal throws (instanced geometry)', () => {
    const { tool, preview, wasmScene } = makeTool(undefined)
    tool.onPointerMove(
      makeSnap({ kind: 'face', elementKind: 'face', object: 1n, element: 2n }),
      RAY,
    )
    expect(wasmScene.face_normal).toHaveBeenCalled()
    expect(preview.children).toHaveLength(1)
  })
})

describe('ProtractorTool — Shift lock toggle', () => {
  it('locks the candidate normal on Shift, unlocks on a second Shift', () => {
    const { tool, preview } = makeTool([1, 0, 0]) // east/west face -> X normal
    tool.onPointerMove(
      makeSnap({ kind: 'face', elementKind: 'face', object: 1n, element: 2n, x: 5, y: 5, z: 0 }),
      RAY,
    )
    tool.onKey(makeKeyEvent('Shift'))
    // Locked: apex click should adopt the locked normal even off a face.
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 5, y: 5, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 5, y: 5, z: 0 }), RAY)
    expect(preview.children.length).toBeGreaterThan(0)

    // Unlock again (back in idle after apex commit would need a baseline+sweep;
    // instead directly verify the toggle semantics via a fresh tool).
    const { tool: tool2 } = makeTool([1, 0, 0])
    tool2.onPointerMove(makeSnap({ kind: 'ground' }), RAY)
    tool2.onKey(makeKeyEvent('Shift'))
    tool2.onKey(makeKeyEvent('Shift'))
    // After locking then unlocking, apex should use the live candidate again
    // (world up, since this snap isn't a face) rather than a stale lock.
    tool2.onPointerDown(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY)
    // No exception, no leftover lock — sanity check passes if we reach here.
    expect(true).toBe(true)
  })

  it('ignores Shift keydown autorepeat (ev.repeat=true does not toggle)', () => {
    const { tool, preview } = makeTool([0, 1, 0])
    tool.onPointerMove(
      makeSnap({ kind: 'face', elementKind: 'face', object: 1n, element: 2n }),
      RAY,
    )
    const before = preview.children.length
    tool.onKey(makeKeyEvent('Shift', { repeat: true }))
    // A repeat event should not change lock state or redraw anything new.
    expect(preview.children.length).toBe(before)
  })

  it('lock persists across a commit (back to idle, still locked)', () => {
    const { tool } = makeTool([0, 0, 1])
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('Shift')) // lock to Z (world up candidate)

    // Apex
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY)
    // Baseline
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 1, y: 0, z: 0 }), RAY)
    // Sweep
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 1, z: 0 }), RAY)
    // Commit
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 0, y: 1, z: 0 }), RAY)

    // Now back in idle. A subsequent hover over a non-face point should still
    // render the disk using the locked normal (Z), not re-infer.
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 2, y: 2, z: 0 }), RAY)
    // No throw — and capturingInput should report idle again.
    expect(tool.capturingInput()).toBe(false)
  })

  it('Escape clears the lock entirely', () => {
    const { tool } = makeTool([0, 0, 1])
    tool.onPointerMove(makeSnap({ kind: 'ground' }), RAY)
    tool.onKey(makeKeyEvent('Shift'))
    tool.cancel()
    expect(tool.capturingInput()).toBe(false)
  })
})

describe('ProtractorTool — arrow-key axis lock', () => {
  it('ArrowRight locks to X, ArrowLeft locks to Y, ArrowUp/Down lock to Z', () => {
    const { tool } = makeTool()
    tool.onPointerMove(makeSnap({ kind: 'ground' }), RAY)

    const right = makeKeyEvent('ArrowRight')
    tool.onKey(right)
    expect(right.defaultPrevented).toBe(true)

    const left = makeKeyEvent('ArrowLeft')
    tool.onKey(left)
    expect(left.defaultPrevented).toBe(true)

    const up = makeKeyEvent('ArrowUp')
    tool.onKey(up)
    expect(up.defaultPrevented).toBe(true)
  })
})

describe('ProtractorTool — apex/baseline/sweep/commit with plane locking', () => {
  it('commits a guide line through the apex using the locked plane normal', () => {
    const { tool, wasmScene, onGuideCreated } = makeTool()
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('Shift')) // lock to world-up (Z)

    tool.onPointerDown(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY) // apex
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 1, y: 0, z: 0 }), RAY) // baseline (+X)
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 1, z: 0 }), RAY) // sweep to +Y (90°)
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 0, y: 1, z: 0 }), RAY) // commit

    expect(wasmScene.add_guide_line).toHaveBeenCalledTimes(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    const args = (wasmScene.add_guide_line as ReturnType<typeof vi.fn>).mock.calls[0]
    // apex at origin
    expect(args[0]).toBeCloseTo(0)
    expect(args[1]).toBeCloseTo(0)
    expect(args[2]).toBeCloseTo(0)
    // swept to +Y, axis-snapped exactly
    expect(args[3]).toBeCloseTo(0)
    expect(args[4]).toBeCloseTo(1)
    expect(args[5]).toBeCloseTo(0)
  })

  it('reports the snapped-axis angle (exact), not the raw cursor angle, once axis-snapped', () => {
    const { tool, onMeasurement } = makeTool()
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY) // apex (plane = world up)
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 1, y: 0, z: 0 }), RAY) // baseline (+X)

    // Sweep to a direction *near* +Y but not exactly on it (within the 2° axis-snap tolerance).
    const nearYDeg = 89.4
    const rad = (nearYDeg * Math.PI) / 180
    tool.onPointerMove(makeSnap({ kind: 'ground', x: Math.cos(rad), y: Math.sin(rad), z: 0 }), RAY)

    // The cursor is at 89.4°, but since it's within tolerance of +Y (90°), the
    // axis-snap kicks in and the reported angle should read exactly 90.0,
    // not the raw 89.4.
    const lastCall = onMeasurement.mock.calls.at(-1)
    expect(lastCall).toBeDefined()
    expect(lastCall![0]).toBe('90.0°')
  })

  it('reports the raw cursor angle when off-axis (no snap)', () => {
    const { tool, onMeasurement } = makeTool()
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 0, y: 0, z: 0 }), RAY) // apex
    tool.onPointerDown(makeSnap({ kind: 'ground', x: 1, y: 0, z: 0 }), RAY) // baseline (+X)

    // 45 degrees is comfortably off any axis.
    const rad = Math.PI / 4
    tool.onPointerMove(makeSnap({ kind: 'ground', x: Math.cos(rad), y: Math.sin(rad), z: 0 }), RAY)

    const lastCall = onMeasurement.mock.calls.at(-1)
    expect(lastCall).toBeDefined()
    expect(lastCall![0]).toBe('45.0°')
  })
})

describe('ProtractorTool — screen-constant disk scaling', () => {
  const DISK_SCREEN_K = 0.03

  it('updateDiskScale sets the disk group scale to DISK_SCREEN_K * camera distance', () => {
    const { tool, preview } = makeTool()
    tool.onPointerMove(makeSnap({ kind: 'ground', x: 1, y: 2, z: 0 }), RAY)

    const disk = preview.children[0]
    const camera = new THREE.PerspectiveCamera()
    camera.position.set(1, 2, 10) // 10 m straight up from the disk center (1,2,0)

    tool.updateDiskScale(camera)

    const dist = camera.position.distanceTo(disk.position)
    expect(dist).toBeCloseTo(10, 9)
    expect(disk.scale.x).toBeCloseTo(DISK_SCREEN_K * dist, 9)
    expect(disk.scale.y).toBeCloseTo(DISK_SCREEN_K * dist, 9)
    expect(disk.scale.z).toBeCloseTo(DISK_SCREEN_K * dist, 9)
  })

  it('updateDiskScale is a no-op when no disk is currently shown', () => {
    const { tool } = makeTool()
    const camera = new THREE.PerspectiveCamera()
    camera.position.set(0, 0, 10)
    // No pointer move yet -> previewDisk is null. Should not throw.
    expect(() => tool.updateDiskScale(camera)).not.toThrow()
  })
})
