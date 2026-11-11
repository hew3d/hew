/**
 * Mend through the REAL compiled wasm module and the Model Health batch
 * path: a box imported as a soup whose corners miss each other by a hair is
 * listed under "Needs mending", previews what Mend will do, mends in one
 * undo step, and ends up solid. Bad input comes back as typed refusals in
 * the app's plain words.
 */
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { readHealthReportFresh, classifyRow } from './healthReport'
import { runBatch } from './healthBatch'
import { DEFAULT_MEND_PLAN, mendOp, mendOutcomes, mendPreviewLine, mendToast, readMendCensus, readWeldRange } from './mend'
import { BANNED_RE } from './bannedWords'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

type V = [number, number, number]

/** A 100 mm box as an STL soup: every face keeps its own corners, each
 *  moved by `jitter` mm so no two corners coincide exactly. */
function boxStl(jitter: number): Uint8Array {
  const c: V[] = [
    [0, 0, 0], [100, 0, 0], [100, 100, 0], [0, 100, 0],
    [0, 0, 100], [100, 0, 100], [100, 100, 100], [0, 100, 100],
  ]
  const quads = [
    [0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7],
  ]
  let n = 0
  const wiggle = (p: V): V => {
    n += 1
    return [p[0] + jitter * Math.sin(n * 12.9), p[1] + jitter * Math.sin(n * 78.2), p[2] + jitter * Math.sin(n * 37.7)]
  }
  const lines = ['solid soup']
  for (const q of quads) {
    for (const tri of [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]) {
      lines.push('facet normal 0 0 0', 'outer loop')
      for (const i of tri) lines.push(`vertex ${wiggle(c[i]).map((x) => x.toFixed(7)).join(' ')}`)
      lines.push('endloop', 'endfacet')
    }
  }
  lines.push('endsolid soup')
  return new TextEncoder().encode(lines.join('\n'))
}

function importSoup(scene: Scene, jitter: number): bigint {
  scene.import_stl(boxStl(jitter), 0.001, 'soup')
  const ids = Array.from(scene.last_import_object_ids())
  expect(ids.length).toBeGreaterThan(0)
  return ids[0]
}

describe('Mend through the real kernel', () => {
  it('previews, mends in one undo step, and ends solid', () => {
    const scene = new Scene()
    const id = importSoup(scene, 0.00005)
    const asWasm = scene as unknown as WasmScene

    const report = readHealthReportFresh(asWasm, null)
    const row = report.rows.find((r) => r.object === id)
    expect(row).toBeDefined()
    expect(row!.verdict.kind).not.toBe('solid')

    const census = readMendCensus(asWasm, id)
    expect(census).not.toBeNull()
    const line = mendPreviewLine(census!)
    expect(line).toMatch(/^Mend will .*(nearby point|spots? where|small gap)/)
    expect(line).not.toMatch(BANNED_RE)
    const range = readWeldRange(asWasm, id)
    expect(range).not.toBeNull()
    expect(range![1]).toBeGreaterThan(range![0])
    expect(scene.mend_census_points(id, JSON.stringify({ weld: true }), 0).length).toBeGreaterThan(0)

    const depth = scene.undo_depth()
    const result = runBatch(asWasm, (n) => `Mend ${n}`, [mendOp(row!, DEFAULT_MEND_PLAN)])
    expect(result.failed).toEqual([])
    expect(result.done).toHaveLength(1)
    const outcome = [...mendOutcomes(result).values()][0]
    expect(outcome.closed).toBe(true)
    expect(outcome.welded).toBeGreaterThan(0)
    expect(mendToast(result)).toMatch(/^(Merged|Separated|Filled).*It.s solid now\.$/)

    // One undo step, and the object is solid.
    expect(scene.undo_depth()).toBe(depth + 1)
    const after = readHealthReportFresh(asWasm, [id])
    expect(after.rows.filter((r) => r.object === id)).toEqual([])
    scene.scene_undo()
    expect(scene.undo_depth()).toBe(depth)
    const back = readHealthReportFresh(asWasm, [id]).rows.find((r) => r.object === id)
    expect(back).toBeDefined()
    expect(classifyRow(back!)).toBe(classifyRow(row!))
  })

  it('a second Mend refuses with the plain "nothing to mend" copy', () => {
    const scene = new Scene()
    const id = importSoup(scene, 0.00005)
    const asWasm = scene as unknown as WasmScene
    const row = readHealthReportFresh(asWasm, null).rows.find((r) => r.object === id)!
    runBatch(asWasm, (n) => `Mend ${n}`, [mendOp(row)])
    const again = runBatch(asWasm, (n) => `Mend ${n}`, [mendOp(row)])
    expect(again.done).toEqual([])
    expect(again.failed).toHaveLength(1)
    expect(again.failed[0].explanation).toBe('Nothing to mend here: no nearby points, touching parts or small gaps.')
  })

  it('a merge distance out of range refuses in plain words', () => {
    const scene = new Scene()
    const id = importSoup(scene, 0.00005)
    const asWasm = scene as unknown as WasmScene
    const row = readHealthReportFresh(asWasm, null).rows.find((r) => r.object === id)!
    const result = runBatch(asWasm, (n) => `Mend ${n}`, [mendOp(row, { ...DEFAULT_MEND_PLAN, weld: 10 })])
    expect(result.failed).toHaveLength(1)
    expect(result.failed[0].explanation).toBe('That distance for merging points is out of range. Try a distance nearer the one first shown.')
  })
})
