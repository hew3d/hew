/**
 * ObjectInfoPanel — draft-ref (the drawing on a plane — planeUnits.ts)
 * selection: one kind word (Shape / Curve / Line / Region) with a subtitle for
 * where it is drawn (on an object, on the ground, or just "drawing"), a Points
 * row, and — for a selection made only of facets of ONE circle — an editable
 * Segments field that re-facets the circle through a draft. It is checked
 * BEFORE the whole-sketch branch it would otherwise fall into (a draft ref
 * carries no sketch handle).
 *
 * A plain JS object stands in for the wasm Scene (the panel imports the type
 * only, erased at runtime — the same convention ComponentsPanel.test.tsx /
 * scenePanels.test.tsx use), so no wasm/loader mock is required.
 */
import { render, screen, fireEvent, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ObjectInfoPanel } from './ObjectInfoPanel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from './treeModel'
import { setLengthUnit, formatLength } from '../settings/units'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function makeScene(overrides: Record<string, any> = {}): WasmScene {
  return {
    top_level_nodes: () => [],
    group_members: () => [],
    object_name: () => undefined,
    object_drawing_planes: () => new Float64Array(0),
    ...overrides,
  } as unknown as WasmScene
}

const baseProps = {
  docRev: 0,
  onNameChanged: vi.fn(),
  onGeometryChanged: vi.fn(),
  onTagsChanged: vi.fn(),
  onSelectMany: vi.fn(),
}


// ---------------------------------------------------------------------------
// A plane-listing double. Every plane is described by its canonical context
// string; `plane_units_json` lists the units on it, and `pick_plane_unit`
// answers the kernel's curve key for a circle facet from its midpoint.
// ---------------------------------------------------------------------------

type Pt = [number, number, number]
const hex = (n: number): string => n.toString(16).padStart(16, '0')
interface PEdge { key: number; a: Pt; b: Pt; store: object; circle?: boolean; polygon?: boolean }
interface PUnit { key: number; edges: PEdge[]; /** Circle radius listed as null (a stretched instance). */ ellipse?: boolean }
interface PRegion { key: number; outer: Pt[] }
interface PState { units: PUnit[]; regions?: PRegion[] }

const CURVE = 0xc1
const RUN = 0xd1
const GROUND = '{"plane":[0,0,0,0,0,1]}'
const FACE = '{"plane":[0,0,0.5,0,0,1]}'
const topo = (object: number) => ({ kind: 'topology', object })
const free = (sketch: number) => ({ kind: 'free', sketch })

/** A closed square of four plain lines, keys base..base+3. */
function square(base: number, z: number, store: object): PEdge[] {
  const c: Pt[] = [[0, 0, z], [1, 0, z], [1, 1, z], [0, 1, z]]
  return c.map((p, i) => ({ key: base + i, a: p, b: c[(i + 1) % 4], store }))
}
/** A circle of `n` facet edges, keys base..base+n-1. */
function circleFacets(n: number, base: number, z: number, store: object): PEdge[] {
  return Array.from({ length: n }, (_v, i) => {
    const t0 = (2 * Math.PI * i) / n
    const t1 = (2 * Math.PI * (i + 1)) / n
    return {
      key: base + i,
      a: [Math.cos(t0), Math.sin(t0), z] as Pt,
      b: [Math.cos(t1), Math.sin(t1), z] as Pt,
      store,
      circle: true,
    }
  })
}

/** The curve RUNS of a unit: its circle facets form one run (curve key CURVE),
 *  its polygon sides another (curve key CURVE + 1). */
function curvesOf(u: PUnit): object[] {
  const out: object[] = []
  const circ = u.edges.filter((e) => e.circle)
  const poly = u.edges.filter((e) => e.polygon)
  if (circ.length > 0) {
    out.push({ key: hex(RUN), curve_key: hex(CURVE), kind: 'circle', center: [0, 0, circ[0].a[2]], radius: u.ellipse === true ? null : 1, edges: circ.map((e) => hex(e.key)) })
  }
  if (poly.length > 0) {
    out.push({ key: hex(RUN + 1), curve_key: hex(CURVE + 1), kind: 'polygon', center: [0, 0, poly[0].a[2]], radius: 1, edges: poly.map((e) => hex(e.key)) })
  }
  return out
}

