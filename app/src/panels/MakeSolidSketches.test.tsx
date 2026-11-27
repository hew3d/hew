/**
 * MakeSolidSketches — Object ▸ Make Solid… on a selection of sketches
 * (wireframe faces §6): which selections count, the success toast and
 * selection, and the decline's text and open-edge overlay. A plain object
 * stands in for the wasm Scene.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MakeSolidSketches, declineSentences, selectedSketchIds } from './MakeSolidSketches'
import type { NodeRef } from './treeModel'
import type { Scene as WasmScene } from '../wasm/loader'

function sceneWith(result: unknown, segments: number[] = [0, 0]) {
  const scene = {
    make_solid_from_sketches: vi.fn(() => JSON.stringify(result)),
    faces_open_edge_segments: vi.fn(() => new Float32Array(segments)),
  }
  return scene as unknown as WasmScene & typeof scene
}

function setup(scene: ReturnType<typeof sceneWith>, over: { docRev?: number } = {}) {
  const props = {
    sketchIds: [3n, 4n],
    docRev: over.docRev ?? 0,
    onClose: vi.fn(),
    onHealthOverlay: vi.fn(),
    onGeometryChanged: vi.fn(),
    onSelectObject: vi.fn(),
    onToast: vi.fn(),
  }
  const utils = render(<MakeSolidSketches scene={scene} {...props} />)
  return { ...utils, props, scene }
}

const run = () => fireEvent.click(screen.getByRole('button', { name: 'Make Solid' }))

/** Four open edges (one square), no non-planar loop. */
const SQUARE = [4, 0, ...Array.from({ length: 24 }, (_, i) => (i % 5) / 4)]

describe('selectedSketchIds — which selections the second input accepts', () => {
  it('takes whole sketches and the units of a free sketch', () => {
    const sel: NodeRef[] = [
      { kind: 'sketch', id: 7n },
      { kind: 'draft-edge', id: 99n, ctx: '{"sketch":12}', key: '0000000000000063' },
      { kind: 'draft-region', id: 98n, ctx: '{"sketch":12}', key: '0000000000000062' },
    ]
    expect(selectedSketchIds(sel)).toEqual([7n, 12n])
  })

  it('ignores Objects, groups, and drawing on a plane that is not a sketch', () => {
    const sel: NodeRef[] = [
      { kind: 'object', id: 1n },
      { kind: 'group', id: 2n },
      { kind: 'draft-edge', id: 5n, ctx: '{"plane":[0,0,1,0]}', key: '05' },
      { kind: 'draft-edge', id: 6n, ctx: 'not json', key: '06' },
    ]
    expect(selectedSketchIds(sel)).toEqual([])
  })
})

/** A plane listing with one rectangle island (key 0a, edges 01..04) drawn in
 *  free sketch 9, a region over it, and one rectangle on an Object's face. */
const PLANE_CTX = '{"plane":[0,0,0,0,0,1]}'
function planeScene() {
  const edge = (k: string, store: unknown) => ({ key: k, a: [0, 0, 0], b: [1, 0, 0], store, curve: null })
  const listing = {
    units: [
      { key: '000000000000000a', edges: ['01', '02', '03', '04'].map((k) => edge(k, { kind: 'free', sketch: 9 })), curves: [] },
      { key: '000000000000000b', edges: [edge('05', { kind: 'topology', object: 1 })], curves: [] },
    ],
    regions: [{ key: '00000000000000f1', outer: [], holes: [], over: null, extrude: { sketch: 9, region: 2 } }],
  }
  return { plane_units_json: vi.fn(() => JSON.stringify(listing)) } as unknown as Parameters<typeof selectedSketchIds>[1]
}

describe('selectedSketchIds — a plain drawn Rectangle', () => {
  it('resolves a marquee-style draft-island on a plane to its free sketch', () => {
    const sel: NodeRef[] = [{ kind: 'draft-island', id: 10n, ctx: PLANE_CTX, key: '000000000000000a' }]
    expect(selectedSketchIds(sel, planeScene())).toEqual([9n])
  })

  it('resolves Outliner-style edge and region rows on a plane', () => {
    const sel: NodeRef[] = [
      { kind: 'draft-edge', id: 1n, ctx: PLANE_CTX, key: '03' },
      { kind: 'draft-region', id: 0xf1n, ctx: PLANE_CTX, key: '00000000000000f1' },
    ]
    expect(selectedSketchIds(sel, planeScene())).toEqual([9n])
  })

  it('does not take drawing that lives in an Object, or an unknown key', () => {
    const sel: NodeRef[] = [
      { kind: 'draft-island', id: 11n, ctx: PLANE_CTX, key: '000000000000000b' },
      { kind: 'draft-island', id: 12n, ctx: PLANE_CTX, key: '00000000000000ee' },
    ]
    expect(selectedSketchIds(sel, planeScene())).toEqual([])
  })

  it('needs the scene to resolve a plane context', () => {
    const sel: NodeRef[] = [{ kind: 'draft-island', id: 10n, ctx: PLANE_CTX, key: '000000000000000a' }]
    expect(selectedSketchIds(sel)).toEqual([])
  })
})

