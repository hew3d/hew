/**
 * fixPlan — the five kinds, the one ordered command list, the scope
 * defaults and per-item ticks, the thickness choices, and the Ready
 * sentences (docs/design/import-health.md, Revision 3).
 */
import { describe, expect, it } from 'vitest'
import { EMPTY_COUNTS, parseHealthReport, type HealthReport } from './healthReport'
import { buildPlan, carryPerItem, classifyRow, defaultChoices, itemActs, itemDetail, leftByChoice, leftTotal, pagesWithItems, planCommands, readySentences, type FixChoices } from './fixPlan'

const RIM = { role: 'outer', planar: true, closed: true, cappable: true, sheet_edge: false, shared: false, perimeter: 4, extent: 1, edges: 4 }
const SEAM = { ...RIM, shared: true }
const WARPED = { ...RIM, planar: false, cappable: false }

function row(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    object: '1',
    name: 'Thing',
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

function report(rows: Record<string, unknown>[], families: unknown[] = [], solids = 3): HealthReport {
  return parseHealthReport(JSON.stringify({ rows, families, counts: { ...EMPTY_COUNTS, objects: solids + rows.length, solids } }))
}

const choices = (over: Partial<FixChoices> = {}): FixChoices => ({ ...defaultChoices(), ...over })
const ticks = (entries: [string, boolean][]): ReadonlyMap<string, boolean> => new Map(entries)
const methods = (r: ReturnType<typeof planCommands>) => {
  if (!r.ok) throw new Error(r.error)
  return r.ops.map((o) => [o.method, o.object, o.params ?? null])
}

describe('classification', () => {
  it('every verdict / rim combination lands on exactly one kind', () => {
    const kind = (over: Record<string, unknown>) => classifyRow(report([row(over)]).rows[0])
    expect(kind({ verdict: { kind: 'cappable' } })).toBe('openings')
    expect(kind({ verdict: { kind: 'holes', planar: 2, warped: 0 }, rims: [RIM, RIM] })).toBe('openings')
    expect(kind({ verdict: { kind: 'mixed' }, rims: [RIM, RIM] })).toBe('openings')
    expect(kind({ verdict: { kind: 'holes', planar: 1, warped: 1 }, rims: [RIM, WARPED] })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'mixed' }, rims: [WARPED] })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'non_manifold' } })).toBe('closerLook')
    expect(kind({ verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] })).toBe('flatPanels')
    expect(kind({ verdict: { kind: 'bent_sheet' }, rims: [WARPED] })).toBe('curvedSheets')
    expect(kind({ family: 0 })).toBe('split')
  })

  it('builds items per kind: one per object, one per split shape; kept rows are listed too', () => {
    const r = report(
      [
        row({ object: '1', name: 'Wall' }),
        row({ object: '2', name: 'Chair leg', definition: '9', definition_name: 'Chair', placements: 4, instances: ['50'] }),
        row({ object: '3', name: 'Glass', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
        row({ object: '4', name: 'Hill', verdict: { kind: 'bent_sheet' }, rims: [WARPED] }),
        row({ object: '5', name: 'Knot', verdict: { kind: 'non_manifold' } }),
        row({ object: '6', name: 'Old glass', verdict: { kind: 'flat_sheet', holes: 0 }, accepted: true, rims: [WARPED] }),
        row({ object: '7', name: 'Group#1', family: 0 }),
        row({ object: '8', name: 'Group#1', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
      ],
      [{ members: ['7', '8'], shared_rims: 1 }],
      2,
    )
    const plan = buildPlan(r)
    expect(plan.items.openings.map((i) => [i.name, i.copies])).toEqual([['Wall', 1], ['Chair', 4]])
    expect(plan.items.flatPanels.map((i) => i.name)).toEqual(['Glass', 'Old glass'])
    expect(plan.items.curvedSheets.map((i) => i.name)).toEqual(['Hill'])
    expect(plan.items.closerLook.map((i) => i.name)).toEqual(['Knot'])
    expect(plan.items.split.map((i) => [i.key, i.name, i.rows.length])).toEqual([['family-0', 'Group#1', 2]])
    expect(pagesWithItems(plan)).toEqual(['openings', 'split', 'flatPanels', 'curvedSheets', 'closerLook'])
    // Counts: 2 solids; the open rows are Wall, Chair, Knot, Group#1 piece 1 (the sheets are panels).
    expect(plan.objects).toBe(10)
    expect(plan.solids).toBe(2)
    expect(plan.fix).toBe(4)
    expect(plan.panels).toBe(4)
  })

  it('pages with nothing on them are skipped', () => {
    const plan = buildPlan(report([row({ object: '1' })]))
    expect(pagesWithItems(plan)).toEqual(['openings'])
    expect(pagesWithItems(buildPlan(report([])))).toEqual([])
  })
})

describe('defaults', () => {
  it('a model or an import leaves openings open; a selection closes them', () => {
    expect(defaultChoices('model').openings).toBe('leave')
    expect(defaultChoices('import').openings).toBe('leave')
    expect(defaultChoices('selection').openings).toBe('close')
    expect(defaultChoices().openings).toBe('leave')
    expect(defaultChoices('selection').split).toBe('rejoin')
    expect(defaultChoices('selection').flatPanels).toBe('keep')
  })
})

describe('commands', () => {
  const r = report(
    [
      row({ object: '1', name: 'Wall' }),
      row({ object: '2', name: 'Glass', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
      row({ object: '3', name: 'Hill', verdict: { kind: 'bent_sheet' }, rims: [WARPED] }),
      row({ object: '4', name: 'Knot', verdict: { kind: 'non_manifold' } }),
      row({ object: '5', name: 'Group#1', family: 0, rims: [SEAM, RIM] }),
      row({ object: '6', name: 'Group#1', family: 0, rims: [SEAM] }),
      row({ object: '7', name: 'Group#1', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
    ],
    [{ members: ['5', '6', '7'], shared_rims: 1 }],
  )
  const plan = buildPlan(r)

  it('the model default closes the seams of split shapes and deletes the wall, and writes nothing else', () => {
    expect(methods(planCommands(plan, defaultChoices('model')))).toEqual([
      ['hew.solid.cap', 5n, { rims: [0] }],
      ['hew.solid.cap', 6n, { rims: [0] }],
      ['hew.solid.delete_faces', 7n, null],
    ])
  })

  it('the selection default also closes the openings', () => {
    expect(methods(planCommands(plan, defaultChoices('selection')))).toEqual([
      ['hew.solid.cap', 5n, { rims: [0] }],
      ['hew.solid.cap', 6n, { rims: [0] }],
      ['hew.solid.delete_faces', 7n, null],
      ['hew.solid.cap', 1n, null],
    ])
  })

  it('keep the inside walls emits no wall operation', () => {
    const cmds = methods(planCommands(plan, choices({ split: 'keepWalls' })))
    expect(cmds).toEqual([
      ['hew.solid.cap', 5n, { rims: [0] }],
      ['hew.solid.cap', 6n, { rims: [0] }],
    ])
  })

  it('a piece with no shared rim emits no cap', () => {
    const lone = buildPlan(
      report(
        [row({ object: '1', family: 0, rims: [RIM] }), row({ object: '2', family: 0, rims: [SEAM] }), row({ object: '3', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] })],
        [{ members: ['1', '2', '3'], shared_rims: 1 }],
      ),
    )
    expect(methods(planCommands(lone, defaultChoices()))).toEqual([
      ['hew.solid.cap', 2n, { rims: [0] }],
      ['hew.solid.delete_faces', 3n, null],
    ])
  })

  it('leaving everything writes nothing, and never marks a panel or sheet as kept', () => {
    const cmds = methods(planCommands(plan, choices({ split: 'leave', openings: 'leave' })))
    expect(cmds).toEqual([])
    for (const choice of [defaultChoices('model'), defaultChoices('selection'), choices({ flatPanels: 'thicken' })]) {
      for (const [method] of methods(planCommands(plan, choice))) expect(method).not.toBe('hew.solid.mark_surface')
    }
  })

  it('thickness choices: In front, Behind and Centered map to the thicken command', () => {
    const t = (side: 'front' | 'behind' | 'centered') =>
      methods(planCommands(plan, choices({ split: 'leave', flatPanels: 'thicken', thickness: { text: '3 mm', side } })))
    expect(t('front')).toEqual([['hew.solid.thicken', 2n, { distance: 0.003 }]])
    expect(t('behind')).toEqual([['hew.solid.thicken', 2n, { distance: -0.003 }]])
    expect(t('centered')).toEqual([['hew.solid.thicken', 2n, { distance: 0.003, centered: true }]])
  })

  it('a thickness that does not parse is refused with the plain message', () => {
    const bad = planCommands(plan, choices({ flatPanels: 'thicken', thickness: { text: 'abc', side: 'front' } }))
    expect(bad).toEqual({ ok: false, error: 'Enter a thickness, such as 3 mm.' })
    const zero = planCommands(plan, choices({ flatPanels: 'thicken', thickness: { text: '0', side: 'front' } }))
    expect(zero).toEqual({ ok: false, error: 'The thickness must be more than zero.' })
    // Not asked about when no panel would be thickened.
    const none = planCommands(buildPlan(report([row()])), choices({ flatPanels: 'thicken', thickness: { text: 'abc', side: 'front' } }))
    expect(none.ok).toBe(true)
  })

  it('a ticked panel is thickened under Leave, and its thickness is parsed', () => {
    const one = choices({ perItem: ticks([['2', true]]), thickness: { text: '2 mm', side: 'front' } })
    expect(methods(planCommands(plan, { ...one, split: 'leave' }))).toEqual([['hew.solid.thicken', 2n, { distance: 0.002 }]])
    const bad = planCommands(plan, { ...one, thickness: { text: 'abc', side: 'front' } })
    expect(bad.ok).toBe(false)
  })

  it('ticking one opening under Leave closes exactly that one; unticking one under Close skips it', () => {
    const two = buildPlan(report([row({ object: '1' }), row({ object: '2' }), row({ object: '3' })]))
    expect(methods(planCommands(two, choices({ openings: 'leave', perItem: ticks([['2', true]]) })))).toEqual([['hew.solid.cap', 2n, null]])
    expect(methods(planCommands(two, choices({ openings: 'close', perItem: ticks([['2', false]]) })))).toEqual([
      ['hew.solid.cap', 1n, null],
      ['hew.solid.cap', 3n, null],
    ])
  })

  it('ticks never reach curved sheets or items that can not be closed', () => {
    const items = plan.items
    const all = choices({ perItem: ticks([['3', true], ['4', true]]) })
    expect(itemActs(items.curvedSheets[0], all)).toBe(false)
    expect(itemActs(items.closerLook[0], all)).toBe(false)
  })

  it('a split shape can be unticked as a whole', () => {
    expect(methods(planCommands(plan, choices({ perItem: ticks([['family-0', false]]) })))).toEqual([])
  })

  it('rejoinSolid emits one rejoin op with the family and no cap ops', () => {
    const cmds = planCommands(plan, choices({ split: 'rejoinSolid' }))
    if (!cmds.ok) throw new Error(cmds.error)
    expect(cmds.ops).toEqual([
      { object: 5n, method: 'hew.solid.rejoin', params: { family: 0, as: 'one_solid' }, members: [5n, 6n, 7n] },
    ])
    // A real cascade lookup: a refused rejoin takes nothing else with it.
    expect(cmds.options.cascade!(cmds.ops[0], cmds.ops)).toBeNull()
    // And the cascade does fire for a refused cap in the default plan.
    const dflt = planCommands(plan, defaultChoices('model'))
    if (!dflt.ok) throw new Error(dflt.error)
    expect(dflt.options.cascade!(dflt.ops[0], dflt.ops)?.ops).toEqual([dflt.ops[2]])
  })

  it('several rejoins run by descending family index, so earlier ones do not shift later indices', () => {
    const two = buildPlan(
      report(
        [
          row({ object: '1', family: 0, rims: [SEAM] }),
          row({ object: '2', family: 0, rims: [SEAM] }),
          row({ object: '3', family: 1, rims: [SEAM] }),
          row({ object: '4', family: 1, rims: [SEAM] }),
        ],
        [{ members: ['1', '2'], shared_rims: 1 }, { members: ['3', '4'], shared_rims: 1 }],
      ),
    )
    const cmds = planCommands(two, choices({ split: 'rejoinSolid' }))
    if (!cmds.ok) throw new Error(cmds.error)
    expect(cmds.ops.map((o) => o.params)).toEqual([
      { family: 1, as: 'one_solid' },
      { family: 0, as: 'one_solid' },
    ])
  })

  it('rejoin and keepWalls plans are unchanged', () => {
    expect(methods(planCommands(plan, choices({ split: 'rejoin' })))).toEqual([
      ['hew.solid.cap', 5n, { rims: [0] }],
      ['hew.solid.cap', 6n, { rims: [0] }],
      ['hew.solid.delete_faces', 7n, null],
    ])
    expect(methods(planCommands(plan, choices({ split: 'keepWalls' })))).toEqual([
      ['hew.solid.cap', 5n, { rims: [0] }],
      ['hew.solid.cap', 6n, { rims: [0] }],
    ])
  })

  it('per-item unticking removes the rejoin op', () => {
    expect(methods(planCommands(plan, choices({ split: 'rejoinSolid', perItem: ticks([['family-0', false]]) })))).toEqual([])
  })

  it('a refused piece only keeps the wall of its own split shape', () => {
    const two = buildPlan(
      report(
        [
          row({ object: '1', family: 0, rims: [SEAM] }),
          row({ object: '2', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
          row({ object: '3', family: 1, rims: [SEAM] }),
          row({ object: '4', family: 1, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
        ],
        [{ members: ['1', '2'], shared_rims: 1 }, { members: ['3', '4'], shared_rims: 1 }],
      ),
    )
    const cmds = planCommands(two, defaultChoices())
    if (!cmds.ok) throw new Error(cmds.error)
    const [cap0, cap1, wall0, wall1] = cmds.ops
    const dropped = cmds.options.cascade!(cap0, cmds.ops)
    expect(dropped?.ops).toEqual([wall0])
    expect(cmds.options.cascade!(cap1, cmds.ops)?.ops).toEqual([wall1])
    expect(cmds.options.cascade!(wall0, cmds.ops)).toBeNull()
  })
})

describe('Ready sentences', () => {
  const plan = buildPlan(
    report(
      [
        row({ object: '1' }),
        row({ object: '2' }),
        row({ object: '3', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
        row({ object: '4', verdict: { kind: 'bent_sheet' }, rims: [WARPED] }),
        row({ object: '5', verdict: { kind: 'non_manifold' } }),
        row({ object: '6', family: 0, rims: [SEAM] }),
      ],
      [{ members: ['6', '9'], shared_rims: 1 }],
    ),
  )

  it('one sentence per decision, then how many stay as they are', () => {
    expect(readySentences(plan, defaultChoices('model'))).toEqual([
      'Rejoin 1 split shape.',
      'Leave 5 objects as they are.',
    ])
    expect(readySentences(plan, defaultChoices('selection'))).toEqual([
      'Rejoin 1 split shape.',
      'Close the openings in 2 shapes.',
      'Leave 3 objects as they are.',
    ])
  })

  it('rejoinSolid ready sentence names one solid', () => {
    expect(readySentences(plan, choices({ split: 'rejoinSolid' }))).toEqual([
      'Rejoin 1 split shape as one solid.',
      'Leave 5 objects as they are.',
    ])
  })

  it('rejoinSolid ready sentence is plural for several shapes', () => {
    const two = buildPlan(
      report(
        [
          row({ object: '1', family: 0, rims: [SEAM] }),
          row({ object: '2', family: 0, rims: [SEAM] }),
          row({ object: '3', family: 1, rims: [SEAM] }),
          row({ object: '4', family: 1, rims: [SEAM] }),
        ],
        [{ members: ['1', '2'], shared_rims: 1 }, { members: ['3', '4'], shared_rims: 1 }],
      ),
    )
    expect(readySentences(two, choices({ split: 'rejoinSolid' }))[0]).toBe('Rejoin 2 split shapes as one solid.')
  })

  it('reflects non-default choices', () => {
    expect(readySentences(plan, choices({ split: 'keepWalls', openings: 'close', flatPanels: 'thicken', thickness: { text: ' 3 mm ', side: 'centered' } }))).toEqual([
      'Rejoin 1 split shape and keep the inside walls.',
      'Close the openings in 2 shapes.',
      'Give 1 flat panel a thickness of 3 mm.',
      'Leave 2 objects as they are.',
    ])
  })

  it('says so when nothing would change', () => {
    const only = buildPlan(report([row({ object: '1' })]))
    expect(readySentences(only, defaultChoices('model'))).toEqual(['Nothing will change. Go back to make a choice, or close this window.'])
    expect(readySentences(only, choices({ openings: 'close', perItem: ticks([['1', false]]) }))).toEqual([
      'Nothing will change. Go back to make a choice, or close this window.',
    ])
  })

  it('the Ready page leave count covers every page, can not be closed included', () => {
    expect(leftTotal(plan, defaultChoices('model'))).toBe(5)
    expect(leftTotal(plan, defaultChoices('selection'))).toBe(3)
    expect(leftTotal(plan, choices({ openings: 'close', perItem: ticks([['1', false]]) }))).toBe(4)
  })

  it('the "left as you chose" count covers only the pages with a choice', () => {
    // Curved sheets and can't-be-closed items were never a choice.
    expect(leftByChoice(plan, defaultChoices('model'))).toBe(3)
    expect(leftByChoice(plan, defaultChoices('selection'))).toBe(1)
    expect(leftByChoice(plan, choices({ openings: 'close', perItem: ticks([['1', false]]) }))).toBe(2)
  })
})

describe('families that are not split shapes', () => {
  const WALL = { verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }
  const fam = [{ members: ['1', '2'], shared_rims: 1 }]

  it('a family with a piece that cannot be closed is classified member by member, and no wall is deleted', () => {
    const plan = buildPlan(
      report([row({ object: '1', family: 0, verdict: { kind: 'holes', planar: 0, warped: 1 }, rims: [WARPED] }), row({ object: '2', family: 0, ...WALL })], fam),
    )
    expect(plan.items.split).toHaveLength(0)
    expect(plan.items.closerLook.map((i) => i.rows[0].object)).toEqual([1n])
    expect(plan.items.flatPanels.map((i) => i.rows[0].object)).toEqual([2n])
    expect(methods(planCommands(plan, defaultChoices()))).toEqual([])
  })

  it('a family whose only members are walls (the pieces are outside the scope) is a flat panel', () => {
    const plan = buildPlan(report([row({ object: '2', family: 0, ...WALL })], fam))
    expect(plan.items.split).toHaveLength(0)
    expect(plan.items.flatPanels).toHaveLength(1)
  })

  it('a family whose piece was kept in Model Health is still a split shape', () => {
    const plan = buildPlan(
      report([row({ object: '1', family: 0, accepted: true }), row({ object: '2', family: 0, ...WALL })], fam),
    )
    expect(plan.items.split).toHaveLength(1)
    expect(plan.items.flatPanels).toHaveLength(0)
  })

  it('a ticked split shape never passes its tick to its wall if the wall turns up alone', () => {
    const shape = buildPlan(
      report([row({ object: '1', family: 0, verdict: { kind: 'mixed' }, rims: [SEAM] }), row({ object: '2', family: 0, ...WALL })], fam),
    )
    expect(shape.items.split).toHaveLength(1)
    const ticked = new Map([[shape.items.split[0].key, true]])
    // The piece is closed by hand: only the wall is left, now a flat panel.
    const after = buildPlan(report([row({ object: '2', ...WALL })]))
    expect(after.items.flatPanels).toHaveLength(1)
    expect(carryPerItem(shape, after, ticked).size).toBe(0)
  })

  it("an item's detail follows its page, not the raw family", () => {
    // A family that is not a split shape: its piece is listed as can't be closed, with the reason.
    const plan = buildPlan(
      report([row({ object: '1', family: 0, verdict: { kind: 'non_manifold' }, branching_vertices: 1 }), row({ object: '2', family: 0, ...WALL })], fam),
    )
    expect(plan.items.closerLook).toHaveLength(1)
    expect(itemDetail(plan.items.closerLook[0])).toBe('Openings tangled together')
  })

  it('a piece with a shared rim makes a split shape; its seams are capped and the wall deleted', () => {
    const plan = buildPlan(
      report([row({ object: '1', family: 0, verdict: { kind: 'mixed' }, rims: [SEAM, RIM] }), row({ object: '2', family: 0, ...WALL })], fam),
    )
    expect(plan.items.split).toHaveLength(1)
    expect(methods(planCommands(plan, defaultChoices()))).toEqual([
      ['hew.solid.cap', 1n, { rims: [0] }],
      ['hew.solid.delete_faces', 2n, null],
    ])
  })

  it('keeping the inside walls still waits for the piece to close', () => {
    const plan = buildPlan(report([row({ object: '1', family: 0, rims: [SEAM] }), row({ object: '2', family: 0, ...WALL })], fam))
    const cmds = planCommands(plan, choices({ split: 'keepWalls' }))
    if (!cmds.ok) throw new Error(cmds.error)
    expect(cmds.ops.map((o) => o.method)).toEqual(['hew.solid.cap'])
    expect(cmds.options.cascade!(cmds.ops[0], cmds.ops)).toBeNull()
  })
})

describe('freshness helpers', () => {
  it('ticks stay with their object across a re-read, even when split shape numbers move', () => {
    const before = buildPlan(
      report([row({ object: '1' }), row({ object: '5', family: 0, rims: [SEAM] }), row({ object: '6', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] })], [
        { members: ['5', '6'], shared_rims: 1 },
      ]),
    )
    const after = buildPlan(
      report([row({ object: '5', family: 3, rims: [SEAM] }), row({ object: '6', family: 3, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }), row({ object: '8' })], [
        {}, {}, {}, { members: ['5', '6'], shared_rims: 1 },
      ]),
    )
    const out = carryPerItem(before, after, ticks([['1', true], ['family-0', false]]))
    expect([...out]).toEqual([['family-3', false]])
  })
})

describe('needs mending (Mend)', () => {
  const CENSUS = {
    weld: 0.000002,
    weldVertices: 312,
    weldMaxGap: 0,
    folds: 0,
    branchingVertices: 2,
    islands: 1,
    pieces: 0,
    fillRims: 7,
    fillFlat: 3,
    fillCurved: 4,
    skippedRims: 0,
  }
  const NOTHING = { ...CENSUS, weldVertices: 0, branchingVertices: 0, fillRims: 0 }
  const rows = [
    row({ object: '1', name: 'Knot', verdict: { kind: 'non_manifold' }, branching_vertices: 2 }),
    row({ object: '2', name: 'Sock', verdict: { kind: 'holes', planar: 0, warped: 1 }, rims: [WARPED] }),
    row({ object: '3', name: 'Pipe fitting', verdict: { kind: 'non_manifold' }, branching_vertices: 1 }),
  ]
  const census = (r: { object: bigint }) => (r.object === 3n ? NOTHING : CENSUS)
  const plan = buildPlan(report(rows), census)

  it('items with something to mend carry the census; the rest do not', () => {
    expect(plan.items.closerLook.map((i) => [i.name, i.mend !== null])).toEqual([
      ['Knot', true],
      ['Sock', true],
      ['Pipe fitting', false],
    ])
  })

  it('Mend is on by default and writes one mend op per item with something to mend, in the one transaction', () => {
    expect(defaultChoices().mend).toBe('mend')
    const r = planCommands(plan, defaultChoices())
    expect(methods(r)).toEqual([
      ['hew.solid.mend', 1n, { weld: true, folds: true, split: true, fill: 'holes' }],
      ['hew.solid.mend', 2n, { weld: true, folds: true, split: true, fill: 'holes' }],
    ])
    expect(readySentences(plan, defaultChoices())).toEqual(['Mend 2 objects.', 'Leave 1 object as it is.'])
  })

  it('leaving them writes nothing; a tick on one item wins over the page', () => {
    expect(methods(planCommands(plan, choices({ mend: 'leave' })))).toEqual([])
    const one = choices({ mend: 'leave', perItem: ticks([['2', true]]) })
    expect(methods(planCommands(plan, one)).map((m) => m[1])).toEqual([2n])
    const off = choices({ perItem: ticks([['1', false]]) })
    expect(methods(planCommands(plan, off)).map((m) => m[1])).toEqual([2n])
  })

  it('an item with nothing to mend never acts, whatever its tick', () => {
    expect(itemActs(plan.items.closerLook[2], choices({ perItem: ticks([['3', true]]) }))).toBe(false)
  })

  it('"left as you chose" counts the mendable items left alone', () => {
    expect(leftByChoice(plan, defaultChoices())).toBe(0)
    expect(leftByChoice(plan, choices({ mend: 'leave' }))).toBe(2)
  })

  it('without a census every item stays as before and nothing mends', () => {
    const bare = buildPlan(report(rows))
    expect(bare.items.closerLook.every((i) => i.mend === null)).toBe(true)
    expect(methods(planCommands(bare, defaultChoices()))).toEqual([])
  })
})