function planeFake(byCtx: Record<string, PState>) {
  const json = (st: PState): string =>
    JSON.stringify({
      units: st.units.map((u) => ({
        key: hex(u.key),
        edges: u.edges.map((e) => ({
          key: hex(e.key),
          a: e.a,
          b: e.b,
          store: e.store,
          curve: e.circle ? { center: [0, 0, e.a[2]], radius: u.ellipse === true ? null : 1 } : null,
        })),
        curves: curvesOf(u),
      })),
      regions: (st.regions ?? []).map((r) => ({ key: hex(r.key), outer: r.outer, holes: [], over: null, extrude: null })),
    })
  return {
    plane_units_json: (ctx: string) => {
      const st = byCtx[ctx]
      if (st === undefined) throw new Error('StalePlane')
      return json(st)
    },
    pick_plane_unit: (ctx: string, ray: Float64Array) => {
      const st = byCtx[ctx]
      if (st === undefined) return 'null'
      for (const u of st.units) for (const e of u.edges) {
        const mx = (e.a[0] + e.b[0]) / 2
        const my = (e.a[1] + e.b[1]) / 2
        if (Math.abs(mx - ray[0]) < 1e-9 && Math.abs(my - ray[1]) < 1e-9) {
          return JSON.stringify({ edge: hex(e.key), unit: hex(u.key), curve: e.circle ? hex(CURVE) : null })
        }
      }
      return 'null'
    },
  }
}

const draftRef = (kind: NodeRef['kind'], ctx: string, key: number): NodeRef => ({ kind, id: BigInt(key), ctx, key: hex(key) })

const panelScene = (byCtx: Record<string, PState>, extra: Record<string, unknown> = {}) =>
  makeScene({
    top_level_nodes: () => [{ kind: 'object', id: 1n }],
    object_name: (id: bigint) => (id === 1n ? 'Panel' : undefined),
    ...planeFake(byCtx),
    ...extra,
  })

