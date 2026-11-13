/**
 * healthBatch — frame building (labels, commands, public ids), the
 * refusal-retry loop, and the plans behind the header buttons.
 */
import { i18n } from '@lingui/core'
import { describe, expect, it, vi } from 'vitest'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  STALE_OBJECT_EXPLANATION,
  batchLabel,
  batchSummary,
  capOp,
  closableRows,
  deleteFacesOp,
  failureLine,
  familyCascade,
  familyRejoinSolidOps,
  familyRepairOps,
  groupBatchButtons,
  refusalReason,
  rejoinSolidOp,
  markOp,
  markableRows,
  runBatch,
  thickenOp,
} from './healthBatch'
import { parseHealthReport, type HealthRow } from './healthReport'
import { taggedDispatch } from '../test/taggedDispatch'

function rows(...specs: Array<[string, Record<string, unknown>?]>): HealthRow[] {
  return parseHealthReport(
    JSON.stringify({
      rows: specs.map(([object, over]) => ({ object, verdict: { kind: 'cappable' }, rims: [], ...over })),
      families: [],
      counts: {},
    }),
  ).rows
}

interface Frame {
  method: string
  params: { label?: string; commands?: Array<{ method: string; params: Record<string, unknown> }> }
}

/** A fake Scene: ids are `obj_<handle>`, `refuse` decides each transact. */
function fakeScene(refuse: (frame: Frame, call: number) => { index: number; explanation: string } | null = () => null, stale: bigint[] = []) {
  const frames: Frame[] = []
  let transacts = 0
  const scene = {
    api_connection_open: vi.fn(() => 7),
    api_connection_close: vi.fn(),
    object_public_ids: vi.fn((ids: BigUint64Array) => Array.from(ids, (id) => (stale.includes(id) ? '' : `obj_${id}`))),
    api_dispatch: vi.fn(taggedDispatch((_conn: number, raw: string) => {
      const req = JSON.parse(raw) as { id: number } & Frame
      frames.push(req)
      if (req.method !== 'hew.doc.transact') return JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} })
      const refusal = refuse(req, transacts++)
      if (refusal === null) return JSON.stringify({ jsonrpc: '2.0', id: req.id, result: { ok: true } })
      return JSON.stringify({
        jsonrpc: '2.0',
        id: req.id,
        error: {
          code: -32000,
          message: 'refused',
          data: { refusal: 'X', failed_index: refusal.index, failed_method: 'm', detail: {}, explanation: refusal.explanation },
        },
      })
    })),
  }
  return { scene: scene as unknown as WasmScene & typeof scene, frames }
}

describe('runBatch — one transaction', () => {
  it('opens a connection, says hello, attaches, sends ONE labeled transact with public ids, closes', () => {
    const { scene, frames } = fakeScene()
    const ops = [capOp(rows(['1'])[0]), capOp(rows(['2'])[0])]
    const result = runBatch(scene, (n) => batchLabel('close', n), ops)
    expect(frames.map((f) => f.method)).toEqual(['hew.meta.hello', 'hew.doc.attach', 'hew.doc.transact'])
    const transact = frames[2]
    expect(transact.params.label).toBe('Close 2 objects')
    expect(transact.params.commands).toEqual([
      { method: 'hew.solid.cap', params: { object: 'obj_1' } },
      { method: 'hew.solid.cap', params: { object: 'obj_2' } },
    ])
    expect(scene.object_public_ids).toHaveBeenCalledWith(BigUint64Array.from([1n, 2n]))
    expect(result.done).toHaveLength(2)
    expect(result.failed).toEqual([])
    expect(scene.api_connection_close).toHaveBeenCalledWith(7)
  })

  it('builds the thicken / mark / delete commands with their params', () => {
    const { scene, frames } = fakeScene()
    const [a, b, c, d] = rows(['1'], ['2'], ['3'], ['4'])
    runBatch(scene, () => 'L', [thickenOp(a, 0.02, false), thickenOp(b, -0.02, false), thickenOp(c, 0.02, true), markOp(d), deleteFacesOp(d)])
    expect(frames[2].params.commands).toEqual([
      { method: 'hew.solid.thicken', params: { object: 'obj_1', distance: 0.02 } },
      { method: 'hew.solid.thicken', params: { object: 'obj_2', distance: -0.02 } },
      { method: 'hew.solid.thicken', params: { object: 'obj_3', distance: 0.02, centered: true } },
      { method: 'hew.solid.mark_surface', params: { object: 'obj_4', accepted: true } },
      { method: 'hew.solid.delete_faces', params: { object: 'obj_4' } },
    ])
  })

  it('an empty batch sends nothing and opens no connection', () => {
    const { scene } = fakeScene()
    expect(runBatch(scene, () => 'L', [])).toEqual({ done: [], failed: [] })
    expect(scene.api_connection_open).not.toHaveBeenCalled()
  })
})

