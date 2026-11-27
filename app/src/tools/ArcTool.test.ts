/**
 * ArcTool unit tests — mirrors CircleTool.test.ts' fake-WasmScene
 * pattern: drive the 3-click gesture (A, B, bulge) and assert the exact
 * kernel calls the tool commits (one draft per commit, unified drafting).
 */
import { describe, it, expect, vi, beforeAll, afterAll, beforeEach, afterEach } from 'vitest'
import * as THREE from 'three'
import { ArcTool } from './ArcTool'
import { draftMock } from './draftMock'
import { arcFromChord, arcSegmentCount } from './arcMath'
import type { Snap } from './types'
import { setLengthUnit, getLengthUnit, parseLengthToMeters, setPrecision, DEFAULT_PRECISION } from '../settings/units'
import { sagittaForDisplayedRadius } from './displayedValue'
import type { Scene as WasmScene } from '../wasm/loader'
import { setPickRenderMode } from './pickPolicy'
import type { Ray } from '../viewport/math'

// A ray straight down the -Z axis from above the origin (tuple-shaped, as the
// real Viewport supplies — the tool indexes ray.origin[0..2]).
const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeKeyEvent(key: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  return { key, repeat: opts.repeat ?? false, preventDefault: () => {} } as unknown as KeyboardEvent
}

/** Feed each character of `s` through `onKey` (digits/'.'/'-' only — enough
 *  for the VCB tests below), mirroring how the Viewport forwards keydown. */
function typeDigits(tool: ArcTool, s: string): void {
  for (const ch of s) tool.onKey(makeKeyEvent(ch))
}

/** A fake `FacePickJs` returning the seeded handles. */
function makePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    depth: () => 1,
    free: vi.fn(),
  }
}

type SegmentCall = [number, number, number, number, number, number]

/** The loop a face/plane draft carried: the START point of every segment, as
 *  a flat xyz Float64Array (a closed loop closes implicitly back to the
 *  first point). */
function loopOf(segments: SegmentCall[]): Float64Array {
  return new Float64Array(segments.flatMap((s) => [s[0], s[1], s[2]]))
}

/** The open path a draft carried: every segment start plus the last segment's
 *  end). */
function pathOf(segments: SegmentCall[]): Float64Array {
  const last = segments[segments.length - 1]
  return new Float64Array([...loopOf(segments), last[3], last[4], last[5]])
}

/**
 * Minimal WasmScene stub — only the members ArcTool calls. Every commit is
 * one draft (plane or face); `segments` records every `draft_add_segment`
 * endpoint pair across the mock's life.
 */
function makeWasmScene(opts: {
  pick?: ReturnType<typeof makePick>
  facePlane?: [number, number, number, number, number, number]
  faceNormal?: [number, number, number]
  addSegmentThrows?: boolean
  splitFaceThrows?: boolean
} = {}) {
  const segments: SegmentCall[] = []
  const fail = opts.addSegmentThrows
    ? new Error('PathNotSimple: edges cross')
    : opts.splitFaceThrows
      ? new Error('BadLoop: invalid cut path')
      : null
  const dm = draftMock({ failSegmentAt: () => fail })
  const baseAdd = dm.members.draft_add_segment.getMockImplementation()!
  dm.members.draft_add_segment.mockImplementation((d, ax, ay, az, bx, by, bz) => {
    const r = baseAdd(d, ax, ay, az, bx, by, bz)
    segments.push([ax, ay, az, bx, by, bz])
    return r
  })
  const scene = {
    history_generation: vi.fn(() => 1n),
    // Every picked object is plain/ungrouped by default (top-level eligibility).
    node_parent: vi.fn(() => undefined),
    // Every sketch lies on the ground plane (origin point, +Z).
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    ...dm.members,
    pick_face: vi.fn(() => opts.pick),
    pick_sketch: vi.fn(() => undefined), // no committed sketches in these fixtures
    face_normal: vi.fn(() => new Float64Array(opts.faceNormal ?? [0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array(opts.facePlane ?? [0, 0, 1, 0, 0, 1])),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])), // translated +5 in x
  }
  return { scene: scene as unknown as WasmScene, segments, dm }
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new ArcTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement }
}

