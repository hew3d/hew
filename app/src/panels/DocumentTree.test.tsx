/**
 * DocumentTree — drawn-shape rows (planeUnits.ts `outlinerDrawing`): a shape
 * (one drawing unit of a plane) lists under its object, as an indented child
 * row, whenever any of its edges is face topology or an object-owned sketch;
 * a shape drawn entirely over the void lists as a top-level "Sketch N" row.
 * Clicking one selects the draft-island ref through the same `onSelect` every
 * other row uses.
 *
 * A plain JS object stands in for the wasm Scene (the panel imports the type
 * only, erased at runtime — the same convention ComponentsPanel.test.tsx /
 * scenePanels.test.tsx use), so no wasm/loader mock is required.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { DocumentTree } from './DocumentTree'
import { makeLabelFor, nodeKey, type NodeRef } from './treeModel'
import { draftRef, outlinerDrawing, type OutlinerDrawing } from '../tools/planeUnits'
import type { Scene as WasmScene } from '../wasm/loader'
import { getOutlinerSort, setOutlinerSort } from '../settings/view'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function makeScene(overrides: Record<string, any> = {}): WasmScene {
  return {
    top_level_nodes: () => [],
    group_members: () => [],
    object_ids: () => [],
    sketch_ids: () => [],
    sketch_island_ids: () => [],
    sketch_plane: () => undefined,
    node_parent: () => undefined,
    object_name: () => undefined,
    group_name: () => undefined,
    instance_name: () => undefined,
    instance_def: () => undefined,
    is_linked: () => false,
    component_name: () => undefined,
    object_drawing_planes: () => new Float64Array(0),
    ...overrides,
  } as unknown as WasmScene
}

const baseProps = {
  docRev: 0,
  docGeneration: 0,
  watertightMap: new Map<bigint, boolean>(),
  selectedIds: [],
  activeContext: [],
  sessionStack: [],
  sessionMembers: null,
  onEnterContext: vi.fn(),
  onExitContext: vi.fn(),
  onSetContextDepth: vi.fn(),
  hiddenKeys: new Set<string>(),
  onToggleHidden: vi.fn(),
  onSetHiddenMany: vi.fn(),
  onReparent: vi.fn(),
}

// ---------------------------------------------------------------------------
// A plane-listing double: `drawn` maps a plane's canonical context to the
// units on it; each unit is a closed square whose edges carry the given stores.
// ---------------------------------------------------------------------------

const hex = (n: number): string => n.toString(16).padStart(16, '0')
const ctxAt = (z: number): string => `{"plane":[0,0,${z},0,0,1]}`
type Store = { kind: 'topology'; object: number } | { kind: 'owned'; object: number; sketch: number } | { kind: 'free'; sketch: number }
/** `rect` (default): a 4-edge unit; `circle`/`polygon`: n edges around a centre,
 *  all in ONE curve run; `line`: one segment. The stores cycle over the edges. */
interface Shape { key: number; stores: Store[]; kind?: 'rect' | 'circle' | 'polygon' | 'line'; ox?: number }

function shapeJson(shapes: Shape[], z: number): string {
  return JSON.stringify({
    units: shapes.map((sh) => {
      const kind = sh.kind ?? 'rect'
      const ox = sh.ox ?? 0
      const n = kind === 'line' ? 1 : kind === 'rect' ? 4 : kind === 'polygon' ? 6 : 8
      const pts: number[][] = []
      if (kind === 'rect') pts.push([ox, 0, z], [ox + 1, 0, z], [ox + 1, 1, z], [ox, 1, z])
      else if (kind === 'line') pts.push([ox, 0, z], [ox + 1, 0, z])
      else for (let i = 0; i < n; i++) pts.push([ox + Math.cos((2 * Math.PI * i) / n), Math.sin((2 * Math.PI * i) / n), z])
      const curve = kind === 'circle' || kind === 'polygon' ? { center: [ox, 0, z], radius: 1 } : null
      const edges = Array.from({ length: n }, (_, i) => ({
        key: hex(sh.key * 100 + i),
        a: pts[i],
        b: pts[(i + 1) % pts.length],
        store: sh.stores[i % sh.stores.length],
        curve,
      }))
      return {
        key: hex(sh.key),
        edges,
        curves: curve === null ? [] : [{ key: hex(sh.key * 1000), curve_key: hex(sh.key * 1000), kind, center: curve.center, radius: 1, edges: edges.map((e) => e.key) }],
      }
    }),
    regions: [],
  })
}

