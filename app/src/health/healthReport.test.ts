import { describe, expect, it } from 'vitest'
import {
  filterRows,
  groupRows,
  groupedRows,
  classifyRow,
  objectHealthStates,
  parseHealthReport,
  parseHealthSummary,
  rowClass,
  rowDisplayName,
  openReason,
  canCloseRow,
  segmentBounds,
  summarize,
  rowDetailText,
  EMPTY_COUNTS,
  type HealthRow,
} from './healthReport'

const RIM = { role: 'outer', planar: true, closed: true, cappable: true, sheet_edge: false, shared: false, perimeter: 4, extent: 1, edges: 4 }

function row(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    object: '10',
    name: 'Wall',
    definition: null,
    definition_name: null,
    placements: 1,
    instances: [],
    accepted: false,
    family: null,
    verdict: { kind: 'cappable' },
    rims: [RIM],
    branching_vertices: 0,
    islands: 1,
    ...over,
  }
}

function report(rows: Record<string, unknown>[], families: unknown[] = []) {
  return parseHealthReport(JSON.stringify({ rows, families, counts: { ...EMPTY_COUNTS, objects: 9, solids: 9 - rows.length } }))
}

describe('parseHealthReport', () => {
  it('turns handles into bigints and snake_case into camelCase', () => {
    const r = report(
      [row({ object: '18446744073709551615', definition: '7', definition_name: 'Chair', placements: 3, instances: ['11', '12'], family: 0 })],
      [{ members: ['18446744073709551615', '5'], shared_rims: 2 }],
    )
    expect(r.rows[0].object).toBe(18446744073709551615n)
    expect(r.rows[0].definition).toBe(7n)
    expect(r.rows[0].definitionName).toBe('Chair')
    expect(r.rows[0].instances).toEqual([11n, 12n])
    expect(r.families[0]).toEqual({ members: [18446744073709551615n, 5n], sharedRims: 2 })
    expect(r.counts.objects).toBe(9)
  })

  it('tolerates missing optional fields', () => {
    const r = parseHealthReport(JSON.stringify({ rows: [{ object: '1', verdict: { kind: 'mixed' } }] }))
    expect(r.rows[0]).toMatchObject({ name: null, definition: null, placements: 1, instances: [], rims: [], islands: 1 })
    expect(r.counts).toEqual(EMPTY_COUNTS)
  })
})

describe('words', () => {
  const one = (over: Record<string, unknown>): HealthRow => report([row(over)]).rows[0]

  it('details a row by its kind', () => {
    expect(rowDetailText(one({}))).toBe('1 opening')
    expect(rowDetailText(one({ rims: [RIM, RIM, RIM] }))).toBe('3 openings')
    expect(rowDetailText(one({ verdict: { kind: 'flat_sheet', holes: 0 }, rims: [] }))).toBe('Flat panel')
    expect(rowDetailText(one({ verdict: { kind: 'flat_sheet', holes: 0 }, rims: [], accepted: true }))).toBe('Flat panel, kept')
    expect(rowDetailText(one({ verdict: { kind: 'bent_sheet' }, rims: [] }))).toBe('Curved sheet')
    expect(rowDetailText(one({ verdict: { kind: 'non_manifold' }, branching_vertices: 2 }))).toBe('Openings tangled together')
    expect(rowDetailText(one({ family: 0 }))).toBe('Piece of a split shape')
  })

  it('never shows the old kernel words', () => {
    for (const kind of ['cappable', 'holes', 'flat_sheet', 'bent_sheet', 'mixed', 'non_manifold']) {
      const r = one({ verdict: { kind, holes: 1, planar: 1, warped: 0 } })
      expect(rowDetailText(r)).not.toMatch(/leaky|rim|planar|manifold|seam|verdict|cappable|surface/i)
    }
  })

  it('details a row that can not be closed by its short reason', () => {
    const lip = { ...RIM, cappable: false, sheet_edge: true }
    expect(rowDetailText(one({ verdict: { kind: 'holes', planar: 1, warped: 0 }, rims: [lip] }))).toBe('Open edges run along a lip')
    const warped = { ...RIM, planar: false, cappable: false }
    expect(rowDetailText(one({ verdict: { kind: 'holes', planar: 0, warped: 1 }, rims: [warped] }))).toBe("Open edges don't lie flat")
  })

  it('names a definition member by its definition', () => {
    expect(rowDisplayName(one({ definition: '3', definition_name: 'Chair' }))).toBe('Chair')
    expect(rowDisplayName(one({ name: null }))).toBe('Object')
  })
})

