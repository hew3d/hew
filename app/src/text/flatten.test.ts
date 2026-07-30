import { describe, expect, it } from 'vitest'
import {
  GLYPH_SAGITTA_TOL_M,
  classifyContourFill,
  contourNestingDepth,
  flattenCubic,
  flattenQuad,
  interiorPoint,
  pathCommandsToContours,
  pointInPolygon,
  sagittaToleranceEm,
  signedArea,
  weldContour,
  type FlattenCommand,
  type Pt2,
} from './flatten'

describe('sagittaToleranceEm', () => {
  it('scales the world sagitta budget by unitsPerEm/height', () => {
    // A 1-meter-tall glyph in a 1000-unit em-square: tolEm = tol * 1000/1.
    const tol = sagittaToleranceEm(1, 1000)
    expect(tol).toBeCloseTo(GLYPH_SAGITTA_TOL_M * 1000, 12)
  })

  it('a taller placement gets a LOOSER em-unit tolerance (fewer facets per glyph)', () => {
    const small = sagittaToleranceEm(0.01, 2048) // 10mm text
    const big = sagittaToleranceEm(0.5, 2048) // 500mm sign
    expect(big).toBeLessThan(small)
  })

  it('falls back to a finite tolerance for degenerate heights', () => {
    expect(Number.isFinite(sagittaToleranceEm(0, 1000))).toBe(true)
    expect(Number.isFinite(sagittaToleranceEm(-5, 1000))).toBe(true)
    expect(Number.isFinite(sagittaToleranceEm(NaN, 1000))).toBe(true)
  })
})

describe('flattenQuad / flattenCubic', () => {
  it('a straight (collinear) quadratic collapses to just the endpoint', () => {
    const pts = flattenQuad([0, 0], [5, 0], [10, 0], 1e-6)
    expect(pts).toEqual([[10, 0]])
  })

  it('a straight (collinear) cubic collapses to just the endpoint', () => {
    const pts = flattenCubic([0, 0], [3, 0], [7, 0], [10, 0], 1e-6)
    expect(pts).toEqual([[10, 0]])
  })

  it('every flattened point stays within tolerance of the true quadratic curve', () => {
    const p0: Pt2 = [0, 0]
    const p1: Pt2 = [50, 100]
    const p2: Pt2 = [100, 0]
    const tol = 0.5
    const pts = flattenQuad(p0, p1, p2, tol)
    // Sample the true curve densely and check every flattened vertex lies
    // close to SOME point on the true curve (a coarse but sufficient
    // fidelity check — the recursive flatness test already bounds the
    // chord-vs-control-point deviation directly).
    const trueCurve: Pt2[] = []
    for (let t = 0; t <= 1; t += 0.001) {
      const x = (1 - t) ** 2 * p0[0] + 2 * (1 - t) * t * p1[0] + t ** 2 * p2[0]
      const y = (1 - t) ** 2 * p0[1] + 2 * (1 - t) * t * p1[1] + t ** 2 * p2[1]
      trueCurve.push([x, y])
    }
    for (const fp of pts) {
      const minDist = Math.min(...trueCurve.map((tp) => Math.hypot(tp[0] - fp[0], tp[1] - fp[1])))
      expect(minDist).toBeLessThan(tol * 2)
    }
  })

  it('a tighter tolerance produces at least as many points as a looser one', () => {
    const p0: Pt2 = [0, 0]
    const p1: Pt2 = [50, 100]
    const p2: Pt2 = [100, 0]
    const loose = flattenQuad(p0, p1, p2, 5)
    const tight = flattenQuad(p0, p1, p2, 0.05)
    expect(tight.length).toBeGreaterThanOrEqual(loose.length)
  })

  it('the last point is always exactly the curve endpoint', () => {
    const pts = flattenCubic([0, 0], [10, 40], [40, 40], [50, 0], 0.1)
    expect(pts[pts.length - 1]).toEqual([50, 0])
  })
})