describe('ArcTool — ground mode', () => {
  it('three clicks (A, B, bulge) commit a connected open chain with exact endpoints', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)   // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)   // B
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY) // pull the bulge
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY) // commit

    const arc = arcFromChord([0, 0], [2, 0], 0.5)!
    const expectSegs = arcSegmentCount(arc.sweep, arc.radius)
    expect(segments.length).toBe(expectSegs)

    // Exact endpoints — no float drift at the chain's ends.
    expect(segments[0][0]).toBe(0)
    expect(segments[0][1]).toBe(0)
    expect(segments[segments.length - 1][3]).toBe(2)
    expect(segments[segments.length - 1][4]).toBe(0)

    // The chain is connected: each segment starts where the previous ended.
    for (let i = 1; i < segments.length; i++) {
      expect(segments[i][0]).toBe(segments[i - 1][3])
      expect(segments[i][1]).toBe(segments[i - 1][4])
      expect(segments[i][2]).toBe(segments[i - 1][5])
    }

    // It is an OPEN chain — last endpoint differs from the first.
    expect(segments[segments.length - 1][3]).not.toBe(segments[0][0])

    // All vertices lie on the bulge side (y >= 0) and on the ground plane.
    for (const s of segments) {
      expect(s[1]).toBeGreaterThanOrEqual(0)
      expect(s[4]).toBeGreaterThanOrEqual(0)
      expect(s[2]).toBe(0)
      expect(s[5]).toBe(0)
    }

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
    // The whole faceted-chain commit is exactly one draft.
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    expect(scene.draft_end).toHaveBeenCalledTimes(1)
  })

  it('there is no cached sketch handle to go stale: a commit opens one fresh plane draft on the ground', () => {
    const { scene, segments } = makeWasmScene()
    const onCommit = vi.fn()
    const onToast = vi.fn()
    const tool = new ArcTool(scene, new THREE.Group(), onCommit, onToast, vi.fn(), vi.fn())

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)   // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)   // B
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY) // commit

    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    expect(scene.draft_end).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segments.length).toBeGreaterThan(0)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('refuses to commit a flat bulge (|sagitta| below tolerance) and hints instead', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY) // B
    tool.onPointerDown(makeSnap({ x: 1, y: 0 }), RAY) // collinear "bulge" — refused

    expect(segments.length).toBe(0)
    expect(onCommit).not.toHaveBeenCalled()
    expect(onMeasurement).toHaveBeenLastCalledWith('Pull out the bulge')

    // A real bulge afterwards still commits (the gesture was not aborted).
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('ignores a degenerate B click (zero-length chord)', () => {
    const { scene } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY) // B on A — ignored
    // Still waiting for B: this click becomes B, not a bulge commit.
    tool.onPointerDown(makeSnap({ x: 3, y: 1 }), RAY)
    expect(onCommit).not.toHaveBeenCalled()
    // And now the bulge click commits.
    tool.onPointerDown(makeSnap({ x: 2, y: 2 }), RAY)
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('Escape steps back one stage at a time (bulge → chord → idle)', () => {
    const { scene } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    expect(tool.capturingInput()).toBe(false)
    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY) // B
    expect(tool.capturingInput()).toBe(true)

    tool.onKey(makeKeyEvent('Escape')) // bulge → chord (A kept)
    expect(tool.capturingInput()).toBe(true)

    // The next click is a NEW B, then a bulge click commits — proving the
    // stage went back to "waiting for B" with A preserved, not to idle.
    tool.onPointerDown(makeSnap({ x: 0, y: 2 }), RAY)  // new B
    tool.onPointerDown(makeSnap({ x: -0.5, y: 1 }), RAY) // bulge → commit
    expect(onCommit).toHaveBeenCalledTimes(1)

    tool.onKey(makeKeyEvent('Escape')) // idle → full cancel (no-op)
    expect(tool.capturingInput()).toBe(false)
  })

  it('each commit is its own draft on the same ground plane (no shared sketch cache)', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY)

    tool.onPointerDown(makeSnap({ x: 4, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 6, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 5, y: 1 }), RAY)

    const begins = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls
    expect(begins).toHaveLength(2)
    for (const b of begins) expect(b.slice(0, 6)).toEqual([0, 0, 0, 0, 0, 1])
    expect(scene.draft_end).toHaveBeenCalledTimes(2)
  })

  it('onDocumentReset leaves nothing stale: the next commit opens a fresh plane draft', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    const beginSketch = scene.draft_begin_plane as ReturnType<typeof vi.fn>

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY)
    expect(beginSketch).toHaveBeenCalledTimes(1)

    tool.onDocumentReset()

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY)
    expect(beginSketch).toHaveBeenCalledTimes(2)
  })

  it('surfaces a kernel error from draft_add_segment as a toast', () => {
    const { scene } = makeWasmScene({ addSegmentThrows: true })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY)

    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('PathNotSimple')
    // The refused draft is cancelled, never ended.
    expect(scene.draft_cancel).toHaveBeenCalledTimes(1)
    expect(scene.draft_end).not.toHaveBeenCalled()
  })

  it('reports the radius through onMeasurement while pulling the bulge', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 1 }), RAY) // semicircle: r = 1 m

    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last.startsWith('R ')).toBe(true)
    expect(last).toContain('1') // 1 m in the default unit format
  })
})