describe('rims and open reasons', () => {
  const one = (over: Record<string, unknown>): HealthRow => report([row(over)]).rows[0]

  it('parses sheet_edge and shared, defaulting both to false', () => {
    const r = one({ rims: [{ ...RIM, sheet_edge: true, shared: true }, { role: 'hole', planar: true, closed: true, cappable: true }] })
    expect(r.rims[0]).toMatchObject({ sheetEdge: true, shared: true })
    expect(r.rims[1]).toMatchObject({ sheetEdge: false, shared: false })
  })

  it('branching vertices or an open chain: the open edges branch', () => {
    expect(openReason(one({ branching_vertices: 1 }))).toBe('branches')
    expect(openReason(one({ rims: [{ ...RIM, closed: false, cappable: false }] }))).toBe('branches')
    // The branching reading wins over a crooked opening.
    expect(openReason(one({ branching_vertices: 3, rims: [{ ...RIM, planar: false, cappable: false }] }))).toBe('branches')
  })

  it('a closed opening off any plane does not lie flat', () => {
    expect(openReason(one({ rims: [RIM, { ...RIM, planar: false, cappable: false }] }))).toBe('notFlat')
  })

  it('a flat opening along the edge of a flat part is a lip', () => {
    expect(openReason(one({ rims: [{ ...RIM, cappable: false, sheet_edge: true }] }))).toBe('lip')
  })

  it('anything else reads as not flat', () => {
    expect(openReason(one({ rims: [] }))).toBe('notFlat')
  })
})

describe('classifyRow', () => {
  const kind = (over: Record<string, unknown>) => classifyRow(report([row(over)]).rows[0])
  const warped = { ...RIM, cappable: false }

  it('sorts every verdict and rim combination into the five kinds', () => {
    expect(kind({ verdict: { kind: 'cappable' } })).toBe('openings')
    expect(kind({ verdict: { kind: 'holes', planar: 2, warped: 0 }, rims: [RIM, RIM] })).toBe('openings')
    expect(kind({ verdict: { kind: 'mixed' }, rims: [RIM, RIM] })).toBe('openings')
    expect(kind({ verdict: { kind: 'holes', planar: 1, warped: 1 }, rims: [RIM, warped] })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'mixed' }, rims: [RIM, warped] })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'cappable' }, rims: [warped] })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'mixed' }, rims: [] })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'non_manifold' }, rims: [RIM] })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'flat_sheet', holes: 1 }, rims: [warped] })).toBe('flatPanels')
    expect(kind({ verdict: { kind: 'bent_sheet' }, rims: [warped] })).toBe('curvedSheets')
  })

  it('any row with a family is a split shape, whatever its verdict', () => {
    for (const k of ['cappable', 'flat_sheet', 'bent_sheet', 'non_manifold', 'mixed']) {
      expect(kind({ family: 0, verdict: { kind: k } })).toBe('split')
    }
  })
})

describe('classification and summary', () => {
  it('sheets and accepted rows are surfaces; the rest need attention', () => {
    const r = report([
      row({ object: '1', verdict: { kind: 'flat_sheet', holes: 0 } }),
      row({ object: '2', verdict: { kind: 'bent_sheet' } }),
      row({ object: '3', verdict: { kind: 'cappable' } }),
      row({ object: '4', verdict: { kind: 'mixed' }, accepted: true }),
      row({ object: '5', verdict: { kind: 'non_manifold' } }),
    ])
    expect(r.rows.map(rowClass)).toEqual(['surface', 'surface', 'open', 'surface', 'open'])
    const summary = parseHealthSummary(
      JSON.stringify({
        counts: { ...EMPTY_COUNTS, objects: 9, solids: 4 },
        open: [['1', 'flat_sheet', false], ['2', 'bent_sheet', false], ['3', 'cappable', false], ['4', 'mixed', true], ['5', 'non_manifold', false]],
      }),
    )
    expect(summary.open[0]).toEqual({ object: 1n, kind: 'flat_sheet', accepted: false })
    expect(summarize(summary)).toEqual({ objects: 9, solids: 4, surfaces: 3, open: 2 })
    const states = objectHealthStates(summary)
    expect(states.get(1n)).toBe('surface')
    expect(states.get(3n)).toBe('open')
    expect(states.get(99n)).toBeUndefined()
  })
})

