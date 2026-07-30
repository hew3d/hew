/**
 * Ground-anchored hard Z lock — the canonical SketchUp gesture Kurt asked
 * for: "I need the ability to axis-lock at any point right up until
 * clicking the second point... it is the SketchUp way." Click a point on
 * the ground (the case every fresh drawing starts in) and press the up
 * arrow to draw straight up.
 *
 * `_planeCursor`'s ground fast path (see LineTool.ts's module doc and
 * `_planeCursor`'s own doc) used to force EVERY point back onto `z = 0`
 * regardless of any active lock — so a Z-locked point could never leave the
 * anchor: the segment always collapsed to zero length and the degenerate-
 * click guard in `_commitPlaneSegment` refused it, with the rubber-band
 * readout stuck at a formatted-zero length the whole time. This is the
 * ground-anchored counterpart of `LineTool.rehome.test.ts`'s off-plane
 * re-homing coverage — same machinery (`_rehomeChain`), reached from the one
 * anchor plane (`plane.ground === true`) that used to be excluded from it
 * entirely.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { formatLength } from '../settings/units'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(p: readonly [number, number, number]): Snap {
  return { x: p[0], y: p[1], z: p[2], kind: 'ground' }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

function makeWasmScene() {
  let sketchCounter = 40n
  const planes = new Map<bigint, [number, number, number, number, number, number]>()
  const segmentCalls: { sketch: bigint; a: [number, number, number]; b: [number, number, number] }[] = []
  const planeCalls: [number, number, number, number, number, number][] = []

  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      planes.set(sketchCounter, [0, 0, 0, 0, 0, 1])
      return sketchCounter
    }),
    begin_sketch_on_plane: vi.fn(
      (px: number, py: number, pz: number, nx: number, ny: number, nz: number) => {
        sketchCounter += 1n
        planeCalls.push([px, py, pz, nx, ny, nz])
        planes.set(sketchCounter, [px, py, pz, nx, ny, nz])
        return sketchCounter
      },
    ),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn((h: bigint) => {
      const p = planes.get(h)
      return p === undefined ? undefined : new Float64Array(p)
    }),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_add_segment: vi.fn(
      (sketch: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
        segmentCalls.push({ sketch, a: [ax, ay, az], b: [bx, by, bz] })
        return {
          new_edges: () => new BigUint64Array([]),
          regions_created: () => new BigUint64Array([]),
          regions_removed: () => new BigUint64Array([]),
          free: vi.fn(),
        }
      },
    ),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return { scene: scene as unknown as WasmScene, segmentCalls, planeCalls }
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, onCommit, onToast, onFaceImprint, onMeasurement }
}

describe('LineTool — ground-anchored hard Z lock (the up-arrow "draw straight up" gesture)', () => {
  it('a Z-locked hover away from the ground anchor reports a real, non-zero measurement instead of a stuck zero', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY) // first click: anchors on the ground
    tool.onKey(makeKeyEvent('ArrowUp')) // Z lock
    onMeasurement.mockClear()
    tool.onPointerMove(makeSnap([2, 3, 4]), RAY) // hover 4m straight up, still Z-locked

    expect(onMeasurement).toHaveBeenCalledWith(formatLength(4))
  })

  it('clicking straight up from a ground anchor while Z-locked commits a real vertical segment instead of refusing every click', () => {
    const { scene, segmentCalls, planeCalls } = makeWasmScene()
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY) // first click: ground anchor
    tool.onKey(makeKeyEvent('ArrowUp')) // Z lock
    tool.onPointerDown(makeSnap([2, 3, 4]), RAY) // second click: straight up

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segmentCalls).toHaveLength(1)
    expect(segmentCalls[0]).toEqual({ sketch: segmentCalls[0].sketch, a: [2, 3, 0], b: [2, 3, 4] })

    // The vertical segment leaves the ground plane, so it re-homes onto a
    // genuine (non-ground) plane through the anchor — same machinery as
    // `LineTool.rehome.test.ts`'s wall-to-wall case, just reached from the
    // ground for the first time. With no previous segment to span and a
    // straight-down view direction exactly antiparallel to the vertical
    // segment (the view-facing fallback degenerates too), `rehomePlaneNormal`
    // lands in its last-resort arbitrary-plane branch: normal (0, 1, 0),
    // any genuine vertical plane containing the drawn line.
    expect(planeCalls).toHaveLength(1)
    expect(planeCalls[0]).toEqual([2, 3, 0, 0, 1, 0])
  })

  it('an ordinary unlocked ground click is completely unaffected — same exact z = 0 coordinates as before', () => {
    const { scene, segmentCalls, planeCalls } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerDown(makeSnap([9, 3, 0]), RAY) // no lock — ordinary horizontal segment

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segmentCalls).toHaveLength(1)
    expect(segmentCalls[0]).toEqual({ sketch: segmentCalls[0].sketch, a: [2, 3, 0], b: [9, 3, 0] })
    expect(planeCalls).toHaveLength(0) // never re-homed — stayed on the shared ground sketch
  })
})