describe('ArcTool — completion modes (Alt cycles open → pie → segment)', () => {
  // Shared geometry: A=(0,0), B=(2,0), bulge (1, 0.5) → s=0.5, r=1.25,
  // center=(1, −0.75) (on the side opposite the bulge).
  const ARC = arcFromChord([0, 0], [2, 0], 0.5)!
  const ARC_SEGS = arcSegmentCount(ARC.sweep, ARC.radius)

  function drawToBulge(tool: ArcTool): void {
    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)   // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)   // B
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY) // pull the bulge
  }

  it('pie (one Alt) closes B → center → A: two extra segments, exact wrap to A', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    drawToBulge(tool)
    tool.onKey(makeKeyEvent('Alt'))                     // open → pie
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY) // commit

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segments.length).toBe(ARC_SEGS + 2)

    // B → center.
    expect(segments[ARC_SEGS][0]).toBe(2)
    expect(segments[ARC_SEGS][1]).toBe(0)
    expect(segments[ARC_SEGS][3]).toBeCloseTo(ARC.center[0], 12)
    expect(segments[ARC_SEGS][4]).toBeCloseTo(ARC.center[1], 12)
    // center → A, closing exactly on the first vertex.
    const last = segments[segments.length - 1]
    expect(last[3]).toBe(segments[0][0])
    expect(last[4]).toBe(segments[0][1])
    expect(last[5]).toBe(segments[0][2])

    // Arc AND closing segments land in ONE draft — a single undo step.
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    expect(scene.draft_end).toHaveBeenCalledTimes(1)
  })

  it('segment (two Alts) closes with the chord B → A as one extra segment', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    drawToBulge(tool)
    tool.onKey(makeKeyEvent('Alt')) // open → pie
    tool.onKey(makeKeyEvent('Alt')) // pie → segment
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segments.length).toBe(ARC_SEGS + 1)
    const last = segments[segments.length - 1]
    expect([last[0], last[1], last[2]]).toEqual([2, 0, 0])
    expect([last[3], last[4], last[5]]).toEqual([0, 0, 0])
  })

  it('a third Alt wraps back to an open arc', () => {
    const { scene, segments } = makeWasmScene()
    const { tool } = makeTool(scene)

    drawToBulge(tool)
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt'))
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)

    expect(segments.length).toBe(ARC_SEGS)
    const last = segments[segments.length - 1]
    expect(last[3]).not.toBe(segments[0][0]) // still an OPEN chain
  })

  it('Alt autorepeat does not spin the cycle', () => {
    const { scene, segments } = makeWasmScene()
    const { tool } = makeTool(scene)

    drawToBulge(tool)
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt', { repeat: true })) // held key — ignored
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)

    expect(segments.length).toBe(ARC_SEGS + 2) // still pie, not segment
  })

  it('Alt outside a gesture is ignored (a fresh arc still commits open)', () => {
    const { scene, segments } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('Alt')) // idle — not capturing, ignored
    drawToBulge(tool)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)

    expect(segments.length).toBe(ARC_SEGS)
  })

  it('the mode persists across commits within the tool activation', () => {
    const { scene, segments } = makeWasmScene()
    const { tool } = makeTool(scene)

    drawToBulge(tool)
    tool.onKey(makeKeyEvent('Alt'))
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)
    expect(segments.length).toBe(ARC_SEGS + 2)

    tool.onPointerDown(makeSnap({ x: 4, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 6, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 5, y: 0.5 }), RAY)
    expect(segments.length).toBe(2 * (ARC_SEGS + 2)) // second arc closed too
  })

  it('Alt while a length is typed keeps the buffer visible, with the mode as suffix', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    drawToBulge(tool)
    typeDigits(tool, '0.5')
    tool.onKey(makeKeyEvent('Alt')) // open → pie — must not hide the buffer

    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last).toContain('0.5')
    expect(last).toContain('· Pie')
  })

  it('a typed bulge commit closes with the current mode too (shared commit path)', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    drawToBulge(tool)                                   // cursor at (1, 0.5) — side established
    tool.onKey(makeKeyEvent('Alt'))                     // open → pie
    typeDigits(tool, '0.5')
    tool.onKey(makeKeyEvent('Enter'))                   // |sagitta| = 0.5 on the +y side

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(segments.length).toBe(ARC_SEGS + 2)          // pie close via the typed path
    const last = segments[segments.length - 1]
    expect(last[3]).toBe(segments[0][0])
    expect(last[4]).toBe(segments[0][1])
  })

  it('onDocumentReset restores the default open mode', () => {
    const { scene, segments } = makeWasmScene()
    const { tool } = makeTool(scene)

    drawToBulge(tool)
    tool.onKey(makeKeyEvent('Alt'))                     // open → pie
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)
    expect(segments.length).toBe(ARC_SEGS + 2)

    tool.onDocumentReset()                              // new/loaded document

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)
    expect(segments.length).toBe(2 * ARC_SEGS + 2)      // second arc committed open
  })

  it('the radius readout names the mode while a closed preview is live', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    drawToBulge(tool)
    tool.onKey(makeKeyEvent('Alt'))
    expect(onMeasurement).toHaveBeenLastCalledWith('Pie')

    // The Viewport re-fires onPointerMove after onKey; the full readout
    // then carries the mode suffix.
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY)
    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last.startsWith('R ')).toBe(true)
    expect(last).toContain('· Pie')
  })

  it('face mode: pie draws a closed loop in one face draft with the center appended', () => {
    const pick = makePick(7n, 3n)
    const { scene, segments } = makeWasmScene({ pick, facePlane: [0, 0, 1, 0, 0, 1], faceNormal: [0, 0, 1] })
    const { tool, onFaceImprint } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)  // A
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 1 }), RAY)  // B
    tool.onPointerMove(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)  // bulge toward −y
    tool.onKey(makeKeyEvent('Alt'))                            // open → pie
    tool.onPointerDown(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)  // commit

    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1) // ONE draft
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
    expect(onFaceImprint).toHaveBeenCalledWith(7n)

    // The loop is the segments' start points (the last segment runs center → A).
    const loop = loopOf(segments)
    const nPts = loop.length / 3
    // First point is exactly A; the appended last point is the arc center —
    // on the face plane, and past the chord on the side opposite the bulge.
    expect(loop[0]).toBe(0.5)
    expect(loop[1]).toBe(2)
    expect(loop[(nPts - 1) * 3 + 2]).toBeCloseTo(1, 10)
    expect(loop[(nPts - 1) * 3 + 1]).toBeGreaterThan(2)
  })

  it('face mode: segment draws the arc plus the chord back to A in one face draft', () => {
    const pick = makePick(7n, 3n)
    const { scene, segments } = makeWasmScene({ pick, facePlane: [0, 0, 1, 0, 0, 1], faceNormal: [0, 0, 1] })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 1 }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)
    tool.onKey(makeKeyEvent('Alt'))
    tool.onKey(makeKeyEvent('Alt')) // pie → segment
    tool.onPointerDown(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)

    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1)

    // The loop (segment starts) is the arc itself, A…B — the last segment is
    // the explicit chord B → A.
    const loop = loopOf(segments)
    const nPts = loop.length / 3
    expect(loop[0]).toBe(0.5)
    expect(loop[1]).toBe(2)
    expect(loop[(nPts - 1) * 3 + 0]).toBe(1.5)
    expect(loop[(nPts - 1) * 3 + 1]).toBe(2)
  })
})