describe('filtering', () => {
  const r = report([
    row({ object: '1', name: 'Roof', verdict: { kind: 'cappable' } }),
    row({ object: '2', name: 'Floor', verdict: { kind: 'flat_sheet', holes: 0 } }),
    row({ object: '3', name: 'Chair leg', verdict: { kind: 'holes', planar: 1, warped: 0 } }),
    row({ object: '4', name: 'Hidden', verdict: { kind: 'bent_sheet' }, accepted: true }),
  ])

  it('hides accepted rows unless asked', () => {
    expect(filterRows(r.rows, { query: '', showAccepted: false }).map((x) => x.name)).toEqual(['Roof', 'Floor', 'Chair leg'])
    expect(filterRows(r.rows, { query: '', showAccepted: true })).toHaveLength(4)
  })

  it('matches the name, case-insensitively', () => {
    expect(filterRows(r.rows, { query: 'ROOF', showAccepted: false }).map((x) => x.name)).toEqual(['Roof'])
  })

  it('matches the detail and the group name', () => {
    expect(filterRows(r.rows, { query: 'flat panel', showAccepted: false }).map((x) => x.name)).toEqual(['Floor'])
    expect(filterRows(r.rows, { query: 'has openings', showAccepted: false }).map((x) => x.name)).toEqual(['Roof', 'Chair leg'])
  })

  it('an object filter lists only that object, accepted or not', () => {
    expect(filterRows(r.rows, { query: '', showAccepted: false, object: 4n }).map((x) => x.name)).toEqual(['Hidden'])
    expect(filterRows(r.rows, { query: '', showAccepted: false, object: 1n }).map((x) => x.name)).toEqual(['Roof'])
  })
})

describe('grouping', () => {
  it('groups by kind in display order, drops empty groups, counts rows', () => {
    const r = report([
      row({ object: '1', verdict: { kind: 'non_manifold' } }),
      row({ object: '2', verdict: { kind: 'cappable' } }),
      row({ object: '3', verdict: { kind: 'cappable' } }),
      row({ object: '4', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [] }),
    ])
    const g = groupRows(r, r.rows)
    expect(g.groups.map((x) => [x.label, x.rows.length])).toEqual([
      ['Has openings', 2],
      ['Flat panels', 1],
      ["Needs mending", 1],
    ])
    expect(g.families).toEqual([])
  })

  it('lists split-shape rows under "<name> → N pieces", once, not in their kind group', () => {
    const r = report(
      [
        row({ object: '1', name: 'Group#18', family: 0, verdict: { kind: 'cappable' } }),
        row({ object: '2', name: 'Group#18', family: 0, verdict: { kind: 'flat_sheet', holes: 0 } }),
        row({ object: '3', name: 'Other', verdict: { kind: 'cappable' } }),
      ],
      [{ members: ['1', '2', '8'], shared_rims: 1 }],
    )
    const g = groupRows(r, r.rows)
    expect(g.groups.map((x) => [x.label, x.rows.length])).toEqual([['Has openings', 1]])
    expect(g.families).toHaveLength(1)
    expect(g.families[0].title).toBe('Group#18 → 3 pieces')
    expect(g.families[0].rows.map((x) => x.object)).toEqual([1n, 2n])
    // Display order: openings, split shapes.
    expect(groupedRows(g).map((x) => x.object)).toEqual([3n, 1n, 2n])
  })
})

describe('segmentBounds', () => {
  it('bounds a segment list; null when empty', () => {
    expect(segmentBounds(new Float32Array([0, 1, 2, 3, -1, 5]))).toEqual({ min: [0, -1, 2], max: [3, 1, 5] })
    expect(segmentBounds(new Float32Array([]))).toBeNull()
  })
})

describe('canCloseRow', () => {
  const one = (over: Record<string, unknown>): HealthRow => report([row(over)]).rows[0]
  const WARPED = { ...RIM, planar: false, cappable: false }

  it('offers Close when some opening can take a face', () => {
    expect(canCloseRow(one({}))).toBe(true)
    expect(canCloseRow(one({ verdict: { kind: 'mixed' }, rims: [RIM, WARPED] }))).toBe(true)
  })

  it('does not offer Close when every opening is warped', () => {
    expect(canCloseRow(one({ verdict: { kind: 'mixed' }, rims: [WARPED] }))).toBe(false)
    expect(canCloseRow(one({ verdict: { kind: 'holes', planar: 1 }, rims: [WARPED, WARPED] }))).toBe(false)
  })

  it('keeps the verdict answer when the report left the rims out', () => {
    expect(canCloseRow(one({ rims: [] }))).toBe(true)
  })

  it('never offers Close for other verdicts', () => {
    expect(canCloseRow(one({ verdict: { kind: 'non_manifold' } }))).toBe(false)
  })
})
