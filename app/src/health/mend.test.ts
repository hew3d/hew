import { describe, expect, it, vi } from 'vitest'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  DEFAULT_MEND_PLAN,
  formatMergeDistance,
  hasMendWork,
  mendOp,
  mendPlanParams,
  mendItemLine,
  mendPreviewLine,
  mendResultSentence,
  mendToast,
  parseMendCensus,
  parseMendOutcome,
  parseMergeDistance,
  concatPoints,
  readMendPoints,
  readMendCensusResult,
  mendIdleLine,
  readMendCensus,
  skippedLine,
  type MendCensus,
  type MendPlan,
} from './mend'
import { BANNED_RE } from './bannedWords'
import { getLengthUnit, setLengthUnit } from '../settings/units'
import type { BatchOp, BatchResult } from './healthBatch'
import type { HealthRow } from './healthReport'

const CENSUS: MendCensus = {
  weld: 0.000002,
  weldVertices: 312,
  weldMaxGap: 0.000001,
  folds: 4,
  branchingVertices: 2,
  islands: 1,
  pieces: 0,
  fillRims: 7,
  fillFlat: 3,
  fillCurved: 4,
  skippedRims: 0,
}

describe('mendPreviewLine', () => {
  it('names every nonzero step in plain words', () => {
    expect(mendPreviewLine(CENSUS)).toBe(
      'Mend will merge 312 nearby points, separate 2 spots where parts touch at a single point, fill 7 small gaps (3 flat, 4 curved) and remove up to 4 paper-thin overlaps.',
    )
  })

  it('leaves a zero step out and joins the rest', () => {
    expect(mendPreviewLine({ ...CENSUS, folds: 0 })).toBe(
      'Mend will merge 312 nearby points, separate 2 spots where parts touch at a single point and fill 7 small gaps (3 flat, 4 curved).',
    )
    expect(mendPreviewLine({ ...CENSUS, weldVertices: 1, branchingVertices: 0, fillRims: 0, folds: 0 })).toBe(
      'Mend will merge 1 nearby point.',
    )
  })

  it('drops the flat / curved split when it does not add up (seams count as gaps)', () => {
    expect(mendPreviewLine({ ...CENSUS, folds: 0, branchingVertices: 0, fillRims: 9, weldVertices: 0 })).toBe(
      'Mend will fill 9 small gaps.',
    )
  })

  it('counts the loose parts of an object with no pinch', () => {
    expect(
      mendPreviewLine({ ...CENSUS, weldVertices: 0, folds: 0, fillRims: 0, branchingVertices: 0, islands: 3, pieces: 2 }),
    ).toBe('Mend will separate 2 loose parts.')
  })

  it('counts every new object the commit makes, beside the pinch it splits', () => {
    // One pinch point and a separate floating island: the commit makes two
    // new objects, and the preview says so.
    const c = { ...CENSUS, weldVertices: 0, folds: 0, fillRims: 0, branchingVertices: 1, islands: 3, pieces: 2 }
    expect(mendPreviewLine(c)).toBe(
      'Mend will separate 1 spot where parts touch at a single point and separate 2 loose parts.',
    )
    expect(hasMendWork({ ...c, branchingVertices: 0 })).toBe(true)
    // With the split off, nothing is separated.
    expect(mendPreviewLine(c, { ...DEFAULT_MEND_PLAN, split: false })).toBeNull()
  })

  it('says how many gaps will stay open, and only when the fill is on', () => {
    expect(mendPreviewLine({ ...CENSUS, skippedRims: 1 })).toMatch(/ 1 gap will stay open\.$/)
    expect(mendPreviewLine({ ...CENSUS, skippedRims: 3 })).toMatch(/fill 7 small gaps \(3 flat, 4 curved\)[^.]*\. 3 gaps will stay open\.$/)
    expect(mendPreviewLine({ ...CENSUS, skippedRims: 0 })).not.toMatch(/stay open/)
    expect(mendPreviewLine({ ...CENSUS, skippedRims: 2 }, { ...DEFAULT_MEND_PLAN, fill: false })).not.toMatch(/stay open/)
    expect(mendItemLine({ ...CENSUS, skippedRims: 2 })).toMatch(/2 gaps will stay open\.$/)
  })

  it('is null when nothing is left to do, and steps switched off count zero', () => {
    const none = { ...CENSUS, weldVertices: 0, folds: 0, branchingVertices: 0, fillRims: 0 }
    expect(mendPreviewLine(none)).toBeNull()
    expect(hasMendWork(none)).toBe(false)
    const off: MendPlan = { weld: false, folds: false, split: false, fill: true }
    expect(mendPreviewLine(CENSUS, off)).toBe('Mend will fill 7 small gaps (3 flat, 4 curved).')
    expect(hasMendWork(CENSUS, { weld: false, folds: false, split: false, fill: false })).toBe(false)
  })

  it('never uses a banned word', () => {
    expect(mendPreviewLine(CENSUS)).not.toMatch(BANNED_RE)
  })
})