describe('ArcTool — typed VCB entry ', () => {
  it('chord stage: typed length commits B at that distance along the live cursor direction', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)   // A
    tool.onPointerMove(makeSnap({ x: 1, y: 0 }), RAY)   // establish +X direction
    typeDigits(tool, '5')
    tool.onKey(makeKeyEvent('Enter'))                   // commits B = (5, 0)

    // Now in the bulge stage with a=(0,0), b=(5,0) — commit via pointer.
    tool.onPointerDown(makeSnap({ x: 2.5, y: 1 }), RAY)
    expect(onCommit).toHaveBeenCalledTimes(1)

    expect(segments[0][0]).toBe(0)
    expect(segments[0][1]).toBe(0)
    expect(segments[segments.length - 1][3]).toBe(5)
    expect(segments[segments.length - 1][4]).toBe(0)
  })

  it('chord stage: typed commit is refused with no live cursor direction yet', () => {
    const { scene } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A — no pointer move yet
    typeDigits(tool, '5')
    tool.onKey(makeKeyEvent('Enter'))

    // Still waiting for B: an ordinary click now becomes B, not a bulge commit.
    tool.onPointerDown(makeSnap({ x: 3, y: 0 }), RAY)
    expect(onCommit).not.toHaveBeenCalled()
    tool.onPointerDown(makeSnap({ x: 1.5, y: 1 }), RAY) // bulge → commits
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('chord stage: a typed distance below ARC_MIN_CHORD_M is refused', () => {
    const { scene } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0 }), RAY)
    typeDigits(tool, '0')
    tool.onKey(makeKeyEvent('Enter'))

    // Still waiting for B.
    tool.onPointerDown(makeSnap({ x: 3, y: 0 }), RAY)
    expect(onCommit).not.toHaveBeenCalled()
    tool.onPointerDown(makeSnap({ x: 1.5, y: 1 }), RAY)
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('bulge stage: typed length sets |sagitta| on the side the live cursor is currently on', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY) // B — chord (0,0)-(2,0)
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY) // cursor on the +y side
    typeDigits(tool, '1')
    tool.onKey(makeKeyEvent('Enter')) // commits sagitta = +1 (the typed magnitude, +y side)

    expect(onCommit).toHaveBeenCalledTimes(1)
    // Every vertex lands on the +y (bulge) side, matching the live cursor's side.
    for (const s of segments) {
      expect(s[1]).toBeGreaterThanOrEqual(0)
      expect(s[4]).toBeGreaterThanOrEqual(0)
    }
    // The sagitta really is ~1 (not the ~0.5 the cursor itself was at) — the
    // typed distance overrides the live cursor's distance, only its side.
    const ys = segments.flatMap((s) => [s[1], s[4]])
    expect(Math.max(...ys)).toBeCloseTo(1, 6)
  })

  it('bulge stage: cursor exactly on the chord falls back to the last nonzero side seen', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY) // B
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY) // establish +y side
    tool.onPointerMove(makeSnap({ x: 1, y: 0 }), RAY)   // cursor now ON the chord
    typeDigits(tool, '1')
    tool.onKey(makeKeyEvent('Enter'))

    expect(onCommit).toHaveBeenCalledTimes(1)
    const ys = segments.flatMap((s) => [s[1], s[4]])
    expect(Math.max(...ys)).toBeCloseTo(1, 6) // committed on the remembered +y side
  })

  it('bulge stage: no side ever established refuses the typed commit (same hint as a flat pointer commit)', () => {
    const { scene } = makeWasmScene()
    const { tool, onCommit, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY) // B — no pointer move since
    typeDigits(tool, '1')
    tool.onKey(makeKeyEvent('Enter'))

    expect(onCommit).not.toHaveBeenCalled()
    expect(onMeasurement).toHaveBeenLastCalledWith('Pull out the bulge')

    // The gesture is still alive — a real pointer bulge still commits.
    tool.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY)
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('the typed buffer readout replaces the live measurement until committed', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0 }), RAY)
    onMeasurement.mockClear()

    typeDigits(tool, '5')
    expect(onMeasurement).toHaveBeenLastCalledWith(expect.stringContaining('5'))

    // Further pointer movement must NOT clobber the typed readout.
    tool.onPointerMove(makeSnap({ x: 3, y: 0 }), RAY)
    expect(onMeasurement).toHaveBeenLastCalledWith(expect.stringContaining('5'))
  })
})

describe('ArcTool — face mode', () => {
  const pick = () => makePick(7n, 3n)

  function primeFaceTool(opts: Parameters<typeof makeWasmScene>[0] = {}) {
    const made = makeWasmScene({ pick: pick(), facePlane: [0, 0, 1, 0, 0, 1], faceNormal: [0, 0, 1], ...opts })
    const t = makeTool(made.scene)
    t.tool.setEditContext({ kind: 'object', id: 7n })
    return { ...made, ...t }
  }

  it('three clicks commit one face draft whose open path endpoints are exactly A and B, carrying the curve bracket of the arc', () => {
    const { tool, onFaceImprint, segments, scene } = primeFaceTool()

    // All snaps on the z=1 face plane.
    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)   // A on the boundary
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 1 }), RAY)   // B on the boundary
    tool.onPointerDown(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)   // bulge into the face

    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1)
    const [object, face] = (scene.draft_begin_face as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(object).toBe(7n)
    expect(face).toBe(3n)
    // The whole arc rides ONE curve bracket carrying its analytic circle.
    expect(scene.draft_begin_curve_with).toHaveBeenCalledTimes(1)
    const [, center, radius] = (scene.draft_begin_curve_with as ReturnType<typeof vi.fn>).mock.calls[0]
    expect((center as Float64Array).length).toBe(3)
    expect(radius).toBeGreaterThan(0)
    expect(scene.draft_end_curve).toHaveBeenCalledTimes(1)
    expect(onFaceImprint).toHaveBeenCalledWith(7n)

    const path = pathOf(segments)
    expect(path.length % 3).toBe(0)
    const nPts = path.length / 3
    expect(nPts).toBeGreaterThanOrEqual(3)
    // Exact endpoints.
    expect(path[0]).toBe(0.5)
    expect(path[1]).toBe(2)
    expect(path[2]).toBe(1)
    expect(path[(nPts - 1) * 3 + 0]).toBe(1.5)
    expect(path[(nPts - 1) * 3 + 1]).toBe(2)
    expect(path[(nPts - 1) * 3 + 2]).toBe(1)
    // Every interior vertex stays on the face plane (z = 1) and bulges toward −y.
    for (let i = 1; i < nPts - 1; i++) {
      expect(path[i * 3 + 2]).toBeCloseTo(1, 10)
      expect(path[i * 3 + 1]).toBeLessThan(2)
    }
  })

  it('ignores a first click on a face of a different object', () => {
    const { scene } = makeWasmScene({ pick: makePick(9n, 3n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)
    expect(tool.capturingInput()).toBe(false)
  })

  it('surfaces a kernel error from a face draft as a toast', () => {
    const { tool, onFaceImprint, onToast } = primeFaceTool({ splitFaceThrows: true })

    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 1 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)

    expect(onFaceImprint).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('BadLoop')
  })

  it('snapConstraint locks to the face plane once the gesture starts', () => {
    const { tool } = primeFaceTool()

    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)
    const constraint = tool.snapConstraint(RAY)
    expect(constraint?.constraintPlane?.point).toEqual([0.5, 2, 1])
    expect(constraint?.constraintPlane?.normal).toEqual([0, 0, 1])
  })
})