describe('pathCommandsToContours', () => {
  it('splits multiple M…Z runs into separate contours and drops the closing duplicate', () => {
    const commands: FlattenCommand[] = [
      { type: 'M', x: 0, y: 0 },
      { type: 'L', x: 10, y: 0 },
      { type: 'L', x: 10, y: 10 },
      { type: 'L', x: 0, y: 10 },
      { type: 'Z' },
      { type: 'M', x: 3, y: 3 },
      { type: 'L', x: 7, y: 3 },
      { type: 'L', x: 7, y: 7 },
      { type: 'L', x: 3, y: 7 },
      { type: 'Z' },
    ]
    const contours = pathCommandsToContours(commands, 0.5)
    expect(contours).toHaveLength(2)
    expect(contours[0]).toHaveLength(4)
    expect(contours[1]).toHaveLength(4)
  })

  it('drops a degenerate contour with fewer than 3 points', () => {
    const commands: FlattenCommand[] = [
      { type: 'M', x: 0, y: 0 },
      { type: 'L', x: 5, y: 5 },
      { type: 'Z' },
    ]
    expect(pathCommandsToContours(commands, 0.5)).toEqual([])
  })
})

describe('weldContour', () => {
  it('drops consecutive duplicate points within tolerance', () => {
    const pts: Pt2[] = [
      [0, 0],
      [0, 1e-12],
      [10, 0],
      [10, 10],
      [0, 10],
    ]
    const welded = weldContour(pts, 1e-9)
    expect(welded).toEqual([
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ])
  })

  it('drops a closing point that coincides with the start', () => {
    const pts: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
      [0, 0],
    ]
    expect(weldContour(pts, 1e-9)).toHaveLength(4)
  })

  it('drops a degenerate (collinear, zero-area) contour entirely', () => {
    const pts: Pt2[] = [
      [0, 0],
      [5, 0],
      [10, 0],
    ]
    expect(weldContour(pts, 1e-9)).toEqual([])
  })

  it('drops a contour that welds down to fewer than 3 points', () => {
    const pts: Pt2[] = [
      [0, 0],
      [0, 1e-12],
      [0, -1e-12],
    ]
    expect(weldContour(pts, 1e-9)).toEqual([])
  })

  it('leaves a clean, non-degenerate contour untouched', () => {
    const square: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ]
    expect(weldContour(square, 1e-9)).toEqual(square)
  })
})

describe('signedArea / pointInPolygon', () => {
  it('signed area is positive for CCW and negative for CW winding of the same square', () => {
    const ccw: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ]
    const cw = [...ccw].reverse()
    expect(signedArea(ccw)).toBeGreaterThan(0)
    expect(signedArea(cw)).toBeLessThan(0)
  })

  it('point-in-polygon finds the center of a square inside, and a far point outside', () => {
    const square: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ]
    expect(pointInPolygon([5, 5], square)).toBe(true)
    expect(pointInPolygon([50, 50], square)).toBe(false)
  })
})