describe('plan params', () => {
  it('maps the person\'s choices to the API form', () => {
    expect(mendPlanParams(DEFAULT_MEND_PLAN)).toEqual({ weld: true, folds: true, split: true, fill: 'holes' })
    expect(mendPlanParams({ weld: 0.000003, folds: false, split: false, fill: false })).toEqual({
      weld: 0.000003,
      folds: false,
      split: false,
      fill: false,
    })
    const row = { object: 5n } as HealthRow
    expect(mendOp(row)).toEqual({ object: 5n, method: 'hew.solid.mend', params: mendPlanParams(DEFAULT_MEND_PLAN) })
  })
})

describe('parseMendCensus / readMendCensus', () => {
  const wire = {
    weld: 0.000002,
    weld_pairs: 400,
    weld_vertices: 312,
    weld_max_gap: 0.000001,
    folds: 4,
    branching_vertices: 2,
    islands: 1,
    pieces: 0,
    hole_rims: 7,
    hole_rims_planar: 3,
    hole_rims_warped: 4,
    outer_rims: 0,
    fill_rims: [0, 1, 2, 3, 4, 5, 6],
    filled_rims: 7,
    fill_curved: 4,
    skipped_rims: 0,
  }

  it('reads the wire form', () => {
    expect(parseMendCensus(JSON.stringify(wire))).toEqual(CENSUS)
    expect(parseMendCensus(JSON.stringify({ ...wire, weld: null })).weld).toBeNull()
  })

  it('counts the gaps the fill closes, not the rims it reaches', () => {
    // A gap the merge made from two rims names both rims but is one fill;
    // a rim the merge closes is no fill at all.
    expect(parseMendCensus(JSON.stringify({ ...wire, fill_rims: [0, 2, 5], filled_rims: 2, fill_curved: 0 })).fillRims).toBe(2)
    expect(parseMendCensus(JSON.stringify({ ...wire, fill_rims: [], filled_rims: 0, fill_curved: 0 })).fillRims).toBe(0)
    const kinds = parseMendCensus(JSON.stringify({ ...wire, filled_rims: 5, fill_curved: 2 }))
    expect([kinds.fillFlat, kinds.fillCurved]).toEqual([3, 2])
  })

  it('asks once per history generation and plan', () => {
    let generation = 1n
    const scene = {
      mend_census_json: vi.fn(() => JSON.stringify(wire)),
      history_generation: () => generation,
    } as unknown as WasmScene
    expect(readMendCensus(scene, 1n)).toEqual(CENSUS)
    readMendCensus(scene, 1n)
    expect(scene.mend_census_json).toHaveBeenCalledTimes(1)
    readMendCensus(scene, 1n, { ...DEFAULT_MEND_PLAN, weld: false })
    expect(scene.mend_census_json).toHaveBeenCalledTimes(2)
    generation = 2n
    readMendCensus(scene, 1n)
    expect(scene.mend_census_json).toHaveBeenCalledTimes(3)
  })

  it('is null for a scene without a census or a refused read', () => {
    expect(readMendCensus({} as WasmScene, 1n)).toBeNull()
    const refusing = {
      mend_census_json: () => {
        throw new Error('UnknownObject: gone')
      },
    } as unknown as WasmScene
    expect(readMendCensus(refusing, 1n)).toBeNull()
  })
})