describe('ObjectInfoPanel — drawn-thing selection', () => {
  const onFace: Record<string, PState> = {
    [FACE]: {
      units: [{ key: 10, edges: square(1, 0.5, topo(1)) }],
      regions: [{ key: 30, outer: [[0, 0, 0.5], [1, 0, 0.5], [1, 1, 0.5], [0, 1, 0.5]] }],
    },
  }

  it('a Shape on an object: Type "Shape", store "on Panel", Name "Shape on Panel", a Points row', () => {
    const scene = panelScene(onFace)
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 10)]} />)
    expect(screen.getByText('Shape')).toBeInTheDocument()
    expect(screen.getByTestId('object-info-store')).toHaveTextContent('on Panel')
    expect(screen.getByText('Shape on Panel')).toBeInTheDocument()
    expect(screen.getByText('Points')).toBeInTheDocument()
    expect(screen.getByText('4')).toBeInTheDocument() // the square's 4 vertices
  })

  it('a Line on an object: Type "Line", Name "Line on Panel", 2 points', () => {
    const scene = panelScene(onFace)
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-edge', FACE, 2)]} />)
    expect(screen.getByText('Line')).toBeInTheDocument()
    expect(screen.getByText('Line on Panel')).toBeInTheDocument()
    expect(screen.getByText('2')).toBeInTheDocument() // the edge's 2 vertices
  })

  it('a Line shows a Length row in the display units; a 0.0106 m line reads 10.6 mm like the measurement box', () => {
    setLengthUnit('mm')
    try {
      const scene = panelScene({
        [FACE]: { units: [{ key: 10, edges: [{ key: 2, a: [0, 0, 0.5], b: [0.0106, 0, 0.5], store: topo(1) }] }] },
      })
      render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-edge', FACE, 2)]} />)
      expect(screen.getByText('Length')).toBeInTheDocument()
      expect(screen.getByTestId('object-info-length')).toHaveTextContent(formatLength(0.0106))
      expect(screen.getByTestId('object-info-length').textContent).toBe('10.6 mm')
    } finally {
      setLengthUnit('m')
    }
  })

  it('a Curve shows Length as the sum of its edges; a Shape and a Region show none', () => {
    const edges = circleFacets(24, 100, 0.5, topo(1))
    const scene = panelScene({
      [FACE]: {
        units: [{ key: 10, edges }, { key: 11, edges: square(1, 0.5, topo(1)) }],
        regions: [{ key: 30, outer: [[0, 0, 0.5], [1, 0, 0.5], [1, 1, 0.5], [0, 1, 0.5]] }],
      },
    })
    const sum = edges.reduce((n, e) => n + Math.hypot(e.b[0] - e.a[0], e.b[1] - e.a[1], e.b[2] - e.a[2]), 0)
    const view = render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-curve', FACE, CURVE)]} />)
    expect(screen.getByTestId('object-info-length')).toHaveTextContent(formatLength(sum))
    view.rerender(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 11)]} />)
    expect(screen.queryByText('Length')).not.toBeInTheDocument()
    view.rerender(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-region', FACE, 30)]} />)
    expect(screen.queryByText('Length')).not.toBeInTheDocument()
  })

  it('a Region on an object: Type "Region", Name "Region on Panel"', () => {
    const scene = panelScene(onFace)
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-region', FACE, 30)]} />)
    expect(screen.getByText('Region')).toBeInTheDocument()
    expect(screen.getByText('Region on Panel')).toBeInTheDocument()
  })

  it('a Curve (a circle) on an object: Type "Curve", Name "Curve on Panel"', () => {
    const scene = panelScene({ [FACE]: { units: [{ key: 10, edges: circleFacets(24, 100, 0.5, topo(1)) }] } })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-curve', FACE, CURVE)]} />)
    expect(screen.getByText('Curve')).toBeInTheDocument()
    expect(screen.getByText('Curve on Panel')).toBeInTheDocument()
    expect(screen.getByTestId('object-info-store')).toHaveTextContent('on Panel')
  })

  it('a drawing on the ground reads "on the ground"; any other free drawing reads "drawing"', () => {
    const OTHER = '{"plane":[0,0,2,0,0,1]}'
    const scene = panelScene({
      [GROUND]: { units: [{ key: 10, edges: square(1, 0, free(5)) }] },
      [OTHER]: { units: [{ key: 11, edges: square(1, 2, free(6)) }] },
    })
    const view = render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', GROUND, 10)]} />)
    expect(screen.getByTestId('object-info-store')).toHaveTextContent('on the ground')
    expect(screen.getByText('Shape on the ground')).toBeInTheDocument()
    view.rerender(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', OTHER, 11)]} />)
    expect(screen.getByTestId('object-info-store')).toHaveTextContent('drawing')
    expect(screen.getByText('Shape drawing')).toBeInTheDocument()
  })

  it('a mixed shape (topology + free edges) is labelled by its object', () => {
    const edges = square(1, 0, topo(1))
    edges[2].store = free(5)
    edges[3].store = free(5)
    const scene = panelScene({ [GROUND]: { units: [{ key: 10, edges }] } })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', GROUND, 10)]} />)
    expect(screen.getByTestId('object-info-store')).toHaveTextContent('on Panel')
  })

  it('renders only the kind label — no Points row, no store — for a stale ref', () => {
    const scene = panelScene({ [FACE]: { units: [] } }) // the shape is gone (pushed, dissolved, re-cut)
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 999)]} />)
    expect(screen.getAllByText('Shape').length).toBeGreaterThan(0)
    expect(screen.queryByText('Points')).not.toBeInTheDocument()
    expect(screen.queryByTestId('object-info-store')).not.toBeInTheDocument()
  })

  it('renders nothing editable for a drawn shape — no Tags section, no Segments', () => {
    const scene = panelScene(onFace)
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 10)]} />)
    expect(screen.queryByText('Tags')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Add tag')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Segments')).not.toBeInTheDocument()
  })
})