describe('ArcTool — instance editing context (component-edit-parity.md phase A2)', () => {
  const INSTANCE = 42n
  const COMPONENT = 5n

  function primeInstanceFaceTool(opts: Parameters<typeof makeWasmScene>[0] = {}) {
    const made = makeWasmScene({
      pick: makePick(7n, 3n, INSTANCE),
      facePlane: [0, 0, 1, 0, 0, 1],
      faceNormal: [0, 0, 1],
      ...opts,
    })
    const t = makeTool(made.scene)
    t.tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    t.tool.setFaceEligibility((_object, instance) => instance === INSTANCE)
    return { ...made, ...t }
  }

  it('an OPEN face arc routes to draft_begin_face_in_instance, never a world call', () => {
    const { tool, onFaceImprint, scene } = primeInstanceFaceTool()

    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 1 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)

    expect(scene.draft_begin_face_in_instance).toHaveBeenCalledTimes(1)
    const [instance, object, face] = (scene.draft_begin_face_in_instance as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(instance).toBe(INSTANCE)
    expect(object).toBe(7n)
    expect(face).toBe(3n)
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
    expect(scene.draft_end).toHaveBeenCalledTimes(1)
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  it('a CLOSED (pie) face arc routes to draft_begin_face_in_instance, never the world draft_begin_face', () => {
    const { tool, onFaceImprint, scene } = primeInstanceFaceTool()

    tool.onPointerDown(makeSnap({ x: 0.5, y: 2, z: 1 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 1 }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)
    tool.onKey(makeKeyEvent('Alt')) // open → pie
    tool.onPointerDown(makeSnap({ x: 1, y: 1.6, z: 1 }), RAY)

    expect(scene.draft_begin_face_in_instance).toHaveBeenCalledTimes(1)
    const [instance, object, face] = (scene.draft_begin_face_in_instance as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(instance).toBe(INSTANCE)
    expect(object).toBe(7n)
    expect(face).toBe(3n)
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(scene.draft_end).toHaveBeenCalledTimes(1)
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  it('plane mode on empty space opens a plane draft in the instance via draft_begin_plane_in_instance', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(makeSnap({ x: 6, y: 1, z: 0 }), { origin: [6, 1, 5], direction: [0, 0, -1] })
    tool.onPointerDown(makeSnap({ x: 9, y: 1, z: 0 }), { origin: [9, 1, 5], direction: [0, 0, -1] })
    tool.onPointerDown(makeSnap({ x: 7.5, y: 2, z: 0 }), { origin: [7.5, 2, 5], direction: [0, 0, -1] })

    expect(scene.draft_begin_plane_in_instance).toHaveBeenCalledTimes(1)
    expect((scene.draft_begin_plane_in_instance as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(INSTANCE)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
    // World-space points go straight through (no pose mapping in the tool).
    const xs = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[1] as number)
    expect(Math.min(...xs)).toBeGreaterThanOrEqual(6)
  })
})

describe('ArcTool — status hint', () => {
  it('tracks the gesture stage: first endpoint → second endpoint → bulge → idle again', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    expect(tool.statusHint()).toContain('first endpoint')
    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A
    expect(tool.statusHint()).toContain('second endpoint')
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY) // B
    expect(tool.statusHint()).toContain('Alt cycles')
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY) // commit
    // Committed: the retype window offers to redraw the arc at a typed
    // bulge; Escape closes it and the idle hint returns.
    expect(tool.statusHint()).toContain('redraw the arc')
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.statusHint()).toContain('first endpoint')
  })
})

// Post-click bulge retype (retypeWindow.ts): a length typed after the third
// click redraws the arc at that sagitta, same chord, same side.
describe('ArcTool — retype the bulge after the third click', () => {
  function makeRetypeScene(opts: Parameters<typeof makeWasmScene>[0] = {}) {
    const base = makeWasmScene(opts).scene as unknown as Record<string, unknown>
    let gen = 1n
    const baseEnd = base.draft_end as (...args: unknown[]) => unknown
    const scene = {
      ...base,
      history_generation: vi.fn(() => gen),
      // A draft that ends records one history step.
      draft_end: vi.fn((...args: unknown[]) => { const r = baseEnd(...args); gen += 1n; return r }),
      scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
      scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    }
    return scene as unknown as WasmScene
  }
  const key = (tool: ArcTool, k: string) => tool.onKey(makeKeyEvent(k))
  const typeIn = (tool: ArcTool, text: string) => { for (const ch of text) key(tool, ch); key(tool, 'Enter') }
  const addCalls = (scene: WasmScene) => (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls
  /** Extreme y among the segment endpoints committed since `from` — the apex of a chord on y=0. */
  const apexY = (scene: WasmScene, from: number, sign: 1 | -1) => {
    const ys = addCalls(scene).slice(from).flatMap((k) => [k[2] as number, k[5] as number])
    return sign > 0 ? Math.max(...ys) : Math.min(...ys)
  }

  it('typing a bulge after the commit redraws the arc at that sagitta on the same side', () => {
    const scene = makeRetypeScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY) // B
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY) // commit, bulge toward +y
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(apexY(scene, 0, 1)).toBeCloseTo(0.5, 2)
    expect(tool.capturesKey('1')).toBe(true)
    const from = addCalls(scene).length
    typeIn(tool, '1')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledTimes(2)
    // Apex now ~1 above the chord, still on +y; chord endpoints unchanged.
    expect(apexY(scene, from, 1)).toBeCloseTo(1, 2)
    expect(apexY(scene, from, -1)).toBeCloseTo(0, 6)
    const xs = addCalls(scene).slice(from).flatMap((k) => [k[1] as number, k[4] as number])
    expect(Math.min(...xs)).toBeCloseTo(0, 6)
    expect(Math.max(...xs)).toBeCloseTo(2, 6)
  })

  it('an arc drawn to the −y side retypes to the −y side', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: -0.5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: -0.5 }), RAY)
    const from = addCalls(scene).length
    typeIn(tool, '0.8')
    expect(apexY(scene, from, -1)).toBeCloseTo(-0.8, 2)
    expect(apexY(scene, from, 1)).toBeCloseTo(0, 6)
  })

  it('a flat typed bulge is refused up front — nothing undone, the hint shows', () => {
    const scene = makeRetypeScene()
    const { tool, onMeasurement } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)
    typeIn(tool, '0')
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(onMeasurement).toHaveBeenLastCalledWith(expect.stringContaining('bulge'))
  })
})

