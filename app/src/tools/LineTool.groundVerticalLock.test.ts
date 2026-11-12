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
import { draftMock } from './draftMock'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(p: readonly [number, number, number]): Snap {
  return { x: p[0], y: p[1], z: p[2], kind: 'ground' }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

function makeWasmScene() {
  const dm = draftMock()
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    ...dm.members,
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  // Every draft is one `draft_begin_plane`; `planeCalls` is its [origin, normal] args.
  return {
    scene: scene as unknown as WasmScene,
    segmentCalls: dm.log.segments,
    get planeCalls() {
      return dm.log.begins.map((b) => b.args.slice(0, 6) as number[])
    },
    log: dm.log,
  }
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
    const w = makeWasmScene()
    const { scene, segmentCalls } = w
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY) // first click: ground anchor
    tool.onKey(makeKeyEvent('ArrowUp')) // Z lock
    tool.onPointerDown(makeSnap([2, 3, 4]), RAY) // second click: straight up

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segmentCalls).toHaveLength(1)
    expect(segmentCalls[0]).toEqual([2, 3, 0, 2, 3, 4])

    // The vertical segment leaves the ground plane, so it re-homes onto a
    // genuine (non-ground) plane through the anchor — same machinery as
    // `LineTool.rehome.test.ts`'s wall-to-wall case, just reached from the
    // ground for the first time. With no previous segment to span and a
    // straight-down view direction exactly antiparallel to the vertical
    // segment (the view-facing fallback degenerates too), `rehomePlaneNormal`
    // lands in its last-resort arbitrary-plane branch: normal (0, 1, 0),
    // any genuine vertical plane containing the drawn line.
    // (One draft, begun directly on the re-homed plane.)
    expect(w.planeCalls).toHaveLength(1)
    expect(w.planeCalls[0]).toEqual([2, 3, 0, 0, 1, 0])
  })

  it('an ordinary unlocked ground click is completely unaffected — same exact z = 0 coordinates as before', () => {
    const w = makeWasmScene()
    const { scene, segmentCalls } = w
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerDown(makeSnap([9, 3, 0]), RAY) // no lock — ordinary horizontal segment

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segmentCalls).toHaveLength(1)
    expect(segmentCalls[0]).toEqual([2, 3, 0, 9, 3, 0])
    // never re-homed — the one draft is on the ground plane
    expect(w.planeCalls).toEqual([[0, 0, 0, 0, 0, 1]])
  })
})

/** A Blue-axis inference straight above (or below) a ground anchor, as the
 *  unconstrained ground-anchored snap resolves it: the full 3D point. */
function blueSnap(p: readonly [number, number, number]): Snap {
  return { x: p[0], y: p[1], z: p[2], kind: 'on-axis', direction: [0, 0, 1] }
}

function typeLength(tool: LineTool, text: string): void {
  for (const ch of text) tool.onKey(makeKeyEvent(ch))
  tool.onKey(makeKeyEvent('Enter'))
}

describe('LineTool — typed length with the cursor on the Blue axis (#51a)', () => {
  it('a typed length while hovering a Blue-axis snap above a ground anchor draws a vertical segment of that length', () => {
    const w = makeWasmScene()
    const { tool, onToast, onCommit } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY) // ground anchor
    tool.onPointerMove(blueSnap([2, 3, 4]), RAY) // on Blue, straight up, no arrow lock
    typeLength(tool, '1.5')

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(w.segmentCalls).toEqual([[2, 3, 0, 2, 3, 1.5]])
    // Re-homed exactly like the arrow-locked riser: one draft, begun on a
    // genuine vertical plane through the anchor (see the Z-lock test above
    // for why the normal is (0, 1, 0) under a straight-down view).
    expect(w.planeCalls).toEqual([[2, 3, 0, 0, 1, 0]])
  })

  it('the chain stays on the vertical plane it re-homed onto after the typed riser', () => {
    const w = makeWasmScene()
    const { tool, onToast } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerMove(blueSnap([2, 3, 4]), RAY)
    typeLength(tool, '1.5')
    // Next segment across the top of the riser, in that same vertical plane.
    tool.onPointerDown({ x: 3, y: 3, z: 1.5, kind: 'free' }, RAY)

    expect(onToast).not.toHaveBeenCalled()
    expect(w.segmentCalls).toEqual([[2, 3, 0, 2, 3, 1.5], [2, 3, 1.5, 3, 3, 1.5]])
    expect(w.planeCalls).toEqual([[2, 3, 0, 0, 1, 0], [2, 3, 0, 0, 1, 0]])
  })

  it('a Blue-axis snap straight BELOW the ground anchor draws the typed length downward', () => {
    const w = makeWasmScene()
    const { tool, onToast } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerMove(blueSnap([2, 3, -4]), RAY)
    typeLength(tool, '1.5')

    expect(onToast).not.toHaveBeenCalled()
    expect(w.segmentCalls).toEqual([[2, 3, 0, 2, 3, -1.5]])
  })

  it('a typed length with no direction at all says so instead of doing nothing', () => {
    const w = makeWasmScene()
    const { tool, onToast, onCommit } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY) // anchored, cursor never moved off it
    typeLength(tool, '1.5')

    expect(onCommit).not.toHaveBeenCalled()
    expect(w.segmentCalls).toHaveLength(0)
    expect(onToast).toHaveBeenCalledWith('Move the cursor to point the line, then type the length.')
  })

  it('a hover back onto the anchor itself (no inference) also says so', () => {
    const w = makeWasmScene()
    const { tool, onToast } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerMove(makeSnap([2, 3, 0]), RAY)
    typeLength(tool, '1.5')

    expect(w.segmentCalls).toHaveLength(0)
    expect(onToast).toHaveBeenCalledWith('Move the cursor to point the line, then type the length.')
  })

  it('a plain click on a Blue-axis snap above the ground anchor commits the vertical segment it points at', () => {
    const w = makeWasmScene()
    const { tool, onToast, onCommit } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerMove(blueSnap([2, 3, 4]), RAY)
    tool.onPointerDown(blueSnap([2, 3, 4]), RAY)

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(w.segmentCalls).toEqual([[2, 3, 0, 2, 3, 4]])
    expect(w.planeCalls).toEqual([[2, 3, 0, 0, 1, 0]])
  })

  it('hovering the Blue axis shows the real vertical length in the measurement box', () => {
    const w = makeWasmScene()
    const { tool, onMeasurement } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    onMeasurement.mockClear()
    tool.onPointerMove(blueSnap([2, 3, 4]), RAY)

    expect(onMeasurement).toHaveBeenCalledWith(formatLength(4))
    expect(tool.snapProjected()).toBe(false)
  })

  it('an ordinary off-axis snap with height (not a named point) is still flattened onto the ground and reported as projected', () => {
    const w = makeWasmScene()
    const { tool } = makeTool(w.scene)

    tool.onPointerDown(makeSnap([2, 3, 0]), RAY)
    tool.onPointerMove({ x: 5, y: 3, z: 2, kind: 'on-edge' }, RAY)
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown({ x: 5, y: 3, z: 2, kind: 'on-edge' }, RAY)

    expect(w.segmentCalls).toEqual([[2, 3, 0, 5, 3, 0]])
    expect(w.planeCalls).toEqual([[0, 0, 0, 0, 0, 1]])
  })
})
