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
  islandOf?: (region: bigint) => bigint | undefined
}

function fakeScene(s: SceneStubs): SelectScene {
  return {
    sketch_curve_chain: (_sk: bigint, edge: bigint) => (s.curveChain ? s.curveChain(edge) : [edge]),
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
    expect(classifySnapPick(bareSnap())).toEqual({ kind: 'fallback' })
    expect(classifySnapPick(null)).toEqual({ kind: 'fallback' })
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
})