describe('runBatch — refusals are never skipped silently', () => {
  it('refused at index 1: that row is dropped with its explanation, the rest re-runs and succeeds', () => {
    const { scene, frames } = fakeScene((_f, call) => (call === 0 ? { index: 1, explanation: 'rim crosses itself' } : null))
    const ops = rows(['1'], ['2'], ['3']).map(capOp)
    const result = runBatch(scene, (n) => batchLabel('close', n), ops)
    const transacts = frames.filter((f) => f.method === 'hew.doc.transact')
    expect(transacts).toHaveLength(2)
    expect(transacts[0].params.commands).toHaveLength(3)
    expect(transacts[1].params.label).toBe('Close 2 objects') // labeled for what finally ran
    expect(transacts[1].params.commands!.map((c) => c.params.object)).toEqual(['obj_1', 'obj_3'])
    expect(result.done.map((o) => o.object)).toEqual([1n, 3n])
    expect(result.failed).toEqual([{ object: 2n, method: 'hew.solid.cap', explanation: 'rim crosses itself' }])
  })

  it('keeps going until the batch is empty when every row refuses', () => {
    const { scene } = fakeScene((_f, call) => ({ index: 0, explanation: `no ${call}` }))
    const result = runBatch(scene, () => 'L', rows(['1'], ['2']).map(capOp))
    expect(result.done).toEqual([])
    expect(result.failed.map((f) => [f.object, f.explanation])).toEqual([[1n, 'no 0'], [2n, 'no 1']])
  })

  it('a stale handle fails honestly and never reaches the frame', () => {
    const { scene, frames } = fakeScene(() => null, [2n])
    const result = runBatch(scene, () => 'L', rows(['1'], ['2']).map(capOp))
    expect(frames[2].params.commands).toHaveLength(1)
    expect(result.failed).toEqual([{ object: 2n, method: 'hew.solid.cap', explanation: i18n._(STALE_OBJECT_EXPLANATION) }])
  })

  it('an error that is not a per-command refusal throws', () => {
    const scene = {
      api_connection_open: () => 1,
      api_connection_close: vi.fn(),
      object_public_ids: () => ['obj_1'],
      api_dispatch: taggedDispatch((_c: number, raw: string) =>
        JSON.stringify({ jsonrpc: '2.0', id: 1, ...(raw.includes('transact') ? { error: { code: -32004, message: 'not ready' } } : { result: {} }) }),
      ),
    } as unknown as WasmScene
    expect(() => runBatch(scene, () => 'L', rows(['1']).map(capOp))).toThrow('not ready')
    expect(scene.api_connection_close).toHaveBeenCalled()
  })

  it('cascade drops dependent ops with their own explanation', () => {
    const { scene, frames } = fakeScene((_f, call) => (call === 0 ? { index: 0, explanation: 'bad rim' } : null))
    const [cap, sheet] = rows(['1'], ['2', { verdict: { kind: 'flat_sheet' } }])
    const result = runBatch(scene, () => 'L', [capOp(cap), deleteFacesOp(sheet)], { cascade: familyCascade })
    expect(frames.filter((f) => f.method === 'hew.doc.transact')).toHaveLength(1)
    expect(result.done).toEqual([])
    expect(result.failed.map((f) => f.object)).toEqual([1n, 2n])
    expect(result.failed[1].explanation).toBe("Left as is, because another piece of this shape couldn't be closed.")
  })
})

