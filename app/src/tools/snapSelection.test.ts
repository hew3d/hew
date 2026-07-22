/**
 * snapSelection — the single, complete resolver both the Select click and the
 * drag-move arm reduce to. These tests drive `resolveSelectableRef` across the
 * matrix of (provenance × editing-context × depth), which IS the click/drag
 * parity: both callers pass their snap+ray through this one function, so a
 * shared result here is a shared result there. classifySnapPick is pinned too
 * (the provenance axis).
 */
import { describe, it, expect, vi } from 'vitest'
import { classifySnapPick, resolveSelectableRef, type ResolveDeps, type SelectScene } from './snapSelection'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

const base = { x: 0, y: 0, z: 0 }
const solidSnap = (object: bigint, instance?: bigint): Snap => ({ ...base, kind: 'on-face', object, instance })
const regionSnap = (sketch: bigint, region: bigint): Snap => ({ ...base, kind: 'on-face', elementKind: 'sketch-region', sketch, sketchRegion: region })
const edgeSnap = (sketch: bigint, edge: bigint): Snap => ({ ...base, kind: 'on-edge', elementKind: 'sketch-edge', sketch, element: edge })
/** A drawn curve's analytic point (center / quadrant / tangent / facet vertex
 * / polygon center): names the CHAIN, carries no `element`. `kind` is cosmetic
 * here — the resolver keys off `elementKind` + `sketchCurve`. */
const curveSnap = (sketch: bigint, curve: bigint, kind = 'center'): Snap => ({ ...base, kind, elementKind: 'sketch-curve', sketch, sketchCurve: curve })
const bareSnap = (): Snap => ({ ...base, kind: 'endpoint' })

/** A ray straight down -Z (dead-centre) unless a direction is given. */
const rayDir = (d: [number, number, number]): Ray => ({ origin: [0, 0, 5], direction: d })
const DOWN = rayDir([0, 0, -1])
const FORWARD: [number, number, number] = [0, 0, -1] // camera looks -Z

interface SceneStubs {
  region?: { sketch: bigint; region: bigint }
  edge?: { sketch: bigint; edge: bigint }
  face?: { object: bigint; instance?: bigint; depth: number }
  curveChain?: (edge: bigint) => bigint[]
  curveEdges?: (curve: bigint) => bigint[]
  islandOf?: (region: bigint) => bigint | undefined
}

function fakeScene(s: SceneStubs): SelectScene {
  return {
    sketch_curve_chain: (_sk: bigint, edge: bigint) => (s.curveChain ? s.curveChain(edge) : [edge]),
    sketch_curve_edges: (_sk: bigint, curve: bigint) => (s.curveEdges ? s.curveEdges(curve) : []),
    sketch_region_island: (_sk: bigint, region: bigint) => (s.islandOf ? s.islandOf(region) : 900n),
    pick_sketch_region: () =>
      s.region && { sketch: () => s.region!.sketch, region: () => s.region!.region, free: vi.fn() },
    pick_sketch_edge: () =>
      s.edge && { sketch: () => s.edge!.sketch, edge: () => s.edge!.edge, free: vi.fn() },
    pick_face: () =>
      s.face && {
        object: () => s.face!.object,
        instance: () => s.face!.instance,
        depth: () => s.face!.depth,
        free: vi.fn(),
      },
  } as unknown as SelectScene
}

function deps(scene: SelectScene, context: NodeRef[], far = 100): ResolveDeps {
  return {
    scene,
    context,
    // The Viewport's resolveObject is context-scoped; the stub tags the object
    // so tests can tell it apart from a sketch ref.
    resolveObject: (object, instance) => ({ kind: 'object', id: object, instance } as unknown as NodeRef),
    cameraForward: FORWARD,
    cameraFar: far,
  }
}

const GROUP_CTX: NodeRef[] = [{ kind: 'group', id: 1n } as unknown as NodeRef]

