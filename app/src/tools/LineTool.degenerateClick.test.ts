/**
 * A click that resolves back onto the anchor must never be silently
 * discarded (tool-parity playtest-2 defect: a soft-axis/lock inference
 * candidate that collapses onto `anchor` — e.g. the wrong world axis
 * spuriously winning the ranking — used to make `_commitPlaneSegment`
 * `return` with NO toast, no error, and no visible feedback at all: the
 * sketch's segment count simply didn't change and nothing told the user
 * why. This drives LineTool's real commit path (not a mocked snap result)
 * with a snap that lands exactly back on the anchor — the actual shape the
 * kernel-level defect produces — and asserts the click is refused
 * AUDIBLY/VISIBLY (`onToast`), not silently.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(p: readonly [number, number, number], kind: string = 'ground'): Snap {
  return { x: p[0], y: p[1], z: p[2], kind }
}

function makeWasmScene() {
  let sketchCounter = 10n
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
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
  return { scene: scene as unknown as WasmScene, addSegment: scene.sketch_add_segment }
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

/** A plain (ungrouped, non-instanced) top-level object's face, always under the ray. */
function makeFaceWasmScene() {
  const pick = { object: () => 7n, face: () => 3n, instance: () => undefined, free: vi.fn() }
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    pick_face: vi.fn(() => pick),
    pick_sketch: vi.fn(() => undefined),
    node_parent: vi.fn(() => undefined), // top-level, not grouped
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    split_face: vi.fn(() => ({ free: vi.fn() })),
    split_face_in_instance: vi.fn(() => ({ free: vi.fn() })),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return { scene: scene as unknown as WasmScene, splitFace: scene.split_face }
}

describe('LineTool — a click that resolves back onto the anchor is refused AUDIBLY, never silently', () => {
  it('ground plane: a second click whose snap collapses onto the anchor toasts and stays anchored, instead of vanishing with no feedback', () => {
    const { scene, addSegment } = makeWasmScene()
    const { tool, onToast, onCommit } = makeTool(scene)

    // First click: anchors the chain at (2, 3, 0).
    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    expect(tool.capturingInput()).toBe(true)

    // Second click: this is the EXACT shape the reported defect produces —
    // an inference candidate (e.g. the wrong axis spuriously winning the
    // ranking between two intended ones) whose resolved x/y coincide with
    // the anchor's, even though the user visibly moved the cursor and
    // expected a real segment. `_planeCursor`'s ground fast path reduces
    // this snap to exactly the anchor.
    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)

    // The click must not silently vanish: no kernel segment (correct — it
    // really is degenerate) BUT the user must be told, audibly/visibly.
    expect(addSegment).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/move the cursor|same as the last/i)

    // The chain stays anchored — the user can just try again, exactly like
    // any other refused-but-recoverable gesture in this tool.
    expect(tool.capturingInput()).toBe(true)
  })

  it('a genuinely NEW point still commits normally (the toast is not a general trip-wire)', () => {
    const { scene, addSegment } = makeWasmScene()
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerDown(makeSnap([9, 3, 0]), RAY)

    expect(addSegment).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })

  // Delta-review finding 2: `_appendFacePoint` had the identical
  // collapse-to-the-last-point guard as `_commitPlaneSegment` above, but
  // (unlike it) discarded the click with no toast at all — a click on a
  // face could vanish just as silently as the original plane-mode defect.
  it('face mode: a second click on the same point toasts and stays anchored, instead of vanishing with no feedback', () => {
    const { scene, splitFace } = makeFaceWasmScene()
    const { tool, onToast, onFaceImprint } = makeTool(scene)

    // First click: anchors a face chain at (0, 0, 1) — see `makeFaceWasmScene`.
    tool.onPointerDown(makeSnap([0, 0, 1], 'face'), RAY)
    expect(tool.capturingInput()).toBe(true)

    // Second click resolves to the SAME point as the anchor — the face-mode
    // counterpart of the plane-mode repro above (e.g. a snap candidate that
    // collapses back onto the last placed point).
    tool.onPointerDown(makeSnap([0, 0, 1], 'face'), RAY)

    // Refused, but AUDIBLY: no cut committed, but the user is told why.
    expect(splitFace).not.toHaveBeenCalled()
    expect(onFaceImprint).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/move the cursor|same as the last/i)

    // The chain stays anchored — same recoverable shape as plane mode.
    expect(tool.capturingInput()).toBe(true)
  })
})