describe('ObjectInfoPanel — Segments of a circle drawn on a face', () => {
  const circleState = (n = 24): Record<string, PState> => ({
    [FACE]: { units: [{ key: 10, edges: circleFacets(n, 100, 0.5, topo(1)) }] },
  })
  /** A scene whose `draft_refacet_curve` rebuilds the circle at n facets under
   *  a NEW unit key, the way the kernel replaces them. */
  function refacetScene(state: Record<string, PState>, refacet?: () => void) {
    const draftBegin = vi.fn(() => 7n)
    const draftRefacet = vi.fn((_d: bigint, _curve: bigint, n: number) => {
      if (refacet) return refacet()
      state[FACE] = { units: [{ key: 99, edges: circleFacets(n, 500, 0.5, topo(1)) }] }
    })
    const draftEnd = vi.fn(() => '{"objects":[1],"sketches":[],"kept_as_sketch":[]}')
    const draftCancel = vi.fn()
    const scene = panelScene(state, {
      draft_begin_ctx: draftBegin,
      draft_refacet_curve: draftRefacet,
      draft_end: draftEnd,
      draft_cancel: draftCancel,
    })
    return { scene, draftBegin, draftRefacet, draftEnd, draftCancel }
  }
  const CURVE_SEL = [draftRef('draft-curve', FACE, CURVE)]

  it('shows the circle\'s current facet count in the Segments field, for a curve, a shape, or one facet of it', () => {
    const scene = panelScene(circleState(24))
    const view = render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={CURVE_SEL} />)
    expect((screen.getByLabelText('Segments') as HTMLInputElement).value).toBe('24')
    view.rerender(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 10)]} />)
    expect((screen.getByLabelText('Segments') as HTMLInputElement).value).toBe('24')
    view.rerender(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-edge', FACE, 105)]} />)
    expect((screen.getByLabelText('Segments') as HTMLInputElement).value).toBe('24')
  })

  it('shows no Segments field for edges with no circle (a plain line or polygon side)', () => {
    const scene = panelScene({ [FACE]: { units: [{ key: 10, edges: square(1, 0.5, topo(1)) }] } })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 10)]} />)
    expect(screen.queryByLabelText('Segments')).not.toBeInTheDocument()
  })

  it('shows no Segments field for a null-radius circle (an ellipse in a stretched instance)', () => {
    const scene = panelScene({ [FACE]: { units: [{ key: 10, edges: circleFacets(24, 100, 0.5, topo(1)), ellipse: true }] } })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={CURVE_SEL} />)
    expect(screen.queryByLabelText('Segments')).not.toBeInTheDocument()
  })

  it('shows no Segments field for a regular polygon (a polygon run has no facet count)', () => {
    const edges: PEdge[] = square(1, 0.5, topo(1)).map((e) => ({ ...e, polygon: true }))
    const scene = panelScene({ [FACE]: { units: [{ key: 10, edges }] } })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 10)]} />)
    expect(screen.queryByLabelText('Segments')).not.toBeInTheDocument()
  })

  it('shows no Segments field for a shape that mixes circle facets with plain lines', () => {
    const edges = [...circleFacets(24, 100, 0.5, topo(1)), ...square(1, 0.5, topo(1))]
    const scene = panelScene({ [FACE]: { units: [{ key: 10, edges }] } })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[draftRef('draft-island', FACE, 10)]} />)
    expect(screen.queryByLabelText('Segments')).not.toBeInTheDocument()
  })

  it('re-facets through ONE draft on the circle\'s plane and re-points a curve selection at the same key', () => {
    const { scene, draftBegin, draftRefacet, draftEnd } = refacetScene(circleState(24))
    const onSelectMany = vi.fn()
    const onGeometryChanged = vi.fn()
    const onNameChanged = vi.fn()
    render(
      <ObjectInfoPanel {...baseProps} onSelectMany={onSelectMany} onGeometryChanged={onGeometryChanged} onNameChanged={onNameChanged} scene={scene} selectedIds={CURVE_SEL} />,
    )
    const input = screen.getByLabelText('Segments')
    fireEvent.change(input, { target: { value: '48' } })
    fireEvent.blur(input)

    expect(draftBegin).toHaveBeenCalledWith(FACE, 'Segments')
    expect(draftRefacet).toHaveBeenCalledWith(7n, BigInt(CURVE), 48)
    expect(draftEnd).toHaveBeenCalledWith(7n)
    // A curve keeps its key through the rebuild (centre and radius unchanged).
    expect(onSelectMany).toHaveBeenCalledWith([draftRef('draft-curve', FACE, CURVE)])
    expect(onGeometryChanged).toHaveBeenCalledTimes(1)
    expect(onNameChanged).not.toHaveBeenCalled()
  })

  it('a selected shape is re-pointed at the island holding the circle in the FRESH listing', () => {
    const { scene } = refacetScene(circleState(24))
    const onSelectMany = vi.fn()
    render(<ObjectInfoPanel {...baseProps} onSelectMany={onSelectMany} scene={scene} selectedIds={[draftRef('draft-island', FACE, 10)]} />)
    const input = screen.getByLabelText('Segments')
    fireEvent.change(input, { target: { value: '36' } })
    fireEvent.blur(input)
    // The old unit (key 10) is gone; the rebuilt circle lives in unit 99.
    expect(onSelectMany).toHaveBeenCalledWith([draftRef('draft-island', FACE, 99)])
  })

  it('Escape reverts the field to the current count without committing', () => {
    const { scene, draftBegin } = refacetScene(circleState(24))
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={CURVE_SEL} />)
    const input = screen.getByLabelText('Segments') as HTMLInputElement
    fireEvent.change(input, { target: { value: '48' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(input.value).toBe('24')
    expect(draftBegin).not.toHaveBeenCalled()
  })

  it('refuses a count below the floor visibly, without touching the kernel', () => {
    const { scene, draftBegin, draftRefacet } = refacetScene(circleState(24))
    const onToast = vi.fn()
    render(<ObjectInfoPanel {...baseProps} onToast={onToast} scene={scene} selectedIds={CURVE_SEL} />)
    const input = screen.getByLabelText('Segments') as HTMLInputElement
    fireEvent.change(input, { target: { value: '3' } })
    fireEvent.blur(input)
    expect(draftBegin).not.toHaveBeenCalled()
    expect(draftRefacet).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('SegmentsBelowFloor')
    expect(input.value).toBe('24')
  })

  it('surfaces a kernel refusal as a toast, cancels the draft, and restores the field', () => {
    const { scene, draftCancel } = refacetScene(circleState(24), () => {
      throw new Error('CurveNotRefacetable: only a whole circle')
    })
    const onToast = vi.fn()
    const onSelectMany = vi.fn()
    render(<ObjectInfoPanel {...baseProps} onToast={onToast} onSelectMany={onSelectMany} scene={scene} selectedIds={CURVE_SEL} />)
    const input = screen.getByLabelText('Segments') as HTMLInputElement
    fireEvent.change(input, { target: { value: '96' } })
    fireEvent.blur(input)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('CurveNotRefacetable')
    expect(draftCancel).toHaveBeenCalledWith(7n)
    expect(onSelectMany).not.toHaveBeenCalled()
    expect(input.value).toBe('24')
  })
})

