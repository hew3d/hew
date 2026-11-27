/**
 * Copy and paste of drawing (v1.5 decisions D3, KB-46) against the REAL
 * compiled wasm kernel: the selection's drawing names itself to
 * `extract_clipboard`, a paste draws it at the destination, and the pasted
 * shapes are found again from `insert_item`'s `drawn` report. Needs a
 * rebuilt `app/src/wasm/pkg`.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { ctxGround, ctxOfPlane, ctxOfSketchOnly, draftRef, keysToArray, readPlaneUnits } from '../tools/planeUnits'
import { draftEdgesToErase, inHistoryBracket } from '../tools/transformSelection'
import { runDraft } from '../tools/draftCommit'
import { parseInsertResult } from '../tools/LibraryPlaceTool'
import { clipboardDrawing, copiedToast, cutTargets, isCopyableDrawing, pastedDrawingRefs } from './drawingClipboard'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

const NONCE = 'window-test'

function rect(scene: Scene, ctx: string, x0: number, y0: number, x1: number, y1: number, detached = false): void {
  const d = detached ? scene.draft_begin_detached(ctx, 'Rectangle') : scene.draft_begin_ctx(ctx, 'Rectangle')
  const c: [number, number][] = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
  for (let i = 0; i < 4; i++) {
    const [a, b] = [c[i], c[(i + 1) % 4]]
    scene.draft_add_segment(d, a[0], a[1], 0, b[0], b[1], 0)
  }
  scene.draft_end(d)
}

function groundShapes(scene: Scene) {
  const ctx = ctxGround()
  return (readPlaneUnits(scene as unknown as WasmScene, ctx)?.units ?? []).map((u) => draftRef('draft-island', ctx, u.key))
}

describe('drawing on the clipboard', () => {
  it('copies a drawn rectangle and pastes it beside the original, selected', () => {
    const scene = new Scene()
    rect(scene, ctxGround(), 1, 1, 2, 2)
    const shapes = groundShapes(scene)
    expect(shapes).toHaveLength(1)
    expect(isCopyableDrawing(shapes[0])).toBe(true)

    const named = clipboardDrawing(scene as unknown as WasmScene, shapes)
    expect(named.shapes).toBe(1)
    const none = { kinds: new Uint8Array(0), ids: new BigUint64Array(0) }
    const bytes = scene.extract_clipboard(none.kinds, none.ids, named.json, NONCE)
    const placement = scene.extract_clipboard_placement(none.kinds, none.ids, named.json)
    expect(placement).toBeDefined()
    const affine = Float64Array.from(placement as ArrayLike<number>)
    affine[3] += 3

    const result = parseInsertResult(scene.insert_item(bytes, affine, undefined, undefined, NONCE))
    expect(result.rootIds).toHaveLength(0)
    expect(result.drawn).toHaveLength(1)
    expect(result.drawingUnchanged).toBe(false)
    const refs = pastedDrawingRefs(scene as unknown as WasmScene, result.drawn)
    expect(refs).toHaveLength(1)
    expect(refs[0].key).not.toBe(shapes[0].key)
    expect(groundShapes(scene)).toHaveLength(2)
  })

  it('reports a Paste In Place over the original as unchanged', () => {
    const scene = new Scene()
    rect(scene, ctxGround(), 1, 1, 2, 2)
    const named = clipboardDrawing(scene as unknown as WasmScene, groundShapes(scene))
    const none = { kinds: new Uint8Array(0), ids: new BigUint64Array(0) }
    const bytes = scene.extract_clipboard(none.kinds, none.ids, named.json, NONCE)
    const placement = scene.extract_clipboard_placement(none.kinds, none.ids, named.json)
    const affine = Float64Array.from(placement as ArrayLike<number>)
    const result = parseInsertResult(scene.insert_item(bytes, affine, undefined, undefined, NONCE))
    expect(result.drawingUnchanged).toBe(true)
    expect(groundShapes(scene)).toHaveLength(1)
  })

  it('names a detached shape by its islands and pastes a new detached sketch', () => {
    const scene = new Scene()
    rect(scene, ctxGround(), 0, 0, 1, 1, true)
    const all = JSON.parse(scene.plane_units_all_json()) as { sketch?: number }[]
    const sketch = all.find((e) => typeof e.sketch === 'number')?.sketch
    expect(sketch).toBeDefined()
    const ctx = ctxOfSketchOnly(sketch ?? 0)
    const unit = readPlaneUnits(scene as unknown as WasmScene, ctx)?.units[0]
    expect(unit).toBeDefined()
    const ref = draftRef('draft-island', ctx, unit?.key ?? '')
    const named = clipboardDrawing(scene as unknown as WasmScene, [ref])
    expect(JSON.parse(named.json)).toEqual([{ sketch, islands: expect.any(Array) }])
    const none = { kinds: new Uint8Array(0), ids: new BigUint64Array(0) }
    const bytes = scene.extract_clipboard(none.kinds, none.ids, named.json, NONCE)
    const affine = Float64Array.from(scene.extract_clipboard_placement(none.kinds, none.ids, named.json) as ArrayLike<number>)
    affine[3] += 4
    const result = parseInsertResult(scene.insert_item(bytes, affine, undefined, undefined, NONCE))
    expect(result.drawn).toHaveLength(1)
    expect(result.drawn[0].detached).not.toBeNull()
    expect(pastedDrawingRefs(scene as unknown as WasmScene, result.drawn)).toHaveLength(1)
  })
})

describe('Cut deletes exactly what Copy carried', () => {
  it('a picked edge cuts its whole shape, not just the edge', () => {
    const scene = new Scene()
    const w = scene as unknown as WasmScene
    rect(scene, ctxGround(), 1, 1, 2, 2)
    const ctx = ctxGround()
    const pu = readPlaneUnits(w, ctx)
    const edge = pu?.units[0].edges[0]
    expect(edge).toBeDefined()
    const picked = draftRef('draft-edge', ctx, edge?.key ?? '')
    // The copy takes the whole shape (1 shape) ...
    expect(clipboardDrawing(w, [picked]).shapes).toBe(1)
    // ... so the cut erases every edge of it.
    const targets = cutTargets(w, [picked])
    const { draftEdgeKeys } = draftEdgesToErase(w, targets, new Set())
    expect([...(draftEdgeKeys.get(ctx) ?? [])].sort()).toEqual((pu?.units[0].edges ?? []).map((e) => e.key).sort())
  })

  it('a delete across plane contexts inside one bracket is one Undo', () => {
    const scene = new Scene()
    const w = scene as unknown as WasmScene
    const up = ctxOfPlane([0, 0, 1], [0, 0, 1]) as string
    rect(scene, ctxGround(), 1, 1, 2, 2)
    const d = scene.draft_begin_ctx(up, 'Rectangle')
    const c: [number, number][] = [[0, 0], [1, 0], [1, 1], [0, 1]]
    for (let i = 0; i < 4; i++) {
      const [a, b] = [c[i], c[(i + 1) % 4]]
      scene.draft_add_segment(d, a[0], a[1], 1, b[0], b[1], 1)
    }
    scene.draft_end(d)
    const count = () => [ctxGround(), up].reduce((n, x) => n + (readPlaneUnits(w, x)?.units.length ?? 0), 0)
    expect(count()).toBe(2)
    inHistoryBracket(w, 'Delete', () => {
      for (const x of [ctxGround(), up]) {
        const units = readPlaneUnits(w, x)?.units ?? []
        runDraft(w, { kind: 'ctx', ctx: x }, 'Delete', (dd) => {
          scene.draft_remove_edges(dd, keysToArray(units.flatMap((u) => u.edges.map((e) => e.key))))
        })
      }
    })
    expect(count()).toBe(0)
    scene.scene_undo()
    expect(count()).toBe(2)
  })
})

describe('copiedToast', () => {
  it('words objects, shapes, and a mix', () => {
    expect(copiedToast(2, 0)).toBe('Copied 2 objects')
    expect(copiedToast(0, 1)).toBe('Copied 1 shape')
    expect(copiedToast(1, 2)).toBe('Copied 3 items')
  })
})