describe('classifyContourFill — the glyph-with-counter case', () => {
  function square(cx: number, cy: number, half: number): Pt2[] {
    return [
      [cx - half, cy - half],
      [cx + half, cy - half],
      [cx + half, cy + half],
      [cx - half, cy + half],
    ]
  }

  it("an 'O'-like glyph (outer square + smaller concentric counter) yields exactly one fill and one hole", () => {
    const outer = square(0, 0, 10)
    const counter = square(0, 0, 4)
    const fills = classifyContourFill([outer, counter])
    expect(fills).toEqual([true, false])
  })

  it('order independence: the counter listed FIRST still classifies correctly', () => {
    const outer = square(0, 0, 10)
    const counter = square(0, 0, 4)
    const fills = classifyContourFill([counter, outer])
    expect(fills).toEqual([false, true])
  })

  it("a 'B'-like glyph (one outer silhouette, two side-by-side counters) yields one fill and two holes", () => {
    const outer = square(0, 0, 10)
    const upperBowl = square(0, 4, 2)
    const lowerBowl = square(0, -4, 2)
    const fills = classifyContourFill([outer, upperBowl, lowerBowl])
    expect(fills).toEqual([true, false, false])
  })

  it('a single simple contour (no counters, e.g. "I" or "T") is fill', () => {
    const stroke = square(0, 0, 5)
    expect(classifyContourFill([stroke])).toEqual([true])
  })

  it('three concentric loops classify fill/hole/fill (even-odd nesting parity)', () => {
    const outer = square(0, 0, 10)
    const middle = square(0, 0, 6)
    const inner = square(0, 0, 2)
    expect(classifyContourFill([outer, middle, inner])).toEqual([true, false, true])
  })

  it('two overlapping squares split into three disjoint regions (A-only, B-only, and the shared overlap lens) all classify as fill', () => {
    // Square A: (0,0)-(10,10). Square B: (5,5)-(20,20), overlapping A in
    // the (5,5)-(10,10) corner. This is what the kernel's region resolver
    // produces for two touching/overlapping glyph strokes (finding: an
    // overlapping/self-touching glyph, docs comment on
    // `kernel::Document::place_text`): three regions tiling the union,
    // sharing boundary VERTICES with each other but none nested inside
    // another — every one of them is real material (fill), none a hole.
    //
    // RED-CHECK NOTE: this fixture does NOT discriminate between the old
    // first-vertex probe (fb2eac0), the old ear-clip+nudge probe
    // (680d00b), or the current scanline probe — all three already return
    // `[true, true, true]` here (verified empirically against both prior
    // revisions). These regions are large and thick with no thin band
    // anywhere near a probe, so no probe implementation in this file's
    // history has ever misclassified them; kept as forward-looking
    // regression coverage for the overlap-lens shape, not as proof of any
    // particular fix. See the two-triangle test below and the thin-band
    // test in the `interiorPoint`/scanline section for the tests that DO
    // discriminate.
    const aOnly: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 5],
      [5, 5],
      [5, 10],
      [0, 10],
    ]
    const bOnly: Pt2[] = [
      [10, 5],
      [20, 5],
      [20, 20],
      [5, 20],
      [5, 10],
      [10, 10],
    ]
    const lens: Pt2[] = [
      [5, 5],
      [10, 5],
      [10, 10],
      [5, 10],
    ]
    const fills = classifyContourFill([aOnly, bOnly, lens])
    expect(fills).toEqual([true, true, true])
  })

  it("the reviewer's exact lens repro: two triangles sharing an edge (an overlap lens's own split boundary, in miniature) both classify as fill, never a hole", () => {
    // Two triangles sharing edge p1-p2, with apexes p3/p4 on OPPOSITE
    // sides of that edge — geometrically, a triangle with two vertices on
    // a line and its apex strictly on one side lies ENTIRELY on that side,
    // so these two triangles are disjoint save for the shared edge: real,
    // non-overlapping, non-nested material on both sides, exactly the
    // shape an overlap lens's own boundary split produces. The FIRST-ROUND
    // probe (fb2eac0, a contour's own first vertex, no nudge at all) used
    // `p2` (respectively `p1`) — a vertex sitting exactly ON THE OTHER
    // triangle's own boundary. Testing a boundary point for containment is
    // inherently ambiguous for a ray-cast: the far-side edge (the one not
    // touching that vertex) can still register a crossing, silently
    // flipping the second triangle from fill to a hole depending purely on
    // the unrelated far-side geometry.
    //
    // RED-CHECK NOTE: this DOES discriminate — verified empirically,
    // fb2eac0's `classifyContourFill` returns `[true, false]` here (wrong),
    // while both 680d00b's ear-clip+nudge probe and the current scanline
    // probe return `[true, true]` (right). It only catches the FIRST-round
    // regression (a probe sitting exactly on a boundary vertex); it does
    // NOT catch the SECOND-round regression (a fixed nudge overshooting a
    // thin material band) — see the thin-band test below, which 680d00b
    // itself fails and only the scanline probe passes.
    const p1: Pt2 = [2.7566061247205944, -6.535629152305365]
    const p2: Pt2 = [-0.559733064980275, -7.736556369452352]
    const p3: Pt2 = [7.608395202552995, -9.012520834636273]
    const p4: Pt2 = [-3.180587478393642, -6.551253474384162]
    const triangleA: Pt2[] = [p1, p2, p3]
    const triangleB: Pt2[] = [p2, p1, p4]
    expect(classifyContourFill([triangleA, triangleB])).toEqual([true, true])
  })

  it('the lens result is order-independent (every rotation of the three regions still classifies all-fill)', () => {
    // RED-CHECK NOTE: same as the main overlap-lens test above — every
    // permutation here already returns all-fill under fb2eac0 and 680d00b
    // too (verified empirically), so this is forward-looking order-
    // independence coverage, not a regression discriminator.
    const aOnly: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 5],
      [5, 5],
      [5, 10],
      [0, 10],
    ]
    const bOnly: Pt2[] = [
      [10, 5],
      [20, 5],
      [20, 20],
      [5, 20],
      [5, 10],
      [10, 10],
    ]
    const lens: Pt2[] = [
      [5, 5],
      [10, 5],
      [10, 10],
      [5, 10],
    ]
    expect(classifyContourFill([lens, aOnly, bOnly])).toEqual([true, true, true])
    expect(classifyContourFill([bOnly, lens, aOnly])).toEqual([true, true, true])
  })

  it('the thin-band repro: a 20×0.01 outer stroke with a nested hole leaving only a 6e-6 material band classifies the outer as fill, the hole as a hole', () => {
    // RED-CHECK: this is the CRITICAL regression the scanline probe fixes.
    // 680d00b's ear-clip+nudge probe anchors on the outer contour's first
    // ear vertex and nudges a FIXED fraction (1e-3) of the way toward that
    // ear's centroid — for this shape, that lands the outer probe at
    // roughly (0.0067, 0.0099933), which is only 6.7e-6 below the outer's
    // own top edge (y=0.01) — INSIDE the nested hole's ink-free interior
    // (the hole's top edge sits at y = 0.01 - 6e-6 = 0.009994). The nudge
    // assumes every reachable glyph shape has "some real thickness"; a
    // sub-pixel stroke this thin (well below `GLYPH_SAGITTA_TOL_M`, but
    // still real, still selectable input) breaks that assumption and
    // silently drops the whole stroke: `classifyContourFill` returns
    // `[false, false]` under 680d00b (verified empirically) instead of
    // `[true, false]`. The tolerance-free scanline probe has no thickness
    // assumption at all — it lands in the band up to the NEAREST crossing,
    // however thin that band actually is — and returns `[true, false]`
    // correctly (fb2eac0's plain first-vertex probe also happens to pass
    // this particular shape, since the outer's first vertex isn't a
    // touching-boundary case here; 680d00b is the one revision this
    // discriminates against).
    const outer: Pt2[] = [
      [0, 0],
      [20, 0],
      [20, 0.01],
      [0, 0.01],
    ]
    const band = 6e-6
    const hole: Pt2[] = [
      [0.0001, 0.0001],
      [19.9999, 0.0001],
      [19.9999, 0.01 - band],
      [0.0001, 0.01 - band],
    ]
    expect(classifyContourFill([outer, hole])).toEqual([true, false])
  })

  it('a CW-wound outer contour with a hole still classifies correctly (the scanline probe never assumes a winding convention)', () => {
    const outerCcw = square(0, 0, 10)
    const outerCw = [...outerCcw].reverse()
    const counter = square(0, 0, 4)
    expect(signedArea(outerCw)).toBeLessThan(0)
    expect(classifyContourFill([outerCw, counter])).toEqual([true, false])
    // Both contours reversed together: still correct.
    expect(classifyContourFill([outerCw, [...counter].reverse()])).toEqual([true, false])
  })

  it('three side-by-side squares sharing vertical edges (routine kernel-split, touching regions) all classify as fill regardless of array order — the shared-edge tied-crossing repro', () => {
    // W, X, Z tile a 3x1 strip, each sharing a full vertical edge with its
    // neighbor (X's left edge = Z's right edge at x=-1; X's right edge =
    // W's left edge at x=0) — exactly the boundary the kernel emits
    // between two adjacent post-split regions. The kernel's own region
    // enumeration order has no relationship to spatial layout, so the same
    // geometry must classify identically no matter which order the three
    // contours are listed in.
    const W: Pt2[] = [
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 1],
    ]
    const X: Pt2[] = [
      [-1, 0],
      [0, 0],
      [0, 1],
      [-1, 1],
    ]
    const Z: Pt2[] = [
      [-2, 0],
      [-1, 0],
      [-1, 1],
      [-2, 1],
    ]
    // Listed in spatial order (Z, X, W): under the OLD merged-crossing
    // probe this order never degenerated (X's tie with Z sorted before X's
    // own entry). The interval-containment replacement computes X's own
    // interval purely from X's own edges regardless of array order, so
    // this order is no longer special — kept for order-independence
    // coverage.
    expect(classifyContourFill([Z, X, W])).toEqual([true, true, true])

    // Listed as [W, X, Z]: under the OLD merged-crossing probe, X's
    // entering crossing (x=-1, shared with Z) tied with Z's crossing
    // pushed AFTER X in this order, making the untied pairing zero-width
    // (the confirmed regression this rewrite replaces). The
    // interval-containment version never merges crossings across contours
    // at all, so X's own interval is unaffected by array order — assert
    // `interiorPoint` (X's own leftmost interval, no siblings involved)
    // still lands strictly inside X, not on either shared edge.
    const probeX = interiorPoint(X)
    expect(probeX[0]).toBeGreaterThan(-1) // strictly right of the shared X/Z edge
    expect(probeX[0]).toBeLessThan(0) // strictly left of the shared X/W edge
    expect(classifyContourFill([W, X, Z])).toEqual([true, true, true])
  })

  it("the reviewer's exact nested-shared-edge repro: a contour N nested inside X but sharing part of X's own boundary still classifies X as fill and N as a hole, in both array orders", () => {
    // X: a 10x10 square. N: a smaller rectangle nested INSIDE X but sharing
    // X's entire left edge over N's own vertical span (touching, not
    // crossing) — the confirmed MAJOR finding against the point-probe
    // approach: at the scanline through N's vertical mid-span, X's
    // ENTERING crossing (x=0) ties with N's own left-edge crossing (also
    // x=0). The old merged-crossing probe skipped past every crossing tied
    // at the entering x and paired with the next DISTINCT one, which
    // landed on N's own right edge (x=3) rather than X's true far edge
    // (x=10) — producing a probe INSIDE N, so X read as nested inside N
    // and got misclassified as a hole (`classifyContourFill` returned
    // `[false, false]`, order-independent, verified against the prior
    // revision). Interval containment never merges crossings across
    // contours in the first place: X's own interval [0,10] is computed
    // from X's own edges alone, N's own interval [0,3] from N's alone, and
    // `[0,3]` does not COVER `[0,10]` (its far end falls short) — so N
    // is correctly found to NOT contain X, while X's own interval [0,10]
    // does cover N's [0,3], so N is correctly found nested inside X.
    const X: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ]
    const N: Pt2[] = [
      [0, 4],
      [3, 4],
      [3, 6],
      [0, 6],
    ]
    expect(classifyContourFill([X, N])).toEqual([true, false])
    expect(classifyContourFill([N, X])).toEqual([false, true])
  })

  it('a long-straight-edge glyph-stroke shape (many collinear points along one edge, mimicking an over-tessellated flattened stroke) still classifies as fill', () => {
    // A vertical stroke (like the stem of an 'I' or 'L') whose long top and
    // bottom edges each carry several redundant COLLINEAR points — the
    // shape a flattened glyph outline can produce when a nearly-straight
    // Bézier segment still gets subdivided. This stresses the scanline
    // probe's "avoid every vertex-y" gap search: most of these points
    // share only two distinct y-values (top and bottom), so the global
    // vertex-y list is small and the tightest gap is the whole span.
    const stroke: Pt2[] = [
      [0, 0],
      [1, 0],
      [2, 0],
      [3, 0],
      [4, 0],
      [4, 20],
      [3, 20],
      [2, 20],
      [1, 20],
      [0, 20],
    ]
    expect(classifyContourFill([stroke])).toEqual([true])
    expect(pointInPolygon(interiorPoint(stroke), stroke)).toBe(true)
  })
})