describe('result sentences', () => {
  const full = { welded_vertices: 312, split_vertices: 2, filled_rims: 7, folds_removed: 0, closed: true, skipped_rims: [] }

  it('one object that closed', () => {
    expect(mendResultSentence([parseMendOutcome(full)])).toBe("Merged 312 points, separated 2 spots where parts touched at a single point, filled 7 gaps. It's solid now.")
  })

  it('gaps left open point at the list, with each reason in plain words', () => {
    const o = parseMendOutcome({
      ...full,
      closed: false,
      skipped_rims: [
        { object: 0, rim: 0, reason: 'FillCrossesFaces' },
        { object: 0, rim: 1, reason: 'FillCrossesFaces' },
        { object: 0, rim: 2, reason: 'FillRimTooSmall' },
      ],
    })
    expect(mendResultSentence([o])).toBe("Merged 312 points, separated 2 spots where parts touched at a single point, filled 7 gaps. 3 gaps couldn't be filled; see the list.")
    expect(skippedLine(o.skipped)).toBe(
      '3 gaps stayed open. Filling this gap would cut through other faces, so it was left open. This gap is too small or thin to fill.',
    )
    expect(skippedLine([])).toBeNull()
  })

  it('reads a reason by its refusal code as well as by the kernel name', () => {
    const o = parseMendOutcome({ ...full, skipped_rims: [{ reason: 'fill_crosses_faces' }, { reason: 'fill_rim_too_small' }] })
    expect(o.skipped).toEqual([
      'Filling this gap would cut through other faces, so it was left open.',
      'This gap is too small or thin to fill.',
    ])
  })

  it('says how many loose parts became objects of their own', () => {
    const o = parseMendOutcome({ ...full, split_vertices: 1, pieces: ['obj_7', 'obj_8'] })
    expect(o.pieces).toBe(2)
    expect(mendResultSentence([o])).toBe(
      "Merged 312 points, separated 1 spot where parts touched at a single point, separated 2 loose parts, filled 7 gaps. It's solid now.",
    )
    expect(mendResultSentence([parseMendOutcome({ pieces: ['obj_9'], closed: false })])).toBe('Separated 1 loose part.')
  })

  it('an unknown reason code still reads as a sentence', () => {
    expect(parseMendOutcome({ ...full, skipped_rims: [{ reason: 'SomethingNew { x: 1 }' }] }).skipped).toEqual(['It was left open.'])
  })

  it('several objects add up', () => {
    const a = parseMendOutcome(full)
    const b = parseMendOutcome({ ...full, welded_vertices: 8, split_vertices: 0, filled_rims: 0, closed: false })
    expect(mendResultSentence([a, b])).toBe('Merged 320 points, separated 2 spots where parts touched at a single point, filled 7 gaps.')
  })

  it('mendToast: the object sentence for one, the batch line plus counts for many', () => {
    const op: BatchOp = { object: 1n, method: 'hew.solid.mend' }
    const op2: BatchOp = { object: 2n, method: 'hew.solid.mend' }
    const one: BatchResult = { done: [op], failed: [], results: new Map([[op, full]]) }
    expect(mendToast(one)).toBe("Merged 312 points, separated 2 spots where parts touched at a single point, filled 7 gaps. It's solid now.")
    const two: BatchResult = {
      done: [op, op2],
      failed: [{ object: 3n, method: 'hew.solid.mend', explanation: 'x' }],
      results: new Map([[op, full], [op2, full]]),
    }
    expect(mendToast(two)).toBe("Mended 2 objects. 1 couldn't be mended; see the list. Merged 624 points, separated 4 spots where parts touched at a single point, filled 14 gaps.")
    expect(mendToast({ done: [], failed: [{ object: 3n, method: 'hew.solid.mend', explanation: 'x' }] })).toBe(
      "Nothing was mended. 1 couldn't be mended; see the list.",
    )
  })
})

describe('merge distance', () => {
  const range: [number, number] = [1e-9, 1e-4]

  it('formats micron values the usual length display would round to zero', () => {
    expect(formatMergeDistance(0.0000019)).toBe('0.0019 mm')
  })

  it('reads a bare number in the unit the field shows, whatever the model unit; a typed unit wins', () => {
    const before = getLengthUnit()
    try {
      for (const unit of ['cm', 'm', 'mm'] as const) {
        setLengthUnit(unit)
        expect(formatMergeDistance(0.000003)).toBe('0.003 mm')
        const bare = parseMergeDistance('0.003', range)
        expect(bare).toEqual({ ok: true, meters: expect.closeTo(0.000003, 15) })
        expect(parseMergeDistance('0.0003 cm', range)).toEqual({ ok: true, meters: expect.closeTo(0.000003, 15) })
      }
      setLengthUnit('frac_in')
      expect(parseMergeDistance('0.001', [1e-9, 1e-3])).toEqual({ ok: true, meters: expect.closeTo(0.0000254, 15) })
    } finally {
      setLengthUnit(before)
    }
  })

  it('accepts a distance in range and refuses the rest in plain words', () => {
    const ok = parseMergeDistance('0.002 mm', range)
    expect(ok).toEqual({ ok: true, meters: expect.closeTo(0.000002, 12) })
    expect(parseMergeDistance('5 mm', range)).toEqual({ ok: false, error: 'Enter a distance between 0.000001 mm and 0.1 mm.' })
    expect(parseMergeDistance('abc', range)).toEqual({ ok: false, error: 'Enter a distance, such as 0.002 mm.' })
    expect(parseMergeDistance('0', range).ok).toBe(false)
    expect(parseMergeDistance('0.002 mm', null).ok).toBe(true)
  })
})

