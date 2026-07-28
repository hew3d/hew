import { describe, expect, it, vi } from 'vitest'
import * as THREE from 'three'
import {
  glyphAttribution,
  glyphBoundingBoxes,
  glyphContoursByIndex,
  glyphIndexAt,
  selectFillRegions,
  TextPlaceTool,
  type TextPlacement,
} from './TextPlaceTool'
import type { GlyphRunContour } from '../text/glyphRun'
import type { Pt2 } from '../text/flatten'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

function square(cx: number, cy: number, half: number): Pt2[] {
  return [
    [cx - half, cy - half],
    [cx + half, cy - half],
    [cx + half, cy + half],
    [cx - half, cy + half],
  ]
}

describe('selectFillRegions', () => {
  it("an 'O'-shaped ring + disk (two nested regions) selects only the ring", () => {
    const ring = square(0, 0, 10)
    const disk = square(0, 0, 4)
    const selected = selectFillRegions(['ring', 'disk'], [ring, disk])
    expect(selected).toEqual(['ring'])
  })

  it('a single simple region (no counters) selects itself', () => {
    const region = square(0, 0, 5)
    expect(selectFillRegions(['solo'], [region])).toEqual(['solo'])
  })

  it("a 'B'-shaped fill with two counters selects only the outer fill", () => {
    const outer = square(0, 0, 10)
    const upperBowl = square(0, 4, 2)
    const lowerBowl = square(0, -4, 2)
    const selected = selectFillRegions(
      ['outer', 'upperBowl', 'lowerBowl'],
      [outer, upperBowl, lowerBowl],
    )
    expect(selected).toEqual(['outer'])
  })

  it('drops a numerically negligible sliver region even though it would classify as fill on its own', () => {
    // The real letterform (a large square) alongside a vanishingly small
    // triangle sliver elsewhere — the kind of artifact a self-intersecting
    // raw glyph outline produces (docs comment on `selectFillRegions`).
    // Both are "fill" by nesting depth (neither contains the other), but
    // the sliver's area is far below 1% of the letterform's — dropped.
    const letterform = square(100, 100, 50) // area 10000
    const sliver: Pt2[] = [
      [1000, 1000],
      [1000.01, 1000],
      [1000, 1000.01],
    ] // area ~0.00005 — utterly negligible next to 10000
    const selected = selectFillRegions(['letter', 'sliver'], [letterform, sliver])
    expect(selected).toEqual(['letter'])
  })

  it('keeps a genuinely small but real counter alongside a large fill (not treated as a sliver)', () => {
    // A small counter is still a meaningful fraction of the fill's area —
    // well above the 1% sliver floor — so it must still count toward
    // nesting depth and get excluded as a hole, not silently kept as fill
    // by the sliver filter mis-firing on real geometry.
    const outer = square(0, 0, 10) // area 400
    const smallCounter = square(0, 0, 2) // area 16 — 4% of the fill, real
    const selected = selectFillRegions(['outer', 'counter'], [outer, smallCounter])
    expect(selected).toEqual(['outer'])
  })

  it('returns nothing for an empty region list', () => {
    expect(selectFillRegions([], [])).toEqual([])
  })

  it('a period next to a large notdef box keeps the period when the sliver floor is scoped per glyph, not the whole run', () => {
    // The period's own (only) region is tiny — nowhere near 1% of the
    // notdef box's area — so a WHOLE-RUN ratio drops it as if it were a
    // numerical artifact, exactly the bug: a real small mark shares no
    // sketch geometry with the box it happens to sit near (design: every
    // glyph gets its own advance-width slot), so it should never be
    // judged against another glyph's scale at all.
    const period = square(0, 0, 0.5) // area 1
    const notdefBox = square(200, 0, 50) // area 10000, a separate glyph far away
    const regionIds = ['period', 'notdefBox']
    const boundaries = [period, notdefBox]

    // No glyph info given (the whole-run default, preserved for backward
    // compatibility and for every single-glyph test above): the period is
    // silently dropped.
    expect(selectFillRegions(regionIds, boundaries)).toEqual(['notdefBox'])

    // Scoped per glyph: the period is judged against its OWN glyph's
    // largest region — itself — and kept.
    const glyphIndexOf = [0, 1]
    expect(selectFillRegions(regionIds, boundaries, glyphIndexOf)).toEqual(['period', 'notdefBox'])
  })

  it('an absolute area floor still drops a genuinely degenerate single-region glyph, where a per-glyph ratio alone (always 1.0 against itself) cannot', () => {
    const degenerate: Pt2[] = [
      [0, 0],
      [1e-6, 0],
      [0, 1e-6],
    ] // area ~5e-13 — a numerical splinter, not a real feature
    const glyphIndexOf = [0]
    // Per-glyph ratio alone: a lone region is always "the largest in its
    // own group" (ratio 1.0), so it survives no matter how small.
    expect(selectFillRegions(['sliver'], [degenerate], glyphIndexOf)).toEqual(['sliver'])
    // Adding an absolute floor above this area drops it regardless.
    expect(selectFillRegions(['sliver'], [degenerate], glyphIndexOf, 1e-9)).toEqual([])
  })

  it("an AMBIGUOUSLY-attributed large region never inflates a small glyph's sliver floor and dropping its own genuine ink (the reviewer's exact repro)", () => {
    // L is a large, unrelated glyph (area 100) whose one resolved region
    // got attributed to S's group via `glyphIndexAt`'s ambiguous-overlap
    // tie-break (a pathological/self-touching outline scenario — see
    // `glyphAttribution`'s doc comment). S is a small glyph with its own
    // genuine, unambiguously-attributed ink (area 0.01), disjoint from
    // L's region so nesting never enters into it — both are independent
    // fill material.
    const lRegion = square(100, 100, 5) // area 100, misattributed to group 0
    const sInk = square(0, 0, 0.05) // area 0.01, S's own real ink
    const regionIds = ['L_region', 'S_ink']
    const boundaries = [lRegion, sInk]
    const glyphIndexOf = [0, 0] // both nominally reported under S's group
    const ambiguousAt = [true, false] // only L's region is the ambiguous one

    // Without the ambiguous flag (the pre-fix call shape, still supported
    // for backward compatibility): L's area of 100 sets group 0's floor to
    // 100 * 0.01 = 1.0, well above S's own 0.01-area ink — dropped.
    expect(selectFillRegions(regionIds, boundaries, glyphIndexOf, 0)).toEqual(['L_region'])

    // With the ambiguous flag: L's region is excluded from group 0's floor
    // computation (leaving only S's own 0.01-area ink to set it) and
    // bypasses the floor outright — both survive.
    const fixed = selectFillRegions(regionIds, boundaries, glyphIndexOf, 0, ambiguousAt)
    expect(fixed).toContain('L_region')
    expect(fixed).toContain('S_ink')
    expect(fixed).toHaveLength(2)
  })
})