describe('interiorPoint', () => {
  it('stays inside a concave (reflex-vertex) polygon, an L-shape', () => {
    const lShape: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 4],
      [4, 4],
      [4, 10],
      [0, 10],
    ]
    expect(pointInPolygon(interiorPoint(lShape), lShape)).toBe(true)
  })

  it('stays inside every contour of the overlap-lens fixture', () => {
    const aOnly: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 5],
      [5, 5],
      [5, 10],
      [0, 10],
    ]
    const bOnly: Pt2[] = [
      [10, 5],
      [20, 5],
      [20, 20],
      [5, 20],
      [5, 10],
      [10, 10],
    ]
    const lens: Pt2[] = [
      [5, 5],
      [10, 5],
      [10, 10],
      [5, 10],
    ]
    for (const poly of [aOnly, bOnly, lens]) {
      expect(pointInPolygon(interiorPoint(poly), poly)).toBe(true)
    }
  })
})

describe('contourNestingDepth — equal-interval (shared-wall) containment', () => {
  // Reviewer's confirmed repro: a full-width band sharing BOTH the left
  // and right walls with its 10x10 square container. Before the fix, the
  // exact x-interval tie at the sampled scanline (`a === enterX && b ===
  // exitX`) was treated as blanket "not containing", so the band's own
  // depth came out even (0) instead of odd (1) — filled when it should
  // have been a hole.
  const square: Pt2[] = [
    [0, 0],
    [10, 0],
    [10, 10],
    [0, 10],
  ]
  const fullWidthBand: Pt2[] = [
    [0, 3],
    [10, 3],
    [10, 7],
    [0, 7],
  ]

  it('the square contains the full-width band regardless of array order', () => {
    const forward = [square, fullWidthBand]
    expect(contourNestingDepth(forward, 0)).toBe(0)
    expect(contourNestingDepth(forward, 1)).toBe(1)
    expect(classifyContourFill(forward)).toEqual([true, false])

    const reversed = [fullWidthBand, square]
    expect(contourNestingDepth(reversed, 0)).toBe(1)
    expect(contourNestingDepth(reversed, 1)).toBe(0)
    expect(classifyContourFill(reversed)).toEqual([false, true])
  })

  it('resolves the mirrored case by vertical extent when the band is the TALLER (containing) shape', () => {
    // Same x-tie mechanism as above (both share the full [0,10] width,
    // tying the sampled scanline's enter/exit exactly), but with the
    // vertical-extent relationship reversed: here the "band" is taller
    // than the square, so the disambiguation's mirrored clause fires (X's
    // extent properly covers C's) rather than the primary one — the
    // shape playing "container" and the shape playing "band" have
    // swapped roles from the case above.
    const tallBand: Pt2[] = [
      [0, -5],
      [10, -5],
      [10, 15],
      [0, 15],
    ]
    const contours = [tallBand, square]
    expect(contourNestingDepth(contours, 0)).toBe(0) // the band is outermost here
    expect(contourNestingDepth(contours, 1)).toBe(1) // the square is nested inside it
    expect(classifyContourFill(contours)).toEqual([true, false])
  })
})