// Boundary-click fix: a click on an object's edge/vertex misses `pick_face`
// outright (strict polygon test at the boundary) — `_facePickAt` falls back
// to the ranked boundary pick (`FacePickCache.faceThrough`) instead of
// silently sending the whole gesture to the ground plane.
describe('ArcTool — boundary clicks (FacePickCache.faceThrough)', () => {
  function makeBoundaryScene(pick: ReturnType<typeof makePick>) {
    const made = makeWasmScene({ faceNormal: [0, 0, 1], facePlane: [0, 0, 1, 0, 0, 1] })
    ;(made.scene.pick_face as ReturnType<typeof vi.fn>).mockImplementation(
      (_ox: number, _oy: number, _oz: number, dx: number, dy: number, dz: number) =>
        dx === 0 && dy === 0 && dz === -1 ? undefined : pick,
    )
    return made
  }

  it('a click on a midpoint snap (pick_face misses the exact ray) still anchors FACE mode, not the ground', () => {
    const { scene } = makeBoundaryScene(makePick(7n, 3n))
    const { tool, onFaceImprint, onCommit } = makeTool(scene)

    const boundarySnap = makeSnap({ x: 0.5, y: 2, z: 1, kind: 'midpoint', object: 7n, element: 5n, elementKind: 'edge' })
    tool.onPointerDown(boundarySnap, RAY) // A, exactly on the boundary
    expect(tool.capturingInput()).toBe(true) // anchored — not silently dropped

    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 1 }), { origin: [1.5, 2, 5], direction: [0, 0, -1] }) // B
    tool.onPointerDown(makeSnap({ x: 1, y: 1.6, z: 1 }), { origin: [1, 1.6, 5], direction: [0, 0, -1] }) // bulge → commit

    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1) // the FACE draft
    expect(scene.draft_begin_plane).not.toHaveBeenCalled() // never fell through to a plane draft
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
    expect(onCommit).not.toHaveBeenCalled()
  })
})

// Snapped face cursor fix: face mode used to project the ray onto the face
// plane unconditionally, ignoring whatever the snap chip claimed.
// `_faceCursor` now honours an on-plane snap exactly (this was ALREADY
// ArcTool's behaviour for a bare snap — the fix adds the on-plane guard so
// an off-plane snap still falls back to the ray, like the other draw tools).
describe('ArcTool — snapped face cursor (honours the chip, not ray∩plane)', () => {
  it('the chord endpoint B lands on the snap, not the sub-millimeter-off ray∩plane point', () => {
    const pick = makePick(7n, 3n)
    const { scene, segments } = makeWasmScene({ pick, faceNormal: [0, 0, 1], facePlane: [0, 0, 1, 0, 0, 1] })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), RAY) // A
    // The chord endpoint's snap sits exactly at (4,0,1); the ray it
    // travelled along would land 5mm further out on the SAME plane — a
    // deliberately different point the fix must NOT use.
    tool.onPointerDown(
      makeSnap({ x: 4, y: 0, z: 1, kind: 'endpoint' }),
      { origin: [4.005, 0, 5], direction: [0, 0, -1] },
    ) // B
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 1 }), { origin: [2, 1, 5], direction: [0, 0, -1] }) // bulge → commit

    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1)
    const path = pathOf(segments)
    const n = path.length / 3
    // `arcPolylineOnPlane` always ends the chain on the EXACT chord endpoint
    // it was built with, so the committed path's last vertex reflects
    // exactly what `_faceCursor` resolved B to.
    expect(path[(n - 1) * 3 + 0]).toBeCloseTo(4, 6)
    expect(path[(n - 1) * 3 + 1]).toBeCloseTo(0, 6)
  })
})