describe('MakeSolidSketches — success', () => {
  it('calls the wasm with the selected sketches, toasts, selects the new Object, refreshes, and closes', () => {
    const { props, scene } = setup(sceneWith({ cells: [{ created: 21 }], created: 1, grew: 0, unjoined: 0, declined: 0 }))
    run()
    expect(scene.make_solid_from_sketches).toHaveBeenCalledWith(BigUint64Array.from([3n, 4n]))
    expect(props.onToast).toHaveBeenCalledTimes(1)
    expect(props.onToast).toHaveBeenCalledWith('The closed shape is now a solid.')
    expect(props.onSelectObject).toHaveBeenCalledWith(21n)
    expect(props.onGeometryChanged).toHaveBeenCalledTimes(1)
    expect(props.onClose).toHaveBeenCalledTimes(1)
    expect(props.onHealthOverlay).not.toHaveBeenCalled()
  })

  it('one toast, the refused join first, then growth, then creation', () => {
    const { props } = setup(
      sceneWith({
        cells: [{ created: 21 }, { grew: { from: 5, to: 6 } }, { unjoined: { object: 22, with: 6 } }],
        created: 1,
        grew: 1,
        unjoined: 1,
        declined: 0,
      }),
    )
    run()
    expect(props.onToast).toHaveBeenCalledTimes(1)
    expect(props.onToast).toHaveBeenCalledWith('The closed shape is now a solid of its own. It could not be joined to the one it touches.')
    expect(props.onSelectObject).toHaveBeenCalledWith(21n)
  })

  it('a grown solid selects the grown Object', () => {
    const { props } = setup(sceneWith({ cells: [{ grew: { from: 5, to: 6 } }], created: 0, grew: 1, unjoined: 0, declined: 0 }))
    run()
    expect(props.onToast).toHaveBeenCalledWith('The closed shape joined the solid it was drawn on.')
    expect(props.onSelectObject).toHaveBeenCalledWith(6n)
  })
})

const PLANE_OPENING =
  "Those edges don't lie flat on one plane, so a single face can't close them. Draw a line across the opening to split it into flat faces."