describe('contourNestingDepth — randomized cross-check against brute-force containment', () => {
  // Deterministic PRNG (mulberry32) — fixed seeds so this test is
  // reproducible, not flaky, while still exercising geometry no
  // hand-written fixture happens to cover.
  function mulberry32(seed: number): () => number {
    let s = seed >>> 0
    return () => {
      s = (s + 0x6d2b79f5) | 0
      let t = s
      t = Math.imul(t ^ (t >>> 15), t | 1)
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296
    }
  }

  function rectContour(x0: number, y0: number, x1: number, y1: number): Pt2[] {
    return [
      [x0, y0],
      [x1, y0],
      [x1, y1],
      [x0, y1],
    ]
  }

  /**
   * Builds a random FOREST of axis-aligned rectangle contours rooted at
   * `rect`: pushes `rect` itself, then — while size and depth budget allow
   * — splits `rect`'s inner area (after a margin) into 1-3 horizontal
   * slots, each shrunk by a gap, and recurses into each slot as a new root.
   * Every rectangle is either a STRICT ancestor/descendant of another (full
   * containment, margin > 0) or fully disjoint from it (gap > 0), OR —
   * occasionally — a FULL-WIDTH child: margin zero on the left and right
   * walls (it shares both exactly with its parent) while still shrunk on
   * top and bottom, so this generator keeps producing the reviewer's
   * confirmed equal-interval repro shape (a same-width band nested by
   * vertical extent alone) alongside the ordinary margin>0/gap>0 cases.
   * `bruteForceDepth` below is bbox-based specifically so this shared-wall
   * case (which `pointInPolygon`'s boundary-exclusive ray cast can't
   * resolve consistently) still has an unambiguous ground truth.
   */
  function buildForest(
    rng: () => number,
    rect: readonly [number, number, number, number],
    depth: number,
    maxDepth: number,
    out: Pt2[][],
  ): void {
    const [x0, y0, x1, y1] = rect
    out.push(rectContour(x0, y0, x1, y1))
    if (depth >= maxDepth) return
    const w = x1 - x0
    const h = y1 - y0
    if (w < 4 || h < 4) return // too small to subdivide further with a real margin/gap

    // Occasionally emit a single FULL-WIDTH child instead of the ordinary
    // margin>0 slots: zero margin on the left+right walls (shares both
    // exactly with `rect`), shrunk on top+bottom only. This is exactly the
    // reviewer's repro shape, generated instead of hand-written, so the
    // equal-interval scanline tie stays covered as this fixture evolves.
    if (rng() < 0.25) {
      const marginY = h * 0.15
      const innerY0 = y0 + marginY
      const innerY1 = y1 - marginY
      if (innerY1 > innerY0) {
        buildForest(rng, [x0, innerY0, x1, innerY1], depth + 1, maxDepth, out)
        return // this rect's only child is the full-width band, not ordinary siblings
      }
    }

    const margin = Math.min(w, h) * 0.15
    const innerX0 = x0 + margin
    const innerX1 = x1 - margin
    const innerY0 = y0 + margin
    const innerY1 = y1 - margin
    if (innerX1 <= innerX0 || innerY1 <= innerY0) return

    const n = 1 + Math.floor(rng() * 3) // 1..3 sibling slots
    const innerW = innerX1 - innerX0
    const slot = innerW / n
    for (let i = 0; i < n; i++) {
      const slotX0 = innerX0 + i * slot
      const slotX1 = innerX0 + (i + 1) * slot
      const gap = slot * 0.15
      const cx0 = slotX0 + gap
      const cx1 = slotX1 - gap
      if (cx1 <= cx0) continue
      buildForest(rng, [cx0, innerY0, cx1, innerY1], depth + 1, maxDepth, out)
    }
  }

  /** Every contour here is an axis-aligned rectangle (`rectContour`), so its
   *  bounding box IS its shape — reducing ground truth to bbox comparison
   *  sidesteps `pointInPolygon`'s boundary-exclusive ray cast, which can't
   *  consistently classify a vertex sitting exactly on another contour's
   *  wall (as the generator's occasional full-width child now does by
   *  construction). Ground truth: `C` properly contains `X` iff `C`'s bbox
   *  covers `X`'s on both axes with at least one strict inequality —
   *  valid here specifically because `buildForest` guarantees every pair
   *  of rectangles is either fully nested (by margin, or by a shared
   *  full-width wall) or fully disjoint (by gap), never partially
   *  overlapping. */
  function bruteForceDepth(contours: readonly Pt2[][], index: number): number {
    function bounds(contour: readonly Pt2[]): [number, number, number, number] {
      let minX = Infinity
      let maxX = -Infinity
      let minY = Infinity
      let maxY = -Infinity
      for (const [x, y] of contour) {
        if (x < minX) minX = x
        if (x > maxX) maxX = x
        if (y < minY) minY = y
        if (y > maxY) maxY = y
      }
      return [minX, maxX, minY, maxY]
    }

    const [tx0, tx1, ty0, ty1] = bounds(contours[index])
    let depth = 0
    for (let j = 0; j < contours.length; j++) {
      if (j === index) continue
      const [ox0, ox1, oy0, oy1] = bounds(contours[j])
      const properlyContains =
        ox0 <= tx0 && ox1 >= tx1 && oy0 <= ty0 && oy1 >= ty1 && (ox0 < tx0 || ox1 > tx1 || oy0 < ty0 || oy1 > ty1)
      if (properlyContains) depth++
    }
    return depth
  }

  const SEEDS = [1, 2, 3, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71]

  it(`interval-containment depth matches brute-force vertex containment across ${SEEDS.length} random non-crossing contour forests`, () => {
    let totalContours = 0
    for (const seed of SEEDS) {
      const rng = mulberry32(seed)
      const contours: Pt2[][] = []
      buildForest(rng, [0, 0, 100, 100], 0, 4, contours)
      expect(contours.length).toBeGreaterThan(1) // the generator should produce real structure, not just the root

      const fills = classifyContourFill(contours)
      for (let i = 0; i < contours.length; i++) {
        const expected = bruteForceDepth(contours, i)
        const actual = contourNestingDepth(contours, i)
        expect(actual).not.toBeNull()
        expect(actual).toBe(expected)
        expect(fills[i]).toBe(expected % 2 === 0)
      }
      totalContours += contours.length
    }
    // Sanity: the seeds above actually exercise multiple nesting levels,
    // not just a handful of top-level siblings.
    expect(totalContours).toBeGreaterThan(SEEDS.length * 2)
  })
})