describe('review fixes', () => {
  const wire = JSON.stringify({ weld: 0.000002, weld_vertices: 3, folds: 0, branching_vertices: 0, islands: 1, filled_rims: 0 })

  it('a census is not reused across a file load (the generation restarts, the revision moves)', () => {
    const scene = { mend_census_json: vi.fn(() => wire), history_generation: () => 0n } as unknown as WasmScene
    readMendCensus(scene, 1n, DEFAULT_MEND_PLAN, 4)
    readMendCensus(scene, 1n, DEFAULT_MEND_PLAN, 4)
    expect(scene.mend_census_json).toHaveBeenCalledTimes(1)
    readMendCensus(scene, 1n, DEFAULT_MEND_PLAN, 5)
    expect(scene.mend_census_json).toHaveBeenCalledTimes(2)
  })

  it('hover points are cached per generation', () => {
    const scene = { mend_census_points: vi.fn(() => new Float32Array([1, 2, 3])), history_generation: () => 1n } as unknown as WasmScene
    readMendPoints(scene, 1n, DEFAULT_MEND_PLAN, 0)
    readMendPoints(scene, 1n, DEFAULT_MEND_PLAN, 0)
    readMendPoints(scene, 1n, DEFAULT_MEND_PLAN, 1)
    expect(scene.mend_census_points).toHaveBeenCalledTimes(2)
  })

  it('joins hundreds of thousands of floats without spreading them', () => {
    const big = new Float32Array(300000).fill(1)
    const all = concatPoints([big, big, new Float32Array([7])])
    expect(all.length).toBe(600001)
    expect(all[600000]).toBe(7)
  })

  it('a range bound is printed so that it is accepted', () => {
    const range: [number, number] = [2.6666e-9, 9.9999e-5]
    const bad = parseMergeDistance('5 mm', range)
    expect(bad.ok).toBe(false)
    const [lo, hi] = (bad as { error: string }).error.match(/between (.+) and (.+)\.$/)!.slice(1)
    expect(parseMergeDistance(lo, range).ok).toBe(true)
    expect(parseMergeDistance(hi, range).ok).toBe(true)
  })

  it('an edge-only split still reads as a result', () => {
    const o = parseMendOutcome({ split_vertices: 0, split_edges: 4, closed: false })
    expect(mendResultSentence([o])).toBe('Separated 4 edges where parts touched.')
  })
})

describe('kernel wire changes', () => {
  it('a skipped gap with no rim still reads in plain words', () => {
    const o = parseMendOutcome({
      welded_vertices: 2,
      closed: false,
      skipped_rims: [{ object: 0, rim: null, reason: 'FillCrossesFaces' }, { object: 0, reason: 'FillRimTooSmall' }],
    })
    expect(o.skipped).toHaveLength(2)
    expect(mendResultSentence([o])).toBe("Merged 2 points. 2 gaps couldn't be filled; see the list.")
    expect(skippedLine(o.skipped)).toContain('2 gaps stayed open.')
  })

  it('the preview says "up to" for folds; the done sentence stays exact', () => {
    expect(mendPreviewLine({ ...CENSUS, weldVertices: 0, branchingVertices: 0, fillRims: 0, folds: 206 })).toBe(
      'Mend will remove up to 206 paper-thin overlaps.',
    )
    expect(mendResultSentence([parseMendOutcome({ folds_removed: 33, closed: true })])).toBe(
      "Removed 33 paper-thin overlaps. It's solid now.",
    )
  })

  it('a refused census gives its plain copy and is remembered until the generation moves', () => {
    let generation = 1n
    const scene = {
      mend_census_json: vi.fn(() => {
        throw new Error('WeldDistanceOutOfRange: 1e-3 > 1e-6')
      }),
      history_generation: () => generation,
    } as unknown as WasmScene
    const bad = readMendCensusResult(scene, 1n)
    expect(bad.census).toBeNull()
    expect(bad.error).toBe('That distance for merging points is out of range. Try a distance nearer the one first shown.')
    readMendCensusResult(scene, 1n)
    readMendCensusResult(scene, 1n)
    expect(scene.mend_census_json).toHaveBeenCalledTimes(1)
    generation = 2n
    readMendCensusResult(scene, 1n)
    expect(scene.mend_census_json).toHaveBeenCalledTimes(2)
  })
})

describe('plain wording', () => {
  it('the idle line says what is true for the ticked steps', () => {
    expect(mendIdleLine({ weld: true, folds: false, split: false, fill: false })).toBe('Nothing to merge here. Tick another step.')
    expect(mendIdleLine({ weld: true, folds: false, split: true, fill: true })).toBe('Nothing to merge, separate or fill here. Tick another step.')
    expect(mendIdleLine({ weld: false, folds: false, split: false, fill: false })).toBe('Choose at least one step.')
    // Every step is already ticked: nothing left to tick.
    expect(mendIdleLine({ weld: true, folds: true, split: true, fill: true })).toBe('Nothing to mend here.')
  })
  it('uses no pinch or fold words', () => {
    const line = mendPreviewLine({ ...CENSUS })!
    expect(line).toContain('spots where parts touch at a single point')
    expect(line).toContain('paper-thin overlaps')
    expect(line).not.toMatch(/pinch|zero-width|fold/i)
  })
})