// Shift-pinned drawing plane (GitHub issue 14 / planePin.ts): pressing Shift
// while hovering a face pins that face's plane for the next gesture, even
// once the cursor leaves the face entirely.
describe('ArcTool — Shift-pinned drawing plane (GitHub issue 14)', () => {
  it('pins the hovered face plane; a later click over empty space anchors plane mode on it', () => {
    const pick = makePick(7n, 3n)
    // Face at z=1, normal +Z — a genuinely non-ground plane. `pick_face`
    // hits only under the original hover ray, so the later clicks are
    // genuinely over empty space, not merely ignored by the mode dispatch.
    const { scene, segments } = makeWasmScene({ faceNormal: [0, 0, 1], facePlane: [0, 0, 1, 0, 0, 1] })
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockImplementation(
      (ox: number, oy: number, oz: number, dx: number, dy: number, dz: number) =>
        ox === 0 && oy === 0 && oz === 5 && dx === 0 && dy === 0 && dz === -1 ? pick : undefined,
    )
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), RAY)
    tool.setShiftHeld(true)
    expect(tool.statusHint()).toContain('Pinned')

    const emptyRay: Ray = { origin: [10, 10, 5], direction: [0, 0, -1] }
    expect(tool.snapConstraint(emptyRay)?.constraintPlane).toEqual({ point: [0, 0, 1], normal: [0, 0, 1] })

    // A, B, bulge — all over empty space, all on the pinned plane.
    tool.onPointerDown(makeSnap({ x: 10, y: 10, z: 1 }), emptyRay) // A
    tool.onPointerDown(makeSnap({ x: 13, y: 10, z: 1 }), { origin: [13, 10, 5], direction: [0, 0, -1] }) // B
    tool.onPointerDown(makeSnap({ x: 11.5, y: 11, z: 1 }), { origin: [11.5, 11, 5], direction: [0, 0, -1] }) // bulge

    // One plane draft on the pinned z = 1 plane (not the ground, not a face).
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    const begin = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(begin[2]).toBeCloseTo(1, 9)
    expect(begin.slice(3, 6)).toEqual([0, 0, 1])
    expect(segments.length).toBeGreaterThan(0) // a real plane-mode chain committed
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('a second Shift press releases the pin; autorepeat (Shift held) does not toggle twice', () => {
    const pick = makePick(7n, 3n)
    const { scene } = makeWasmScene({ pick, faceNormal: [0, 0, 1], facePlane: [0, 0, 1, 0, 0, 1] })
    const { tool } = makeTool(scene)

    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), RAY)
    tool.setShiftHeld(true)
    expect(tool.statusHint()).toContain('Pinned')

    tool.setShiftHeld(true)
    tool.setShiftHeld(true)
    expect(tool.statusHint()).toContain('Pinned')

    tool.setShiftHeld(false)
    tool.setShiftHeld(true)
    expect(tool.statusHint()).not.toContain('Pinned')
  })

  it('Escape while idle clears the pin', () => {
    const pick = makePick(7n, 3n)
    const { scene } = makeWasmScene({ pick, faceNormal: [0, 0, 1], facePlane: [0, 0, 1, 0, 0, 1] })
    const { tool } = makeTool(scene)

    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), RAY)
    tool.setShiftHeld(true)
    expect(tool.statusHint()).toContain('Pinned')

    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.statusHint()).not.toContain('Pinned')
    expect(tool.hasArmedGesture()).toBe(false)
  })
})


describe('ArcTool — clicked chord and radius are exactly the displayed values', () => {
  // These pin the Exact digits (9.399 mm); the default mm rounding is coarser.
  beforeAll(() => setPrecision('exact', 'mm'))
  afterAll(() => setPrecision(DEFAULT_PRECISION.mm, 'mm'))
  const prev = getLengthUnit()
  const at = (x: number, y: number, kind = 'ground') => makeSnap({ x, y, z: 0, kind })
  // Chord shows "20.004 mm" exactly when B is at 0.0200041 (off-grid); bulge cursor off-grid.
  const B = 0.0200041
  const CUR = { x: 0.0100021, y: 0.0043217 }

  function shownRadius(measure: ReturnType<typeof vi.fn>): number {
    const texts = measure.mock.calls.map((c) => String(c[0])).filter((t) => t.startsWith('R '))
    return parseLengthToMeters(texts[texts.length - 1].slice(2))!
  }

  it('free clicks commit the same arc as typing the shown chord then the sagitta of the shown radius', () => {
    setLengthUnit('mm')
    try {
      const { scene, segments } = makeWasmScene()
      const { tool, onMeasurement } = makeTool(scene)
      tool.onPointerDown(at(0, 0), RAY)
      tool.onPointerMove(at(B, 0), RAY)
      tool.onPointerDown(at(B, 0), RAY)
      tool.onPointerMove(at(CUR.x, CUR.y), RAY)
      const R = shownRadius(onMeasurement)
      tool.onPointerDown(at(CUR.x, CUR.y), RAY)

      // Reference: the typed path with the converted values.
      const c = 20.004 * 0.001
      const sd = sagittaForDisplayedRadius(c, CUR.y, CUR.y > c / 2)!
      const { scene: ref, segments: refSegments } = makeWasmScene()
      const r = makeTool(ref)
      r.tool.onPointerDown(at(0, 0), RAY)
      r.tool.onPointerMove(at(B, 0), RAY)
      typeDigits(r.tool, '20.004')
      r.tool.onKey(makeKeyEvent('Enter'))
      r.tool.onPointerMove(at(CUR.x, CUR.y), RAY)
      typeDigits(r.tool, String(sd / 0.001))
      r.tool.onKey(makeKeyEvent('Enter'))

      expect(segments.length).toBeGreaterThan(2)
      expect(segments).toEqual(refSegments)
      // The committed arc's radius is the displayed one.
      expect(arcFromChord([0, 0], [c, 0], sd)!.radius).toBeCloseTo(R, 12)
    } finally {
      setLengthUnit(prev)
    }
  })

  it('hard-snapped (endpoint) chord and bulge clicks keep their exact points', () => {
    setLengthUnit('mm')
    try {
      const { scene, segments } = makeWasmScene()
      const { tool } = makeTool(scene)
      tool.onPointerDown(at(0, 0), RAY)
      tool.onPointerDown(at(B, 0, 'endpoint'), RAY)
      tool.onPointerMove(at(CUR.x, CUR.y, 'midpoint'), RAY)
      tool.onPointerDown(at(CUR.x, CUR.y, 'midpoint'), RAY)
      // Chord end B is exactly the snapped x (the arc's final vertex).
      expect([segments[0][0], segments[0][1]]).toEqual([0, 0])
      expect(segments[segments.length - 1][3]).toBe(B)
    } finally {
      setLengthUnit(prev)
    }
  })
})

describe('ArcTool — a flat (circle-less) arc', () => {
  const FLAT = { chain: [[0, 0, 0], [1, 0, 0], [2, 0, 0]] as [number, number, number][], curveSegments: 2, geom: null }
  type Runner = { _runArcDraft(target: unknown, chain: unknown): boolean }

  it('commits as ONE curve chain with no circle, on a face', () => {
    const { scene, segments, dm } = makeWasmScene()
    const { tool } = makeTool(scene)
    expect((tool as unknown as Runner)._runArcDraft({ kind: 'face', object: 1n, face: 2n, instance: null }, FLAT)).toBe(true)
    expect(segments).toHaveLength(2)
    expect(dm.members.draft_begin_curve).toHaveBeenCalledTimes(1)
    expect(dm.members.draft_begin_curve_with).not.toHaveBeenCalled()
    expect(dm.log.curves.map((c) => c.kind)).toEqual(['curve', 'end'])
  })

  it('closes the bracket before a plain closing edge, on a plane context', () => {
    const { scene, segments, dm } = makeWasmScene()
    const { tool } = makeTool(scene)
    const chain = { ...FLAT, chain: [...FLAT.chain, [0, 0, 0] as [number, number, number]] }
    expect((tool as unknown as Runner)._runArcDraft({ kind: 'ctx', ctx: '{"plane":[0,0,0,0,0,1]}' }, chain)).toBe(true)
    expect(segments).toHaveLength(3)
    expect(dm.log.curves.map((c) => c.kind)).toEqual(['curve', 'end'])
  })
})