describe('plans and words', () => {
  it('labels and summaries use the plain words', () => {
    expect(batchLabel('close', 1)).toBe('Close 1 object')
    expect(batchLabel('close', 12)).toBe('Close 12 objects')
    expect(batchLabel('mark', 30)).toBe('Keep 30 objects as is')
    expect(batchLabel('thicken', 8)).toBe('Add thickness to 8 panels')
    expect(batchLabel('repair', 3)).toBe('Rejoin 3 split shapes')
    expect(batchLabel('mend', 1)).toBe('Mend 1 object')
    expect(batchLabel('mend', 4)).toBe('Mend 4 objects')
    const ok = (n: number) => Array.from({ length: n }, () => capOp(rows(['1'])[0]))
    const fail = (n: number) => Array.from({ length: n }, () => ({ object: 1n, method: 'm', explanation: 'x' }))
    expect(batchSummary('close', { done: ok(11), failed: fail(1) })).toBe("Closed 11 objects. 1 couldn't be closed; see the list.")
    expect(batchSummary('close', { done: ok(2), failed: [] })).toBe('Closed 2 objects.')
    expect(batchSummary('close', { done: [], failed: fail(2) })).toBe("Nothing was closed. 2 couldn't be closed; see the list.")
    expect(batchSummary('mark', { done: ok(3), failed: [] })).toBe('Kept 3 objects.')
    expect(failureLine('close', 'no good')).toBe("Couldn't close: no good")
    expect(failureLine('mend', 'no good')).toBe("Couldn't mend: no good")
    expect(batchSummary('mend', { done: ok(3), failed: fail(1) })).toBe("Mended 3 objects. 1 couldn't be mended; see the list.")
    expect(failureLine('thicken', 'no good')).toBe("Couldn't add thickness: no good")
    expect(failureLine('mark', 'no good')).toBe("Couldn't keep: no good")
  })

  it('which buttons each kind of group carries', () => {
    expect(groupBatchButtons('openings')).toEqual({ close: true, mark: false, thicken: null })
    expect(groupBatchButtons('flatPanels')).toEqual({ close: false, mark: true, thicken: 'enabled' })
    expect(groupBatchButtons('curvedSheets')).toEqual({ close: false, mark: true, thicken: 'disabled' })
    expect(groupBatchButtons('closerLook')).toEqual({ close: false, mark: true, thicken: null })
    expect(groupBatchButtons('split')).toEqual({ close: false, mark: false, thicken: null })
  })

  it('a refusal shows the app copy for its code, else the API explanation', () => {
    expect(refusalReason({ refusal: 'cap_would_be_coplanar', explanation: 'raw' }, 'x')).toMatch(/flat panel/)
    expect(refusalReason({ refusal: 'not_a_real_code', explanation: 'raw words' }, 'x')).toBe('raw words')
    expect(refusalReason(undefined, 'fallback')).toBe('fallback')
  })

  it('Close all takes rows with a flat opening; Mark all skips already-accepted rows', () => {
    const rs = rows(
      ['1'],
      ['2', { verdict: { kind: 'holes', planar: 0, warped: 2 } }],
      ['3', { verdict: { kind: 'holes', planar: 1, warped: 0 } }],
      ['4', { accepted: true }],
    )
    expect(closableRows(rs).map((r) => r.object)).toEqual([1n, 3n, 4n])
    expect(markableRows(rs).map((r) => r.object)).toEqual([1n, 2n, 3n])
  })

  it('the split-shape repair caps every piece (any member that is not a flat sheet) and deletes (or marks) flat sheets', () => {
    const fam = rows(
      ['1'],
      ['2'],
      ['3', { verdict: { kind: 'flat_sheet' } }],
      ['4', { verdict: { kind: 'holes', planar: 1, warped: 0 } }],
    )
    expect(familyRepairOps(fam, false).map((o) => [o.method, o.object])).toEqual([
      ['hew.solid.cap', 1n],
      ['hew.solid.cap', 2n],
      ['hew.solid.cap', 4n],
      ['hew.solid.delete_faces', 3n],
    ])
    expect(familyRepairOps(fam, true).map((o) => o.method)).toEqual([
      'hew.solid.cap',
      'hew.solid.cap',
      'hew.solid.cap',
      'hew.solid.mark_surface',
    ])
  })
})