describe('ObjectInfoPanel — annotation selection', () => {
  it('shows a selected leader text with its type and tag, read-only', () => {
    const scene = makeScene({
      annotation_kind: () => 'leader',
      annotation_text: () => 'Check clearance',
      annotation_tags: () => ['Notes/Review'],
    })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[]} selectedAnnotation={7n} />)
    expect(screen.getByText('Check clearance')).toBeTruthy()
    expect(screen.getByText('Text')).toBeTruthy()
    expect(screen.getByText('Notes / Review')).toBeTruthy()
    expect(screen.queryByLabelText('Add tag')).toBeNull()
  })
})

describe('ObjectInfoPanel — guide selection', () => {
  it('shows a selected guide point read-only', () => {
    const scene = makeScene({ guide_kind: () => 'point', guide_tags: () => [] })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[]} selectedGuide={7n} />)
    expect(screen.getByText('Guide point')).toBeTruthy()
    expect(screen.getByText('Guide')).toBeTruthy()
    expect(screen.queryByLabelText('Add tag')).toBeNull()
    // Same as annotations: an untagged guide shows no Tags row at all.
    expect(screen.queryByText('Tags')).toBeNull()
  })

  it('names a selected guide line', () => {
    const scene = makeScene({ guide_kind: () => 'line', guide_tags: () => [] })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[]} selectedGuide={7n} />)
    expect(screen.getByText('Guide line')).toBeTruthy()
  })

  it('shows the tags of a tagged guide read-only', () => {
    const scene = makeScene({ guide_kind: () => 'line', guide_tags: () => ['Rig', 'Cameras/Wide'] })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[]} selectedGuide={7n} />)
    expect(screen.getByText('Tags')).toBeTruthy()
    expect(screen.getByText('Rig')).toBeTruthy()
    expect(screen.getByText('Cameras / Wide')).toBeTruthy()
    expect(screen.queryByLabelText('Add tag')).toBeNull()
  })
})