describe('glyphIndexAt — real ink-based attribution (previously zero coverage: only glyphIndexOf arrays were injected by hand)', () => {
  function contour(glyphIndex: number, points: Pt2[]): GlyphRunContour {
    return { points, fill: true, glyphIndex }
  }

  it('attributes correctly under OVERLAPPING em-boxes with DISJOINT ink — a bounding-box test would pick the wrong glyph here', () => {
    // Glyph 0's em-BOX is [0,10]x[0,10], but its actual ink is only the
    // lower-left triangle (x+y < 10) — a diagonal stroke, or simply a
    // glyph whose box is much bigger than its ink (common for any
    // non-rectangular letterform). Glyph 1's box [5,15]x[5,15] overlaps
    // glyph 0's box in the (5,5)-(10,10) corner (the kind of overlap tight
    // or negative kerning, or an italic overhang, produces), and glyph 1's
    // ink is a full square filling its own box.
    const glyph0Ink: Pt2[] = [
      [0, 0],
      [10, 0],
      [0, 10],
    ]
    const glyph1Ink: Pt2[] = [
      [5, 5],
      [15, 5],
      [15, 15],
      [5, 15],
    ]
    const contours = [contour(0, glyph0Ink), contour(1, glyph1Ink)]
    const boxes = glyphBoundingBoxes(contours)
    const byGlyph = glyphContoursByIndex(contours)

    const probe: Pt2 = [7, 7] // inside glyph 0's BOX (0..10,0..10) but x+y=14 ≥ 10, outside glyph 0's actual ink; inside glyph 1's box AND ink.
    // A bounding-box test (iterating glyph 0 first) would wrongly return 0
    // here — glyph 0's box contains (7,7) even though its ink does not.
    expect(boxes.get(0)).toEqual({ minX: 0, maxX: 10, minY: 0, maxY: 10 })
    expect(glyphIndexAt(probe, byGlyph, boxes)).toBe(1)
  })

  it('an ambiguous TRUE ink overlap (both glyphs\' actual outlines contain the point) scopes to the SMALLER candidate glyph, conservatively', () => {
    // Reachable only through a pathological/self-touching font outline —
    // real placed-text glyphs never actually share ink by design — but the
    // attribution must still resolve deterministically rather than pick
    // whichever glyph's Map entry happens to iterate first.
    const bigGlyph = contour(0, [
      [0, 0],
      [20, 0],
      [20, 20],
      [0, 20],
    ]) // box/ink area 400
    const smallGlyph = contour(1, [
      [8, 8],
      [12, 8],
      [12, 12],
      [8, 12],
    ]) // box/ink area 16, entirely inside bigGlyph's ink too
    const contours = [bigGlyph, smallGlyph]
    const boxes = glyphBoundingBoxes(contours)
    const byGlyph = glyphContoursByIndex(contours)

    const probe: Pt2 = [10, 10] // inside BOTH glyphs' actual ink.
    expect(glyphIndexAt(probe, byGlyph, boxes)).toBe(1) // the smaller glyph, not glyph 0 (inserted first)
    // `glyphAttribution` (the richer form `glyphIndexAt` wraps) must also
    // report this resolution as AMBIGUOUS — `selectFillRegions` needs that
    // flag to keep this region's area from scaling glyph 1's own sliver
    // floor (see its `ambiguousAt` doc comment).
    expect(glyphAttribution(probe, byGlyph, boxes)).toEqual({ index: 1, ambiguous: true })
  })

  it('glyphAttribution reports ambiguous: false for every unambiguous resolution (single-candidate ink, box-overlap-but-disjoint-ink, and the no-candidate box-distance fallback)', () => {
    const glyph0Ink: Pt2[] = [
      [0, 0],
      [10, 0],
      [0, 10],
    ]
    const glyph1Ink: Pt2[] = [
      [5, 5],
      [15, 5],
      [15, 15],
      [5, 15],
    ]
    const contours = [contour(0, glyph0Ink), contour(1, glyph1Ink)]
    const boxes = glyphBoundingBoxes(contours)
    const byGlyph = glyphContoursByIndex(contours)

    // Overlapping em-boxes, disjoint ink: exactly one glyph's ink contains
    // the probe, unambiguous.
    expect(glyphAttribution([7, 7], byGlyph, boxes)).toEqual({ index: 1, ambiguous: false })

    // No glyph's ink contains the probe at all: the nearest-box fallback,
    // also unambiguous (only ever one nearest box).
    expect(glyphAttribution([100, 100], byGlyph, boxes)).toEqual({ index: 1, ambiguous: false })
  })

  it('attributes correctly for ordinary non-overlapping glyphs (the common case, exercised end to end)', () => {
    const letterA = contour(0, [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ])
    const letterB = contour(1, [
      [20, 0],
      [30, 0],
      [30, 10],
      [20, 10],
    ])
    const contours = [letterA, letterB]
    const boxes = glyphBoundingBoxes(contours)
    const byGlyph = glyphContoursByIndex(contours)

    expect(glyphIndexAt([5, 5], byGlyph, boxes)).toBe(0)
    expect(glyphIndexAt([25, 5], byGlyph, boxes)).toBe(1)
  })
})