/** Scene extras: planes `drawn` (z -> shapes), the face handle that lies on a
 *  plane (`object_drawing_planes`), and the sketches' planes. */
function drawingScene(opts: {
  drawn: Record<number, Shape[]>
  /** Object 1's faces: face handle -> plane z (reported via object_drawing_planes). */
  faces?: Record<number, number>
  /** Sketch handle -> plane z. */
  sketches?: Record<number, number>
  extra?: Record<string, unknown>
}) {
  const faces = opts.faces ?? {}
  const sketches = opts.sketches ?? {}
  return makeScene({
    top_level_nodes: () => [{ kind: 'object', id: 1n }],
    object_ids: () => [1n],
    object_name: (id: bigint) => (id === 1n ? 'Panel' : undefined),
    object_drawing_planes: (o: bigint) =>
      o === 1n
        ? new Float64Array(Object.keys(faces).flatMap((f) => [0, 0, faces[Number(f)] ?? 0, 0, 0, 1]))
        : new Float64Array(0),
    face_normal: () => new Float64Array([0, 0, 1]),
    sketch_ids: () => Object.keys(sketches).map((k) => BigInt(k)),
    sketch_plane: (sid: bigint) => (sketches[Number(sid)] === undefined ? undefined : new Float64Array([0, 0, sketches[Number(sid)], 0, 0, 1])),
    plane_units_json: (ctx: string) => {
      for (const [z, shapes] of Object.entries(opts.drawn)) {
        if (ctx === ctxAt(Number(z))) return shapeJson(shapes, Number(z))
      }
      return JSON.stringify({ units: [], regions: [] })
    },
    ...opts.extra,
  })
}

const topo1: Store = { kind: 'topology', object: 1 }
const FACE_Z = 0.5