describe('ObjectInfoPanel — selection path line', () => {
  // Kitchen(g1) > Base(g2) > Door(o10); Floor(o12) is top level.
  const parents = new Map<string, bigint>([['1:2', 1n], ['0:10', 2n]])
  const names: Record<string, string> = { '1': 'Kitchen', '2': 'Base Cabinets' }
  function pathScene() {
    return makeScene({
      top_level_nodes: () => [{ kind: 'group', id: 1n }, { kind: 'object', id: 12n }],
      group_members: (id: bigint) =>
        id === 1n ? [{ kind: 'group', id: 2n }] : id === 2n ? [{ kind: 'object', id: 10n }] : [],
      node_parent: (kind: number, id: bigint) => parents.get(`${kind}:${id}`),
      group_name: (id: bigint) => names[String(id)],
      node_tags: () => [],
      object_materials: () => [],
      object_solid: () => true,
      object_name: (id: bigint) => (id === 10n ? 'Door' : id === 12n ? 'Floor' : undefined),
    })
  }

  it('shows "in A › B" under Type for a nested object, each segment selecting that node', () => {
    const onSelectMany = vi.fn()
    render(
      <ObjectInfoPanel {...baseProps} onSelectMany={onSelectMany} scene={pathScene()} selectedIds={[{ kind: 'object', id: 10n }]} />,
    )
    const line = screen.getByTestId('object-info-path')
    expect(line.textContent).toBe('in Kitchen › Base Cabinets')
    fireEvent.click(screen.getByRole('button', { name: 'Kitchen' }))
    expect(onSelectMany).toHaveBeenCalledWith([{ kind: 'group', id: 1n }])
    fireEvent.click(screen.getByRole('button', { name: 'Base Cabinets' }))
    expect(onSelectMany).toHaveBeenLastCalledWith([{ kind: 'group', id: 2n }])
  })

  it('shows the line for a nested group too', () => {
    render(<ObjectInfoPanel {...baseProps} scene={pathScene()} selectedIds={[{ kind: 'group', id: 2n }]} />)
    expect(screen.getByTestId('object-info-path').textContent).toBe('in Kitchen')
  })

  it('shows no line for a top-level object', () => {
    render(<ObjectInfoPanel {...baseProps} scene={pathScene()} selectedIds={[{ kind: 'object', id: 12n }]} />)
    expect(screen.queryByTestId('object-info-path')).toBeNull()
  })

  it('names the open frames above a selection whose parent links the session erased, without making them buttons', () => {
    const erased = pathScene()
    ;(erased as unknown as { node_parent: () => undefined }).node_parent = () => undefined
    render(
      <ObjectInfoPanel
        {...baseProps}
        scene={erased}
        selectedIds={[{ kind: 'object', id: 10n }]}
        sessionStack={[{ node: { kind: 'group', id: 1n }, label: 'Kitchen' }]}
        sessionMembers={[{ kind: 'object', id: 10n }]}
      />,
    )
    expect(screen.getByTestId('object-info-path').textContent).toBe('in Kitchen')
    expect(screen.queryByRole('button', { name: 'Kitchen' })).toBeNull()
  })
})