function makePick(object: bigint, face: bigint, instance?: bigint) {
  return { object: () => object, face: () => face, instance: () => instance, free: vi.fn() }
}

/** Minimal WasmScene stub — only the members TextPlaceTool calls. */
function makeWasmScene(opts: { pick?: () => ReturnType<typeof makePick> | undefined } = {}): WasmScene {
  let sketchCounter = 900n
  return {
    pick_face: vi.fn(() => opts.pick?.()),
    node_parent: vi.fn(() => undefined),
    // An eligible face at z=1, normal +Z — never the plane this suite's
    // "lock beats face" case expects to win.
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    begin_sketch_on_plane: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    sketch_begin_gesture: vi.fn(),
    sketch_add_segment: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_regions: vi.fn(() => new BigUint64Array([1n])),
    region_boundary: vi.fn(() => new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0])),
    place_text: vi.fn(() => 555n),
  } as unknown as WasmScene
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => {} } as unknown as KeyboardEvent
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const placement: TextPlacement = {
    contours: [
      {
        points: [
          [-1, -1],
          [1, -1],
          [1, 1],
          [-1, 1],
        ],
        fill: true,
        glyphIndex: 0,
      },
    ],
    scale: 1,
    depth: 1,
    name: 'X',
  }
  const onPlaced = vi.fn()
  const onToast = vi.fn()
  const tool = new TextPlaceTool(scene, preview, placement, onPlaced, onToast)
  return { tool, onPlaced, onToast }
}

describe('TextPlaceTool — plane resolution', () => {
  it('an active idle plane lock beats an eligible face pick (RectangleTool.ts:246-248 precedent: "an active idle plane lock beats face pick... the user already chose a plane")', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) }) // an eligible face IS under the cursor
    const { tool } = makeTool(scene)

    tool.onKey(makeKeyEvent('ArrowRight')) // engage the red/X-axis idle lock

    // Crosses the x=0 plane at (0, 0, 3); would also cross the mocked
    // z=1 face plane if face-pick ran first.
    const ray: Ray = { origin: [2, 0, 5], direction: [-1, 0, -1] }
    tool.onPointerDown(null, ray)

    // The face pick is never even consulted once a lock is active — the
    // lock wins outright, it doesn't merely out-prioritize a pick result.
    expect(scene.pick_face).not.toHaveBeenCalled()
    expect(scene.face_plane).not.toHaveBeenCalled()

    expect(scene.begin_sketch_on_plane).toHaveBeenCalledTimes(1)
    const call = (scene.begin_sketch_on_plane as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call.slice(0, 3)).toEqual([0, 0, 3]) // origin: the ray hits x=0 at (0,0,3)
    expect(call.slice(3, 6)).toEqual([1, 0, 0]) // normal: the X-axis-locked plane, not the face's +Z
  })

  it('without a lock, an eligible face still wins (unlocked default, unchanged)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)

    const ray: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
    tool.onPointerDown(null, ray)

    expect(scene.pick_face).toHaveBeenCalledTimes(1)
    expect(scene.face_plane).toHaveBeenCalledWith(7n, 3n)
    const call = (scene.begin_sketch_on_plane as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call.slice(3, 6)).toEqual([0, 0, 1]) // the face's own +Z normal
  })
})