describe('DocumentTree — drawn shape child rows', () => {
  it("lists an object's single drawn shape as a child row under it (a circle is labelled \"Circle\")", () => {
    const scene = drawingScene({ drawn: { [FACE_Z]: [{ key: 10, stores: [topo1], kind: 'circle' }] }, faces: { 10: FACE_Z } })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Panel')).toBeInTheDocument()
    expect(screen.getByText('Circle')).toBeInTheDocument()
  })

  it('clicking the shape child row selects the draft-island ref via onSelect', () => {
    const scene = drawingScene({ drawn: { [FACE_Z]: [{ key: 10, stores: [topo1], kind: 'circle' }] }, faces: { 10: FACE_Z } })
    const onSelect = vi.fn()
    render(<DocumentTree {...baseProps} scene={scene} onSelect={onSelect} />)
    fireEvent.click(screen.getByText('Circle'))
    expect(onSelect).toHaveBeenCalledWith(
      { kind: 'draft-island', id: 10n, ctx: ctxAt(FACE_Z), key: hex(10) },
      false,
    )
  })

  it('an object with no drawing shows no shape child row', () => {
    const scene = drawingScene({ drawn: {}, faces: {} })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Panel')).toBeInTheDocument()
    expect(screen.queryByText('Circle')).not.toBeInTheDocument()
  })

  it('numbers two shapes on one object ("Circle 1" / "Circle 2")', () => {
    const scene = drawingScene({
      drawn: { [FACE_Z]: [{ key: 10, stores: [topo1], kind: 'circle' }, { key: 11, stores: [topo1], kind: 'circle', ox: 3 }] },
      faces: { 10: FACE_Z },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Circle 1')).toBeInTheDocument()
    expect(screen.getByText('Circle 2')).toBeInTheDocument()
  })
})

describe('DocumentTree — shape names come from the unit\'s edges', () => {
  it('names a rectangle-shaped unit, a circle, a polygon, and an owned line', () => {
    const owned1: Store = { kind: 'owned', object: 1, sketch: 7 }
    const scene = drawingScene({
      drawn: {
        [FACE_Z]: [
          { key: 10, stores: [topo1], kind: 'rect' },
          { key: 11, stores: [topo1], kind: 'circle', ox: 3 },
          { key: 12, stores: [topo1], kind: 'polygon', ox: 6 },
        ],
        2: [{ key: 20, stores: [owned1], kind: 'line' }],
      },
      faces: { 10: FACE_Z },
      sketches: { 7: 2 },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
    expect(screen.getByText('Circle')).toBeInTheDocument()
    expect(screen.getByText('Polygon')).toBeInTheDocument()
    expect(screen.getByText('Line')).toBeInTheDocument()
    expect(screen.queryByText('Shape')).not.toBeInTheDocument()
  })

  it('numbers repeated names 1-based ("Circle 1" / "Circle 2") but leaves a lone name bare', () => {
    const scene = drawingScene({
      drawn: {
        [FACE_Z]: [
          { key: 10, stores: [topo1], kind: 'circle' },
          { key: 11, stores: [topo1], kind: 'circle', ox: 3 },
          { key: 12, stores: [topo1], kind: 'rect', ox: 6 },
        ],
      },
      faces: { 10: FACE_Z },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Circle 1')).toBeInTheDocument()
    expect(screen.getByText('Circle 2')).toBeInTheDocument()
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
  })
})

describe('DocumentTree — outliner placement by store', () => {
  const owned1: Store = { kind: 'owned', object: 1, sketch: 7 }
  const free5: Store = { kind: 'free', sketch: 5 }

  it('an object-OWNED sketch\'s shape lists under its object (found from the sketch\'s plane, no face feature needed)', () => {
    const scene = drawingScene({
      drawn: { 2: [{ key: 20, stores: [owned1, owned1, owned1, owned1] }] },
      sketches: { 7: 2 },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
    // Not a free-sketch row.
    expect(screen.queryByText('Sketch 1')).not.toBeInTheDocument()
  })

  it('a shape entirely in a FREE sketch lists as a top-level Sketch row, not under an object', () => {
    const scene = drawingScene({
      drawn: { 0: [{ key: 30, stores: [free5, free5, free5, free5] }] },
      sketches: { 5: 0 },
    })
    const onSelect = vi.fn()
    render(<DocumentTree {...baseProps} scene={scene} onSelect={onSelect} />)
    expect(screen.getByText('Sketch 1')).toBeInTheDocument()
    expect(screen.queryByText('Rectangle')).not.toBeInTheDocument()
    fireEvent.click(screen.getByText('Sketch 1'))
    expect(onSelect).toHaveBeenCalledWith(
      { kind: 'draft-island', id: 30n, ctx: ctxAt(0), key: hex(30) },
      false,
    )
  })

  it('a MIXED shape (some topology edges, some free) lists under its object only', () => {
    const scene = drawingScene({
      drawn: { 0: [{ key: 40, stores: [topo1, topo1, free5, free5] }] },
      faces: { 40: 0 },
      sketches: { 5: 0 },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
    expect(screen.queryByText('Sketch 1')).not.toBeInTheDocument()
  })

  it('two free shapes on the ground get two numbered Sketch rows', () => {
    const scene = drawingScene({
      drawn: { 0: [{ key: 30, stores: [free5, free5, free5, free5] }, { key: 31, stores: [free5, free5, free5, free5] }] },
      sketches: { 5: 0 },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(screen.getByText('Sketch 1')).toBeInTheDocument()
    expect(screen.getByText('Sketch 2')).toBeInTheDocument()
  })

  it('a selected draft-edge or draft-curve lights its shape\'s row', () => {
    const scene = drawingScene({ drawn: { [FACE_Z]: [{ key: 10, stores: [topo1, topo1, topo1, topo1] }] }, faces: { 10: FACE_Z } })
    render(
      <DocumentTree
        {...baseProps}
        scene={scene}
        onSelect={vi.fn()}
        selectedIds={[{ kind: 'draft-edge', id: 1001n, ctx: ctxAt(FACE_Z), key: hex(1001) }]}
      />,
    )
    // Lit as a selected (non-primary) row: the selection is the edge, not the shape.
    const row = screen.getByText('Rectangle').closest('div')
    expect(row).toHaveStyle({ background: 'var(--accent-tint-15)' })
  })
})

describe('makeLabelFor — parity with the Outliner rows', () => {
  const owned1: Store = { kind: 'owned', object: 1, sketch: 7 }

  it('numbers a free sketch by its Outliner row, not by its sketch_ids() position', () => {
    const scene = drawingScene({
      drawn: {
        1: [{ key: 30, stores: [{ kind: 'free', sketch: 5 }], kind: 'rect' }],
        2: [{ key: 31, stores: [{ kind: 'free', sketch: 6 }], kind: 'rect' }],
      },
      sketches: { 5: 1, 6: 2 },
      // sketch_ids() lists them in the opposite order to the Outliner's rows.
      extra: { sketch_ids: () => [6n, 5n] },
    })
    const drawing = outlinerDrawing(scene as unknown as Parameters<typeof outlinerDrawing>[0], [1n])
    expect(drawing.free).toHaveLength(2)
    const labelFor = makeLabelFor(scene, new Map(), new Map(), () => drawing)
    drawing.free.forEach((u, i) => {
      const e = u.unit.edges[0].store
      if (e.kind !== 'free') throw new Error('expected a free store')
      expect(labelFor({ kind: 'sketch', id: e.sketch })).toBe(`Sketch ${i + 1}`)
    })
    // Without a drawing it falls back to the sketch_ids() position.
    expect(makeLabelFor(scene, new Map())({ kind: 'sketch', id: 6n })).toBe('Sketch 1')
  })

  it('prints a drawn selection with the Outliner\'s numbered label, for the shape and its lines', () => {
    const scene = drawingScene({
      drawn: { [FACE_Z]: [{ key: 10, stores: [topo1], kind: 'circle' }, { key: 11, stores: [topo1], kind: 'circle', ox: 3 }] },
      faces: { 10: FACE_Z },
    })
    const drawing: OutlinerDrawing = outlinerDrawing(scene as unknown as Parameters<typeof outlinerDrawing>[0], [1n])
    const labelFor = makeLabelFor(scene, new Map(), new Map(), () => drawing)
    const island2 = draftRef('draft-island', ctxAt(FACE_Z), hex(11))
    expect(drawing.labels.get(nodeKey(island2))).toBe('Circle 2')
    expect(labelFor(island2)).toBe('Circle 2')
    // A line of that shape resolves to the shape's row label.
    expect(labelFor(draftRef('draft-edge', ctxAt(FACE_Z), hex(11 * 100)))).toBe('Circle 2')
    // No drawing to consult: the bare kind word.
    expect(makeLabelFor(scene, new Map())(island2)).toBe('Shape')
    void owned1
  })
})

describe('DocumentTree — filter keeps positional labels', () => {
  it('an unnamed child keeps its unfiltered "Group N" while siblings are filtered out', () => {
    const g = (id: number): NodeRef => ({ kind: 'group', id: BigInt(id) })
    const scene = makeScene({
      top_level_nodes: () => [g(1)],
      group_ids: () => [1n, 2n, 3n, 4n],
      group_members: (id: bigint) => (id === 1n ? [g(2), g(3), g(4)] : []),
      group_name: (id: bigint) => (id === 1n ? 'Top' : undefined),
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    fireEvent.change(screen.getByLabelText('Filter outliner'), { target: { value: 'Group 3' } })
    expect(screen.getByText('Group 3')).toBeInTheDocument()
    expect(screen.queryByText('Group 1')).not.toBeInTheDocument()
  })
})

const SORT_NAMES: Record<string, string> = { document: 'Document order', name: 'Name', kind: 'Kind' }
const pickSort = (mode: string): void => {
  fireEvent.click(screen.getByRole('radio', { name: SORT_NAMES[mode] }))
}

describe('DocumentTree — sort control', () => {
  const o = (id: number): NodeRef => ({ kind: 'object', id: BigInt(id) })
  const g = (id: number): NodeRef => ({ kind: 'group', id: BigInt(id) })
  const i = (id: number): NodeRef => ({ kind: 'instance', id: BigInt(id) })
  const names = (): string[] =>
    Array.from(document.querySelectorAll('[data-testid="outliner-row-label"]')).map((e) => e.textContent ?? '')

  // Top level (document order): Zed (object), Group 2 (unnamed), beta (group),
  // Door (component), Apple (object), Group 10 is nested inside beta with
  // Gamma (object) and an unnamed group "Group 1" (position 0 inside beta).
  const scene = makeScene({
    top_level_nodes: () => [o(1), g(2), g(3), i(4), o(5)],
    group_members: (id: bigint) => (id === 3n ? [g(6), o(7)] : []),
    object_name: (id: bigint) => ({ 1: 'Zed', 5: 'apple', 7: 'Gamma' } as Record<number, string>)[Number(id)],
    group_name: (id: bigint) => (id === 3n ? 'beta' : undefined),
    instance_name: () => undefined,
    instance_def: (id: bigint) => (id === 4n ? 40n : undefined),
    is_linked: () => false,
    component_name: (id: bigint) => (id === 40n ? 'Door' : undefined),
  })
  const renderTree = () => render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)

  beforeEach(() => {
    localStorage.clear()
    setOutlinerSort('document')
  })

  it('defaults to Document order', () => {
    expect(getOutlinerSort()).toBe('document')
    renderTree()
    expect(screen.getByRole('radio', { name: 'Document order' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('radio', { name: 'Name' })).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByRole('radio', { name: 'Kind' })).toHaveAttribute('aria-checked', 'false')
    expect(names()).toEqual(['Zed', 'Group 2', 'beta', 'Door', 'apple'])
  })

  it('is a three-button radio group with a roving tab stop, and the arrow keys move and select', () => {
    renderTree()
    const group = screen.getByRole('radiogroup', { name: 'Sort outliner' })
    expect(group.querySelectorAll('[role="radio"]')).toHaveLength(3)
    const doc = screen.getByRole('radio', { name: 'Document order' })
    const name = screen.getByRole('radio', { name: 'Name' })
    const kind = screen.getByRole('radio', { name: 'Kind' })
    expect([doc.tabIndex, name.tabIndex, kind.tabIndex]).toEqual([0, -1, -1])
    expect(name).toHaveAttribute('title', 'Name')
    doc.focus()
    fireEvent.keyDown(doc, { key: 'ArrowRight' })
    expect(getOutlinerSort()).toBe('name')
    expect(name).toHaveAttribute('aria-checked', 'true')
    expect(document.activeElement).toBe(name)
    fireEvent.keyDown(name, { key: 'ArrowRight' })
    expect(getOutlinerSort()).toBe('kind')
    fireEvent.keyDown(kind, { key: 'ArrowRight' })
    expect(getOutlinerSort()).toBe('document')
    fireEvent.keyDown(doc, { key: 'ArrowLeft' })
    expect(getOutlinerSort()).toBe('kind')
  })

  it('Name sorts case-insensitively; the generated "Group 2" keeps its number', () => {
    renderTree()
    pickSort('name')
    expect(names()).toEqual(['apple', 'beta', 'Door', 'Group 2', 'Zed'])
  })

  it('Kind groups objects, groups, components, then orders by name within a kind', () => {
    renderTree()
    pickSort('kind')
    expect(names()).toEqual(['apple', 'Zed', 'beta', 'Group 2', 'Door'])
  })

  it('applies per level: a group\'s members sort inside it, keeping their positional labels', () => {
    renderTree()
    pickSort('name')
    // Open beta (a collapsed group): its members are Group 1 (unnamed, position 0) and Gamma.
    fireEvent.click(screen.getAllByText('▸')[0])
    const all = names()
    const at = all.indexOf('beta')
    expect(all.slice(at + 1, at + 3)).toEqual(['Gamma', 'Group 1'])
  })

  it('persists per person under hew.settings.outlinerSort', () => {
    renderTree()
    pickSort('kind')
    expect(localStorage.getItem('hew.settings.outlinerSort')).toBe('kind')
    expect(getOutlinerSort()).toBe('kind')
  })

  it('the filter still narrows, and the breadcrumb still shows the open group, under Name sort', () => {
    setOutlinerSort('name')
    render(
      <DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} activeContext={[g(3)]} />,
    )
    expect(screen.getAllByText('beta').length).toBeGreaterThan(0)
    fireEvent.change(screen.getByLabelText('Filter outliner'), { target: { value: 'apple' } })
    expect(screen.getByText('apple')).toBeInTheDocument()
    expect(screen.queryByText('Zed')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Model' })).toBeInTheDocument()
  })
})

describe('DocumentTree — sort covers free sketches, session members, and imprints', () => {
  const o = (id: number): NodeRef => ({ kind: 'object', id: BigInt(id) })
  const g = (id: number): NodeRef => ({ kind: 'group', id: BigInt(id) })
  const i = (id: number): NodeRef => ({ kind: 'instance', id: BigInt(id) })
  const free5: Store = { kind: 'free', sketch: 5 }
  const names = (): string[] =>
    Array.from(document.querySelectorAll('[data-testid="outliner-row-label"]')).map((e) => e.textContent ?? '')
  const sortTo = (mode: string): void => {
    pickSort(mode)
  }

  beforeEach(() => {
    localStorage.clear()
    setOutlinerSort('document')
  })

  it('free-sketch rows sort with the top level: Name puts "Sketch 1" before "Zebra", Kind puts sketches after components', () => {
    const scene = drawingScene({
      drawn: { 0: [{ key: 30, stores: [free5, free5, free5, free5] }] },
      sketches: { 5: 0 },
      extra: {
        top_level_nodes: () => [o(1), i(4)],
        object_name: (id: bigint) => (id === 1n ? 'Zebra' : undefined),
        instance_def: (id: bigint) => (id === 4n ? 40n : undefined),
        is_linked: () => false,
        component_name: (id: bigint) => (id === 40n ? 'Door' : undefined),
      },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(names()).toEqual(['Zebra', 'Door', 'Sketch 1'])
    sortTo('name')
    expect(names()).toEqual(['Door', 'Sketch 1', 'Zebra'])
    sortTo('kind')
    expect(names()).toEqual(['Zebra', 'Door', 'Sketch 1'])
  })

  it('keeps each sketch\'s number when sorting and still selects the right one', () => {
    const scene = drawingScene({
      drawn: { 0: [{ key: 30, stores: [free5, free5, free5, free5] }, { key: 31, stores: [free5, free5, free5, free5], ox: 3 }] },
      sketches: { 5: 0 },
      extra: { top_level_nodes: () => [o(1)], object_name: () => 'Aardvark' },
    })
    const onSelect = vi.fn()
    render(<DocumentTree {...baseProps} scene={scene} onSelect={onSelect} />)
    sortTo('name')
    expect(names()).toEqual(['Aardvark', 'Sketch 1', 'Sketch 2'])
    fireEvent.click(screen.getByText('Sketch 2'))
    expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: 31n }), false)
  })

  it('the filter still hides non-matching sketches under a sort', () => {
    const scene = drawingScene({
      drawn: { 0: [{ key: 30, stores: [free5, free5, free5, free5] }, { key: 31, stores: [free5, free5, free5, free5], ox: 3 }] },
      sketches: { 5: 0 },
      extra: { top_level_nodes: () => [o(1)], object_name: () => 'Aardvark' },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    sortTo('name')
    fireEvent.change(screen.getByLabelText('Filter outliner'), { target: { value: 'sketch 2' } })
    expect(names()).toEqual(['Sketch 2'])
  })

  it('session-member rows sort by Name and by Kind, keeping positional labels', () => {
    const scene = makeScene({
      top_level_nodes: () => [o(1), g(2), i(4), o(5)],
      object_name: (id: bigint) => ({ 1: 'Zed', 5: 'apple' } as Record<number, string>)[Number(id)],
      instance_def: (id: bigint) => (id === 4n ? 40n : undefined),
      is_linked: () => false,
      component_name: (id: bigint) => (id === 40n ? 'Door' : undefined),
    })
    const props = {
      ...baseProps,
      scene,
      onSelect: vi.fn(),
      sessionStack: [{ node: g(9), label: 'Open' }],
      sessionMembers: [o(1), g(2), i(4), o(5)],
    }
    render(<DocumentTree {...props} />)
    expect(names()).toEqual(['Open', 'Zed', 'Group 2', 'Door', 'apple'])
    sortTo('name')
    expect(names()).toEqual(['Open', 'apple', 'Door', 'Group 2', 'Zed'])
    sortTo('kind')
    expect(names()).toEqual(['Open', 'apple', 'Zed', 'Group 2', 'Door'])
  })

  it('imprint rows under an object sort by name under Name and Kind, and keep drawing order in Document', () => {
    const scene = drawingScene({
      drawn: {
        [FACE_Z]: [
          { key: 10, stores: [topo1], kind: 'rect' },
          { key: 11, stores: [topo1], kind: 'circle', ox: 3 },
        ],
      },
      faces: { 10: FACE_Z },
    })
    render(<DocumentTree {...baseProps} scene={scene} onSelect={vi.fn()} />)
    expect(names()).toEqual(['Panel', 'Rectangle', 'Circle'])
    sortTo('name')
    expect(names()).toEqual(['Panel', 'Circle', 'Rectangle'])
    sortTo('kind')
    expect(names()).toEqual(['Panel', 'Circle', 'Rectangle'])
  })
})

// ---------------------------------------------------------------------------
// Linked copies: placements of a linked set read as copies, not components.
// ---------------------------------------------------------------------------

describe('DocumentTree — linked copies', () => {
  /** Two placements (ids 4 and 5) of the linked set `def`; with `solo`, only one. */
  function linkedScene(opts: { solo?: boolean; instanceNames?: Record<string, string> } = {}) {
    const ids = opts.solo === true ? [4n] : [4n, 5n]
    return makeScene({
      top_level_nodes: () => ids.map((id) => ({ kind: 'instance', id })),
      instance_def: () => 40n,
      is_linked: () => true,
      definition_usage: () => ids.length,
      instances_of: () => ids,
      component_name: () => 'Leg',
      instance_name: (id: bigint) => opts.instanceNames?.[String(id)],
      component_member_objects: () => [9n],
      object_name: () => 'Leg',
    })
  }
  const actions = () => ({
    onUnlink: vi.fn(),
    onUnlinkAll: vi.fn(),
    onMakeComponent: vi.fn(),
    onSelectCopies: vi.fn(),
    onRename: vi.fn(),
  })

  it('a_linked_placement_shows_the_link_glyph_and_the_count_badge', () => {
    const { container } = render(<DocumentTree scene={linkedScene()} {...baseProps} onSelect={vi.fn()} />)
    expect(container.querySelectorAll('[data-node-icon="linked"]').length).toBe(2)
    expect(container.querySelector('[data-node-icon="instance"]')).toBeNull()
    const badges = screen.getAllByTestId('outliner-copies-badge')
    expect(badges.map((b) => b.textContent)).toEqual(['×2', '×2'])
    // Rows read as the board, never as a component.
    expect(screen.getAllByTestId('outliner-row-label').map((l) => l.textContent)).toEqual(['Leg', 'Leg'])
  })

  it('a_set_of_one_shows_no_badge', () => {
    const { container } = render(<DocumentTree scene={linkedScene({ solo: true })} {...baseProps} onSelect={vi.fn()} />)
    expect(screen.queryByTestId('outliner-copies-badge')).toBeNull()
    // Still the glyph, and still a board by name.
    expect(container.querySelectorAll('[data-node-icon="linked"]').length).toBe(1)
    expect(screen.getByTestId('outliner-row-label').textContent).toBe('Leg')
  })

  it('hovering_the_badge_highlights_the_other_placements', () => {
    const { container } = render(<DocumentTree scene={linkedScene()} {...baseProps} onSelect={vi.fn()} />)
    const [first] = screen.getAllByTestId('outliner-copies-badge')
    fireEvent.mouseEnter(first)
    const rows = container.querySelectorAll('[data-copies-highlighted="true"]')
    expect(rows.length).toBe(2)
    fireEvent.mouseLeave(first)
    expect(container.querySelectorAll('[data-copies-highlighted="true"]').length).toBe(0)
  })

  it('an ordinary component keeps its glyph and name, with no badge or copies menu', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'instance', id: 4n }],
      instance_def: () => 40n,
      is_linked: () => false,
      definition_usage: () => 3,
      component_name: () => 'Chair',
    })
    const { container } = render(<DocumentTree scene={scene} {...baseProps} onSelect={vi.fn()} linkedActions={actions()} />)
    expect(container.querySelector('[data-node-icon="instance"]')).not.toBeNull()
    expect(screen.queryByTestId('outliner-copies-badge')).toBeNull()
    expect(screen.queryByTestId('outliner-copies-button')).toBeNull()
    expect(screen.getByTestId('outliner-row-label').textContent).toBe('Chair')
  })

  it('the Copies menu offers Unlink, Unlink all, Make component, and Select all copies', () => {
    const a = actions()
    render(<DocumentTree scene={linkedScene()} {...baseProps} onSelect={vi.fn()} linkedActions={a} />)
    fireEvent.click(screen.getAllByTestId('outliner-copies-button')[0])
    const items = screen.getAllByRole('menuitem').map((i) => i.textContent)
    expect(items).toEqual(['Unlink', 'Unlink all', 'Make component', 'Select all copies', 'Rename'])
    fireEvent.click(screen.getByRole('menuitem', { name: 'Unlink' }))
    expect(a.onUnlink).toHaveBeenCalledWith({ kind: 'instance', id: 4n })
    fireEvent.click(screen.getAllByTestId('outliner-copies-button')[0])
    fireEvent.click(screen.getByRole('menuitem', { name: 'Select all copies' }))
    expect(a.onSelectCopies).toHaveBeenCalledWith({ kind: 'instance', id: 4n })
  })

  it('a set of one offers no Unlink all or Select all copies', () => {
    render(<DocumentTree scene={linkedScene({ solo: true })} {...baseProps} onSelect={vi.fn()} linkedActions={actions()} />)
    fireEvent.click(screen.getByTestId('outliner-copies-button'))
    expect(screen.getAllByRole('menuitem').map((i) => i.textContent)).toEqual(['Unlink', 'Make component', 'Rename'])
  })

  it('renaming from the row reports the typed name for the row\'s node', () => {
    const a = actions()
    render(<DocumentTree scene={linkedScene()} {...baseProps} onSelect={vi.fn()} linkedActions={a} />)
    fireEvent.click(screen.getAllByTestId('outliner-copies-button')[1])
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename' }))
    const input = screen.getByTestId('outliner-row-rename') as HTMLInputElement
    expect(input.value).toBe('Leg')
    fireEvent.change(input, { target: { value: 'Back leg' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(a.onRename).toHaveBeenCalledWith({ kind: 'instance', id: 5n }, 'Back leg')
    expect(screen.queryByTestId('outliner-row-rename')).toBeNull()
  })

  it('rename_that_types_back_the_shown_label_is_not_a_rename', () => {
    const a = actions()
    render(<DocumentTree scene={linkedScene()} {...baseProps} onSelect={vi.fn()} linkedActions={a} />)
    fireEvent.click(screen.getAllByTestId('outliner-copies-button')[0])
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename' }))
    // Enter with the label untouched ("Leg" is a fallback here, not a stored name).
    fireEvent.keyDown(screen.getByTestId('outliner-row-rename'), { key: 'Enter' })
    expect(a.onRename).not.toHaveBeenCalled()
  })

  it('the_copies_button_closes_an_open_menu_instead_of_reopening_it', () => {
    render(<DocumentTree scene={linkedScene()} {...baseProps} onSelect={vi.fn()} linkedActions={actions()} />)
    const button = screen.getAllByTestId('outliner-copies-button')[0]
    fireEvent.click(button)
    expect(screen.getByRole('menu')).toBeInTheDocument()
    // A real press is pointerdown, then click.
    fireEvent.pointerDown(button)
    fireEvent.click(button)
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('Escape leaves the name alone', () => {
    const a = actions()
    render(<DocumentTree scene={linkedScene()} {...baseProps} onSelect={vi.fn()} linkedActions={a} />)
    fireEvent.click(screen.getAllByTestId('outliner-copies-button')[0])
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename' }))
    fireEvent.keyDown(screen.getByTestId('outliner-row-rename'), { key: 'Escape' })
    expect(a.onRename).not.toHaveBeenCalled()
  })
})