describe('ArcTool — typed chord with the cursor on the Blue axis (#51a)', () => {
  const blue = (x: number, y: number, z: number): Snap =>
    makeSnap({ x, y, z, kind: 'on-axis', direction: [0, 0, 1] })

  it('hovering Blue straight above a ground A and typing a chord takes the chord straight up, as Line does', () => {
    const { scene, segments } = makeWasmScene()
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 2 }), RAY) // A on the ground
    tool.onPointerMove(blue(1, 2, 3), RAY)            // Blue, straight up from A
    typeDigits(tool, '5')
    tool.onKey(makeKeyEvent('Enter'))                 // B = A + 5 up (ArcTool.rehome.test.ts)
    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 2.5, kind: 'free' }), RAY) // bulge, on the new vertical plane

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    const last = segments[segments.length - 1]
    expect([last[3], last[4], last[5]]).toEqual([1, 2, 5])
  })

  it('a typed chord before the cursor has moved off A says to point the arc first', () => {
    const { scene } = makeWasmScene()
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 2 }), RAY)
    typeDigits(tool, '5')
    tool.onKey(makeKeyEvent('Enter'))

    expect(onToast).toHaveBeenCalledWith('Move the cursor to point the arc, then type the length.')
  })

  it('after the arrow-key re-lock the hint points to, the same Blue hover takes the typed chord straight up', () => {
    const { scene, segments } = makeWasmScene()
    ;(scene as unknown as { axes: () => Float64Array }).axes = () => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 2 }), RAY)
    tool.onKey(makeKeyEvent('ArrowRight'))            // plane through A, normal Red: holds Blue
    tool.onPointerMove(blue(1, 2, 3), RAY)
    typeDigits(tool, '5')
    tool.onKey(makeKeyEvent('Enter'))                 // B = A + 5 up
    tool.onPointerDown(makeSnap({ x: 1, y: 3, z: 2.5 }), RAY) // bulge, in that plane

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    const last = segments[segments.length - 1]
    expect([last[3], last[4], last[5]]).toEqual([1, 2, 5])
  })
})

describe('ArcTool — X-ray: a hidden snap off the face drops onto it, disclosed (cursorOnFrozenPlane)', () => {
  const down = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  // Runs under the mode it is named for.
  beforeEach(() => setPickRenderMode('xray'))
  afterEach(() => setPickRenderMode('shaded'))

  it('the bulge point is the hidden snap dropped onto the face (not the ray hit), says "projected", and clears when idle', () => {
    const { scene, segments } = makeWasmScene({ pick: makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), down(0, 0))
    expect(tool.snapConstraint(down(0, 0))?.seeThroughPoints).toBe(true)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 1, kind: 'endpoint' }), down(1, 0))
    // Shaded's own case is unchanged: an on-face snap is taken exactly.
    tool.onPointerMove(makeSnap({ x: 0.5, y: 0.2, z: 1, kind: 'on-face' }), down(0.5, 0.2))
    expect(tool.snapProjected()).toBe(false)
    // A hidden midpoint below the face over the chord's middle; the ray
    // meets the face farther out (y 0.45).
    const hidden = makeSnap({ x: 0.5, y: 0.3, z: 0.6, kind: 'midpoint' })
    tool.onPointerMove(hidden, down(0.5, 0.45))
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(hidden, down(0.5, 0.45))
    expect(segments.length).toBeGreaterThan(0)
    for (const s of segments) {
      expect(s[2]).toBeCloseTo(1, 12)
      expect(s[5]).toBeCloseTo(1, 12)
    }
    const top = Math.max(...segments.flatMap((s) => [s[1], s[4]]))
    // Facets fall a hair short of the true sagitta; the ray hit would be 0.45.
    expect(Math.abs(top - 0.3)).toBeLessThan(1e-3)
    expect(tool.snapProjected()).toBe(false)
    tool.onPointerMove(makeSnap({ x: 0.3, y: 0.3, z: 1, kind: 'on-face' }), down(0.3, 0.3))
    expect(tool.snapProjected()).toBe(false)
  })
})

describe('ArcTool — see-through snapping on the frozen and hovered planes', () => {
  it('a chord on an arrow-locked (non-ground) plane offers hidden points through it', () => {
    const { scene } = makeWasmScene()
    ;(scene as unknown as { axes: () => Float64Array }).axes = () => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight'))            // plane through the click, normal Red
    tool.onPointerDown(makeSnap({ x: 1, y: 2 }), RAY)
    expect(tool.snapConstraint(RAY)).toEqual({
      constraintPlane: { point: [1, 2, 0], normal: [1, 0, 0] },
      seeThroughPoints: true,
      offPlanePoints: true,
    })
  })

  it('an idle hover over an eligible face offers hidden points through its plane; over the ground, nothing is constrained', () => {
    const { scene } = makeWasmScene({ pick: makePick(7n, 3n), faceNormal: [0, 0, 1], facePlane: [0, 0, 1, 0, 0, 1] })
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint(RAY)).toEqual({
      constraintPlane: { point: [0, 0, 1], normal: [0, 0, 1] },
      seeThroughPoints: true,
      offPlanePoints: true,
    })

    const { scene: bare } = makeWasmScene()
    expect(makeTool(bare).tool.snapConstraint(RAY)).toBeNull()
  })
})