describe('ObjectInfoPanel — whole sketch labels', () => {
  const sketchScene = (listed: bigint[]) =>
    makeScene({
      sketch_ids: () => listed,
      all_sketch_ids: () => listed,
      sketch_island_ids: (sid: bigint) => [sid * 10n],
    })
  it('numbers a listed sketch by its position', () => {
    render(<ObjectInfoPanel {...baseProps} scene={sketchScene([4n, 7n])} selectedIds={[{ kind: 'sketch', id: 7n }]} />)
    expect(screen.getByText('Sketch 2')).toBeInTheDocument()
  })
  it('an unplaced sketch reads "Sketch", never "Sketch 1"', () => {
    render(<ObjectInfoPanel {...baseProps} scene={sketchScene([4n])} selectedIds={[{ kind: 'sketch', id: 99n }]} />)
    // The Type row and the Name row both read the bare word.
    expect(screen.getAllByText('Sketch')).toHaveLength(2)
    expect(screen.queryByText('Sketch 1')).not.toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// Tag chips select the tag; the Materials row lists the union of paint.
// ---------------------------------------------------------------------------

describe('ObjectInfoPanel — tag chips', () => {
  function tagScene(tagsOf: (id: bigint) => string[]) {
    return makeScene({
      top_level_nodes: () => [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }],
      node_tags: (_k: number, id: bigint) => tagsOf(id),
      node_parent: () => undefined,
      object_solid: () => true,
      object_name: (id: bigint) => `Obj${id}`,
      object_materials: () => [],
    })
  }

  it('clicking a chip name selects that tag; the × removes without selecting', () => {
    const onSelectTag = vi.fn()
    const remove = vi.fn()
    const scene = tagScene(() => ['Structure/Roof'])
    render(
      <ObjectInfoPanel
        {...baseProps}
        onTagsChanged={baseProps.onTagsChanged}
        onSelectTag={onSelectTag}
        scene={{ ...scene, remove_node_tag: remove } as unknown as WasmScene}
        selectedIds={[{ kind: 'object', id: 1n }]}
      />,
    )
    fireEvent.click(screen.getByText('Structure / Roof'))
    expect(onSelectTag).toHaveBeenCalledWith(['Structure', 'Roof'])
    fireEvent.click(screen.getByLabelText('Remove tag Structure / Roof'))
    expect(remove).toHaveBeenCalledTimes(1)
    expect(onSelectTag).toHaveBeenCalledTimes(1)
  })

  it('chips in a multi-selection select the tag too', () => {
    const onSelectTag = vi.fn()
    render(
      <ObjectInfoPanel
        {...baseProps}
        onSelectTag={onSelectTag}
        scene={tagScene(() => ['Walls'])}
        selectedIds={[{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }]}
      />,
    )
    fireEvent.click(screen.getByText('Walls'))
    expect(onSelectTag).toHaveBeenCalledWith(['Walls'])
  })
})

describe('ObjectInfoPanel — Materials row', () => {
  const MAT: Record<string, { name: string; rgb: [number, number, number] }> = {
    '10': { name: 'Red', rgb: [255, 0, 0] },
    '11': { name: 'Blue', rgb: [0, 0, 255] },
    '12': { name: 'Oak', rgb: [200, 150, 100] },
  }
  // o1: base Red + a Blue face. o2: Oak. o3: Oak + Red. o4: nothing.
  // g1 = {o2, g2{o3}}; component def 7 = {o1, o4}, instance 5 places it.
  const objectMats: Record<string, bigint[]> = { '1': [10n, 11n], '2': [12n], '3': [12n, 10n], '4': [] }
  function matScene(extra: Record<string, unknown> = {}) {
    return makeScene({
      top_level_nodes: () => [{ kind: 'object', id: 1n }, { kind: 'group', id: 1n }, { kind: 'instance', id: 5n }, { kind: 'object', id: 4n }],
      group_members: (id: bigint) =>
        id === 1n ? [{ kind: 'object', id: 2n }, { kind: 'group', id: 2n }] : id === 2n ? [{ kind: 'object', id: 3n }] : [],
      node_tags: () => [],
      node_parent: () => undefined,
      object_solid: () => true,
      object_name: (id: bigint) => `Obj${id}`,
      group_name: () => 'Grp',
      instance_name: () => undefined,
      instance_def: () => 7n,
      is_linked: () => false,
      component_name: () => 'Def',
      instances_of: () => [5n],
      instance_expanded_members: () => [1n, 4n],
      instance_expanded_inherited: () => [0xffff_ffff_ffff_ffffn, 0xffff_ffff_ffff_ffffn],
      instance_pose: () => undefined,
      object_materials: (id: bigint) => objectMats[String(id)] ?? [],
      material_info: (id: bigint) => {
        const m = MAT[String(id)]
        return m === undefined ? undefined : { name: () => m.name, r: () => m.rgb[0], g: () => m.rgb[1], b: () => m.rgb[2] }
      },
      ...extra,
    })
  }
  const names = () => screen.queryAllByTestId('material-entry').map((e) => e.textContent)

  it('lists an object\'s base material and per-face paint, each once', () => {
    render(<ObjectInfoPanel {...baseProps} scene={matScene()} selectedIds={[{ kind: 'object', id: 1n }]} />)
    expect(names()).toEqual(['Red', 'Blue'])
  })

  it('unions a group\'s painted objects recursively', () => {
    render(<ObjectInfoPanel {...baseProps} scene={matScene()} selectedIds={[{ kind: 'group', id: 1n }]} />)
    expect(names()).toEqual(['Oak', 'Red'])
  })

  it('unions a component instance\'s definition contents', () => {
    render(<ObjectInfoPanel {...baseProps} scene={matScene()} selectedIds={[{ kind: 'instance', id: 5n }]} />)
    expect(names()).toEqual(['Red', 'Blue'])
  })

  it('unions a multi-selection', () => {
    render(
      <ObjectInfoPanel {...baseProps} scene={matScene()} selectedIds={[{ kind: 'object', id: 1n }, { kind: 'group', id: 1n }]} />,
    )
    expect(names()).toEqual(['Red', 'Blue', 'Oak'])
  })

  it('shows "None" when nothing is painted', () => {
    render(<ObjectInfoPanel {...baseProps} scene={matScene()} selectedIds={[{ kind: 'object', id: 4n }]} />)
    expect(names()).toEqual([])
    expect(within(screen.getByTestId('object-info-materials')).getByText('None')).toBeTruthy()
  })

  it('clicking an entry selects that material; there is no remove control', () => {
    const onSelectMaterial = vi.fn()
    render(
      <ObjectInfoPanel {...baseProps} onSelectMaterial={onSelectMaterial} scene={matScene()} selectedIds={[{ kind: 'object', id: 1n }]} />,
    )
    fireEvent.click(screen.getByText('Blue'))
    expect(onSelectMaterial).toHaveBeenCalledWith(11n)
    expect(within(screen.getByTestId('object-info-materials')).queryByRole('button', { name: /remove|×/i })).toBeNull()
  })

  it('folds past 12 swatches into "+N more", which expands', () => {
    const many = Array.from({ length: 15 }, (_v, i) => BigInt(100 + i))
    const scene = matScene({
      object_materials: () => many,
      material_info: (id: bigint) => ({ name: () => `M${id}`, r: () => 1, g: () => 2, b: () => 3 }),
    })
    render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[{ kind: 'object', id: 1n }]} />)
    expect(names()).toHaveLength(12)
    fireEvent.click(screen.getByText('+3 more'))
    expect(names()).toHaveLength(15)
    expect(screen.queryByText(/more$/)).toBeNull()
  })

  it('folds again when the selection changes (the expanded state does not carry over)', () => {
    const many = Array.from({ length: 15 }, (_v, i) => BigInt(100 + i))
    const scene = matScene({
      object_materials: () => many,
      material_info: (id: bigint) => ({ name: () => `M${id}`, r: () => 1, g: () => 2, b: () => 3 }),
    })
    const { rerender } = render(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[{ kind: 'object', id: 1n }]} />)
    fireEvent.click(screen.getByText('+3 more'))
    expect(names()).toHaveLength(15)
    rerender(<ObjectInfoPanel {...baseProps} scene={scene} selectedIds={[{ kind: 'object', id: 2n }]} />)
    expect(names()).toHaveLength(12)
    expect(screen.getByText('+3 more')).toBeTruthy()
  })
})