describe('MakeSolidSketches — a decline', () => {
  const notClosed = { cells: [{ declined: { reason: 'not_closed', open_edges: 4, non_planar_loops: 0 } }], created: 0, grew: 0, unjoined: 0, declined: 1 }

  it('says how many edges are open, draws them in the rim color, and changes nothing', () => {
    const { props, scene } = setup(sceneWith(notClosed, SQUARE), { docRev: 7 })
    run()
    expect(screen.getByText('Not closed yet. 4 open edges are marked in red.')).toBeInTheDocument()
    expect(scene.faces_open_edge_segments).toHaveBeenCalledWith(BigUint64Array.from([3n, 4n]))
    const overlay = props.onHealthOverlay.mock.calls.at(-1)![0]
    expect(overlay.segments.open).toHaveLength(24)
    expect(Array.from(overlay.segments.open)).toEqual(SQUARE.slice(2))
    expect(overlay.segments.warn).toHaveLength(0)
    expect(overlay.rev).toBe(7)
    expect(props.onToast).not.toHaveBeenCalled()
    expect(props.onSelectObject).not.toHaveBeenCalled()
    expect(props.onGeometryChanged).not.toHaveBeenCalled()
    expect(props.onClose).not.toHaveBeenCalled()
  })

  it('one open edge reads in the singular', () => {
    expect(declineSentences({ reason: 'not_closed', openEdges: 1, nonPlanarLoops: 0 })).toEqual(['Not closed yet. 1 open edge is marked in red.'])
  })

  it('marks every open edge red, including a non-flat loop\'s, and gives the loop a warn halo', () => {
    const flat = [1, 1, ...Array.from({ length: 6 + 18 }, (_, i) => i / 8)]
    const result = { cells: [{ declined: { reason: 'not_closed', open_edges: 1, non_planar_loops: 1 } }], created: 0, grew: 0, unjoined: 0, declined: 1 }
    const { props } = setup(sceneWith(result, flat))
    run()
    const overlay = props.onHealthOverlay.mock.calls.at(-1)![0]
    expect(overlay.segments.open).toHaveLength(24)
    expect(overlay.segments.warn).toHaveLength(18)
    expect(screen.getByText('Not closed yet. 1 open edge is marked in red.')).toBeInTheDocument()
    expect(screen.getByText(PLANE_OPENING)).toBeInTheDocument()
    expect(screen.queryByText(/do not lie on one plane/)).not.toBeInTheDocument()
  })

  it('a flat open loop (no non-planar loop) shows no plane sentence', () => {
    expect(declineSentences({ reason: 'not_closed', openEdges: 4, nonPlanarLoops: 0 })).toEqual(['Not closed yet. 4 open edges are marked in red.'])
  })

  it('open edges plus a non-flat loop show both sentences, open-edge first', () => {
    expect(declineSentences({ reason: 'not_closed', openEdges: 6, nonPlanarLoops: 1 })).toEqual([
      'Not closed yet. 6 open edges are marked in red.',
      PLANE_OPENING,
    ])
  })

  it('a non-planar loop with no open edges shows the plane sentence', () => {
    const result = { cells: [{ declined: { reason: 'non_planar', open_edges: 0, non_planar_loops: 1 } }], created: 0, grew: 0, unjoined: 0, declined: 1 }
    const { props } = setup(sceneWith(result, SQUARE))
    run()
    expect(props.onHealthOverlay).toHaveBeenCalled()
    expect(screen.getByText("These lines do not lie on one plane, so they can't form a face.")).toBeInTheDocument()
    expect(screen.queryByText(/Not closed yet/)).not.toBeInTheDocument()
  })

  it('overlapping faces say so in the agreed words and mark the overlap edges red', () => {
    const result = { cells: [{ declined: { reason: 'overlapping_faces', open_edges: 0, non_planar_loops: 0, overlapping_edges: 2 } }], created: 0, grew: 0, unjoined: 0, declined: 1 }
    // Pure-overlap header [0, 0] then two overlap edges (six floats each).
    const edges = [0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0]
    const { props } = setup(sceneWith(result, [0, 0, ...edges.slice(0, 12)]))
    run()
    expect(screen.getByText('Two faces lie on top of each other where the red marks are; erase one of them.')).toBeInTheDocument()
    const overlay = props.onHealthOverlay.mock.calls.at(-1)![0]
    // Both overlap edges are red (open) and carry the warn halo.
    expect(Array.from(overlay.segments.open)).toEqual(edges.slice(0, 12))
    expect(Array.from(overlay.segments.warn)).toEqual(edges.slice(0, 12))
  })

  it('overlapping faces with open edges present say both sentences', () => {
    const out = declineSentences({ reason: 'overlapping_faces', openEdges: 3, nonPlanarLoops: 0, overlappingEdges: 2 })
    expect(out).toEqual([
      'Not closed yet. 3 open edges are marked in red.',
      'Two faces lie on top of each other where the red marks are; erase one of them.',
    ])
  })

  it('other reasons get plain words, no jargon', () => {
    for (const reason of ['overlapping_faces', 'inside_solid', 'self_touching', 'invalid']) {
      const [s] = declineSentences({ reason, openEdges: 0, nonPlanarLoops: 0 })
      expect(s).not.toMatch(/manifold|topolog|watertight|mesh|_/i)
      expect(s.length).toBeGreaterThan(20)
    }
  })

  it('clears the overlay when an edit makes it stale, and when the window closes', () => {
    const { props, rerender, unmount, scene } = setup(sceneWith(notClosed, SQUARE))
    run()
    expect(props.onHealthOverlay).toHaveBeenCalledTimes(1)
    rerender(<MakeSolidSketches scene={scene} {...props} docRev={1} />)
    expect(props.onHealthOverlay).toHaveBeenLastCalledWith(null)
    expect(screen.queryByText(/Not closed yet/)).not.toBeInTheDocument()
    run()
    expect(props.onHealthOverlay.mock.calls.at(-1)![0]).not.toBeNull()
    unmount()
    expect(props.onHealthOverlay).toHaveBeenLastCalledWith(null)
  })

  it('Escape closes the window', () => {
    const { props } = setup(sceneWith(notClosed, SQUARE))
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })
})

describe('MakeSolidSketches — a stale sketch', () => {
  it('a kernel refusal is a toast and nothing else', () => {
    const scene = sceneWith({})
    scene.make_solid_from_sketches.mockImplementation(() => {
      throw new Error('UnknownSketch: gone')
    })
    const { props } = setup(scene)
    run()
    expect(props.onToast).toHaveBeenCalledTimes(1)
    expect(props.onClose).not.toHaveBeenCalled()
    expect(props.onHealthOverlay).not.toHaveBeenCalled()
  })
})