describe('runBatch — rejoin as one solid', () => {
  const family = rows(['1', { family: 4 }], ['2', { family: 4 }], ['3', { family: 4 }])

  it('the rejoin frame is exactly { family: <index>, as } with no object and no member ids', () => {
    const { scene, frames } = fakeScene()
    runBatch(scene, () => 'L', [rejoinSolidOp(family)])
    expect(frames[2].params.commands).toEqual([{ method: 'hew.solid.rejoin', params: { family: 4, as: 'one_solid' } }])
  })

  it('the whole wizard run with rejoin ops is one transaction', () => {
    const { scene, frames } = fakeScene()
    const [a, b] = rows(['7'], ['8'])
    const result = runBatch(scene, () => 'Make Solid', [rejoinSolidOp(family), capOp(a), thickenOp(b, 0.02, false)])
    const transacts = frames.filter((f) => f.method === 'hew.doc.transact')
    expect(transacts).toHaveLength(1)
    expect(transacts[0].params.commands!.map((c) => c.method)).toEqual(['hew.solid.rejoin', 'hew.solid.cap', 'hew.solid.thicken'])
    expect(result.done).toHaveLength(3)
  })

  it('a refused family is reported and left untouched, the rest re-runs without it', () => {
    const { scene, frames } = fakeScene((_f, call) => (call === 0 ? { index: 0, explanation: 'has an opening' } : null))
    const [a] = rows(['7'])
    const result = runBatch(scene, () => 'L', [rejoinSolidOp(family), capOp(a)])
    const transacts = frames.filter((f) => f.method === 'hew.doc.transact')
    expect(transacts).toHaveLength(2)
    expect(transacts[1].params.commands!.map((c) => c.method)).toEqual(['hew.solid.cap'])
    expect(result.failed).toEqual([{ object: 1n, method: 'hew.solid.rejoin', explanation: 'has an opening' }])
  })

  it('a family with a member that is gone fails as stale and never reaches the frame', () => {
    const { scene, frames } = fakeScene(() => null, [3n])
    const result = runBatch(scene, () => 'L', [rejoinSolidOp(family)])
    expect(frames.some((f) => f.method === 'hew.doc.transact')).toBe(false)
    expect(result.failed).toEqual([{ object: 1n, method: 'hew.solid.rejoin', explanation: i18n._(STALE_OBJECT_EXPLANATION) }])
  })

  it('a rejoin-as-one-solid batch has its own failure and toast wording', () => {
    expect(failureLine('rejoinSolid', 'has an opening')).toBe("Couldn't rejoin: has an opening")
    expect(batchSummary('rejoinSolid', { done: [], failed: [{ object: 1n, method: 'hew.solid.rejoin', explanation: 'x' }] })).toBe(
      "Nothing was rejoined. 1 couldn't be rejoined; see the list.",
    )
    expect(batchLabel('rejoinSolid', 1)).toBe('Rejoin as one solid')
  })

  it('the panel button needs two pieces: one piece plus a wall offers no rejoin', () => {
    const wall = { verdict: { kind: 'flat_sheet', holes: 0 }, family: 4 }
    expect(familyRejoinSolidOps(rows(['1', { family: 4 }], ['2', wall]))).toEqual([])
    expect(familyRejoinSolidOps(rows(['1', { family: 4 }], ['2', { family: 4 }], ['3', wall]))).toHaveLength(1)
  })
})