describe('classifySnapPick', () => {
  it('maps each provenance (and null) to its kind', () => {
    expect(classifySnapPick(solidSnap(7n, 9n))).toEqual({ kind: 'object', object: 7n, instance: 9n })
    expect(classifySnapPick(regionSnap(21n, 5n))).toEqual({ kind: 'sketch-region', sketch: 21n, region: 5n })
    expect(classifySnapPick(edgeSnap(11n, 4n))).toEqual({ kind: 'sketch-edge', sketch: 11n, edge: 4n })
    expect(classifySnapPick(curveSnap(11n, 8n))).toEqual({ kind: 'sketch-curve', sketch: 11n, curve: 8n })
    expect(classifySnapPick(bareSnap())).toEqual({ kind: 'fallback' })
    expect(classifySnapPick(null)).toEqual({ kind: 'fallback' })
  })

  it('a sketch-curve snap missing its curve handle degrades to fallback, never a half-built pick', () => {
    // Defensive: element_kind said "sketch-curve" but the handle is absent.
    expect(classifySnapPick({ ...base, kind: 'center', elementKind: 'sketch-curve', sketch: 11n })).toEqual({ kind: 'fallback' })
  })
})

describe('resolveSelectableRef — provenance × context × depth', () => {
  it('a solid snap → the object node, at top level and in a context', () => {
    const scene = fakeScene({})
    expect(resolveSelectableRef(solidSnap(7n, 9n), DOWN, deps(scene, []))).toMatchObject({ kind: 'object', id: 7n })
    expect(resolveSelectableRef(solidSnap(7n), DOWN, deps(scene, GROUP_CTX))).toMatchObject({ kind: 'object', id: 7n })
  })

  it('a region-fill snap → its island at top level', () => {
    const scene = fakeScene({ islandOf: () => 900n })
    expect(resolveSelectableRef(regionSnap(21n, 5n), DOWN, deps(scene, []))).toEqual({ kind: 'sketch-island', id: 900n, sketch: 21n })
  })

  it('a region-fill snap INSIDE a context is out of scope → the in-context solid, never the sketch (FIX 1)', () => {
    // The top-level sketch region is not selectable in-context; the fallback
    // resolves the in-context solid under the ray instead.
    const scene = fakeScene({ face: { object: 42n, depth: 8 } })
    const ref = resolveSelectableRef(regionSnap(21n, 5n), DOWN, deps(scene, GROUP_CTX))
    expect(ref).toMatchObject({ kind: 'object', id: 42n })
  })

  it('a sketch-edge snap → the edge (or its curve) at top level', () => {
    const straight = fakeScene({ curveChain: (e) => [e] })
    expect(resolveSelectableRef(edgeSnap(11n, 4n), DOWN, deps(straight, []))).toEqual({ kind: 'sketch-edge', id: 4n, sketch: 11n })
    const curve = fakeScene({ curveChain: () => [2n, 3n, 4n] })
    expect(resolveSelectableRef(edgeSnap(11n, 4n), DOWN, deps(curve, []))).toEqual({ kind: 'sketch-curve', id: 2n, sketch: 11n })
  })

  it('a sketch-edge snap INSIDE a context is out of scope → the in-context solid, never the sketch (FIX 1)', () => {
    const scene = fakeScene({ face: { object: 42n, depth: 8 } })
    expect(resolveSelectableRef(edgeSnap(11n, 4n), DOWN, deps(scene, GROUP_CTX))).toMatchObject({ kind: 'object', id: 42n })
  })

  it('a provenance-less snap falls back to region → edge → solid, top-level only for sketches', () => {
    // Region under the ray wins.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ region: { sketch: 21n, region: 5n } }), []))).toEqual({ kind: 'sketch-island', id: 900n, sketch: 21n })
    // No region → a sketch edge.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ edge: { sketch: 11n, edge: 4n } }), []))).toEqual({ kind: 'sketch-edge', id: 4n, sketch: 11n })
    // No sketch geometry → a near solid.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ face: { object: 42n, depth: 50 } }), []))).toMatchObject({ kind: 'object', id: 42n })
    // Genuinely empty → null.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({}), []))).toBeNull()
    // In a context, sketch pickers are skipped — only the in-context solid.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ region: { sketch: 21n, region: 5n }, face: { object: 42n, depth: 8 } }), GROUP_CTX))).toMatchObject({ kind: 'object', id: 42n })
  })

  it('the fallback solid pick is bounded by the AXIAL far plane, not the radial distance (FIX 2)', () => {
    // Off-centre ray (cos 0.6 with the camera forward): a VISIBLE solid at
    // RADIAL 150 sits at AXIAL 90 < far 100 → armed (the old radial-vs-far
    // bound wrongly rejected it, silently turning the drag into a marquee).
    const offCentre = rayDir([0.8, 0, -0.6]) // unit; dot with [0,0,-1] = 0.6
    const visible = fakeScene({ face: { object: 42n, depth: 150 } })
    expect(resolveSelectableRef(bareSnap(), offCentre, deps(visible, [], 100))).toMatchObject({ kind: 'object', id: 42n })

    // A genuinely-clipped solid: radial 200 → axial 120 > far 100 → rejected.
    const clipped = fakeScene({ face: { object: 42n, depth: 200 } })
    expect(resolveSelectableRef(bareSnap(), offCentre, deps(clipped, [], 100))).toBeNull()

    // Dead-centre is unchanged: radial == axial.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ face: { object: 42n, depth: 99 } }), [], 100))).toMatchObject({ kind: 'object', id: 42n })
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ face: { object: 42n, depth: 101 } }), [], 100))).toBeNull()
  })

  it('the far bound is UNIFORM: a solid beyond the render far plane is unselectable by click too, not just unmovable', () => {
    // A provenance-less snap whose nearest solid is beyond the far plane. This
    // is the exact call the Select CLICK makes (handleSelect → resolveSelectableRef).
    // Both click and drag share it, so both reject: an undrawn solid must be
    // neither selected nor moved — a selection is the precursor to Delete/Move
    // on geometry the user cannot see. Do NOT re-introduce a click-only exemption:
    // pick_face returns the NEAREST face, so a visible solid (a nearer face) is
    // never rejected here — only a pixel with no drawn solid at all.
    const clipped = deps(fakeScene({ face: { object: 42n, depth: 101 } }), [], 100)
    expect(resolveSelectableRef(bareSnap(), DOWN, clipped)).toBeNull()
    // A drawn solid at the same pixel (nearer face, within far) still selects.
    const visible = deps(fakeScene({ face: { object: 42n, depth: 99 } }), [], 100)
    expect(resolveSelectableRef(bareSnap(), DOWN, visible)).toMatchObject({ kind: 'object', id: 42n })
  })

  it('parity: click and drag get the SAME ref — they are one function of (snap, ray, deps)', () => {
    const cases: { snap: Snap | null; ray: Ray; ctx: NodeRef[]; scene: () => SelectScene }[] = [
      { snap: solidSnap(7n, 9n), ray: DOWN, ctx: [], scene: () => fakeScene({}) },
      { snap: regionSnap(21n, 5n), ray: DOWN, ctx: [], scene: () => fakeScene({}) },
      { snap: regionSnap(21n, 5n), ray: DOWN, ctx: GROUP_CTX, scene: () => fakeScene({ face: { object: 42n, depth: 8 } }) },
      { snap: edgeSnap(11n, 4n), ray: DOWN, ctx: [], scene: () => fakeScene({}) },
      { snap: bareSnap(), ray: rayDir([0.8, 0, -0.6]), ctx: [], scene: () => fakeScene({ face: { object: 42n, depth: 150 } }) },
      { snap: null, ray: DOWN, ctx: [], scene: () => fakeScene({}) },
    ]
    for (const c of cases) {
      // The click path and the drag path each call resolveSelectableRef with
      // the same inputs → identical result, by construction (fresh scene stubs
      // per call, since the pick handles are single-use).
      const clickRef = resolveSelectableRef(c.snap, c.ray, deps(c.scene(), c.ctx))
      const dragRef = resolveSelectableRef(c.snap, c.ray, deps(c.scene(), c.ctx))
      expect(clickRef).toEqual(dragRef)
    }
  })

  it('a sketch-curve snap → the curve ref (through its representative edge)', () => {
    // A polygon center has no rim, so its chain is its edges; the resolver
    // routes through the lowest-id edge, which the chain canonicalizes to a
    // sketch-curve ref (chain length > 1).
    const scene = fakeScene({ curveEdges: () => [30n, 31n, 32n], curveChain: () => [30n, 31n, 32n] })
    expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(scene, []))).toEqual({ kind: 'sketch-curve', id: 30n, sketch: 11n })
  })

  it('a sketch-curve snap INSIDE a context is out of scope → the in-context solid, never the curve', () => {
    const scene = fakeScene({ curveEdges: () => [30n], face: { object: 42n, depth: 8 } })
    expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(scene, GROUP_CTX))).toMatchObject({ kind: 'object', id: 42n })
  })

  it('a sketch-curve snap whose edges are all gone falls through to the ray re-probe, not a dead ref', () => {
    // The curve was deleted between snap and resolve: no live edges. The
    // resolver must not mint a ref from the stale handle — it re-probes.
    const scene = fakeScene({ curveEdges: () => [], region: { sketch: 21n, region: 5n } })
    expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(scene, []))).toEqual({ kind: 'sketch-island', id: 900n, sketch: 21n })
  })

  it('a curve split into disjoint chains resolves its centre to the LOWEST-id edge deterministically, regardless of curve_edges order', () => {
    // A line drawn across a circle splits its facets into two chains that
    // still share the curve id. `sketch_curve_edges` returns slotmap order,
    // NOT id order — so curveRef must pick the lowest id itself. Here the
    // curve's edges come back unsorted ([5,2,8,3]); edges {2,3} are one chain
    // and {5,8} the other. The centre must resolve to the chain holding edge
    // 2 (the global lowest), via that chain's ascending representative — the
    // SAME ref a rim click on that chain gives — never an arbitrary slotmap
    // first element.
    const chainOf = (e: bigint): bigint[] => (e === 2n || e === 3n ? [2n, 3n] : [5n, 8n])
    const scene = () => fakeScene({ curveEdges: () => [5n, 2n, 8n, 3n], curveChain: chainOf })
    expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(scene(), []))).toEqual({ kind: 'sketch-curve', id: 2n, sketch: 11n })
    // A rim click on that same (lowest-id) chain agrees exactly.
    expect(resolveSelectableRef(edgeSnap(11n, 3n), DOWN, deps(scene(), []))).toEqual({ kind: 'sketch-curve', id: 2n, sketch: 11n })
  })

  it('REGRESSION: walking a drawn circle by angle, EVERY snap the rim produces selects the SAME curve — including the centre, the quadrants, the tangents, and the facet vertices', () => {
    // The reported bug: a click that resolved to a Center or Quadrant snap
    // (and, at facet vertices, an Endpoint) carried no selectable provenance,
    // so it fell through to a ray re-probe and selected the sketch ISLAND (no
    // Segments field) instead of the curve — about half the clicks. Here one
    // drawn circle is sketch 11, curve 8, whose facet edges are 30..=41; a
    // rim edge snap names the edge it hit, an analytic-point snap names the
    // curve. Both must resolve to the ONE curve ref, at every angle.
    const CURVE_EDGES = Array.from({ length: 12 }, (_v, i) => 30n + BigInt(i))
    const scene = () => fakeScene({
      // A rim edge belongs to the circle's chain → sketch-curve ref at its rep.
      curveChain: () => CURVE_EDGES,
      curveEdges: () => CURVE_EDGES,
    })
    // The canonical ref a rim-EDGE click yields, which every analytic-point
    // click must match exactly (same id, same kind).
    const expected = { kind: 'sketch-curve', id: 30n, sketch: 11n }

    // Analytic points around the rim: centre, four quadrants, two tangents,
    // and several facet vertices — every snap kind the rim can produce that
    // used to be provenance-less.
    const analytic: Snap[] = [
      curveSnap(11n, 8n, 'center'),
      ...['quadrant', 'quadrant', 'quadrant', 'quadrant'].map((k) => curveSnap(11n, 8n, k)),
      ...['tangent', 'tangent'].map((k) => curveSnap(11n, 8n, k)),
      ...CURVE_EDGES.map(() => curveSnap(11n, 8n, 'endpoint')),
    ]
    for (const snap of analytic) {
      expect(resolveSelectableRef(snap, DOWN, deps(scene(), []))).toEqual(expected)
    }
    // And a rim-EDGE snap (the ~half of clicks that always worked) lands on
    // the identical ref — so the two halves of the rim never disagree.
    for (const e of CURVE_EDGES) {
      expect(resolveSelectableRef(edgeSnap(11n, e), DOWN, deps(scene(), []))).toEqual(expected)
    }
  })
})
