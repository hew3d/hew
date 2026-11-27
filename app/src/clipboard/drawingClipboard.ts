/**
 * drawingClipboard — the drawing half of Copy/Cut/Paste (v1.5 decisions D3,
 * KB-46).
 *
 * Copy names the selected drawing to `Scene.extract_clipboard` at the
 * granularity Move uses: each selected shape (island) rides with its
 * plane's listing keys, a whole sketch by its id, a detached shape by its
 * sketch's islands. Drawing that rides with a selected owner (an object's
 * face drawing) is left to the owner's own copy, exactly as Move leaves it.
 *
 * Paste is a draw at the destination (the kernel's `DrawingInsert::Draw`);
 * `insert_item` hands back, per pasted plane, every pasted segment, and the
 * pasted shapes are found again by geometry in the fresh listing, the way
 * Move+copy hands its copies back.
 */

import { plural, t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { planDraftTransforms } from '../tools/transformSelection'
import {
  ctxOfPlane,
  ctxOfSketchOnly,
  draftRef,
  isDraftRef,
  readPlaneUnits,
  sketchOfCtx,
  unitsAtSegments,
} from '../tools/planeUnits'
import type { DrawnInsert } from '../tools/LibraryPlaceTool'
import type { V3 } from '../viewport/geoHelpers'

/** Whether `n` is drawing a clipboard copy can take: a drawn shape or a
 *  whole sketch. */
export function isCopyableDrawing(n: NodeRef): boolean {
  return isDraftRef(n) || n.kind === 'sketch'
}

/** One entry of `extract_clipboard`'s drawing JSON. */
type DrawingEntry =
  | { sketch: number; islands?: number[] }
  | { ctx: string; keys: string[] }

/** The drawing JSON `extract_clipboard` takes for `selection` (structural
 *  owners included, so drawing that rides with them is left to them), and
 *  how many shapes it names. `shapes` is 0 when nothing is drawing. */
export function clipboardDrawing(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
): { json: string; shapes: number } {
  const entries: DrawingEntry[] = []
  let shapes = 0
  for (const n of selection) {
    if (n.kind !== 'sketch') continue
    entries.push({ sketch: Number(n.id) })
    shapes += 1
  }
  if (selection.some(isDraftRef)) {
    for (const g of planDraftTransforms(wasmScene, withRegionHoles(wasmScene, selection))) {
      const of = sketchOfCtx(g.ctx)
      const detached =
        of !== null &&
        of.instance === null &&
        g.edges.length > 0 &&
        g.edges.every((e) => e.store.kind === 'free' && e.store.detached === true && e.store.sketch === of.sketch)
      if (detached && of !== null) {
        // A detached shape is named by its sketch's islands (its own
        // context lists it apart from every plane's arrangement).
        const keys = JSON.parse(wasmScene.sketch_unit_keys(of.sketch)) as { islands: [number, string][] }
        const islands = keys.islands.filter(([, key]) => g.unitKeys.includes(key)).map(([id]) => id)
        if (islands.length === 0) continue
        entries.push({ sketch: Number(of.sketch), islands })
      } else {
        entries.push({ ctx: g.ctx, keys: g.edges.map((e) => e.key) })
      }
      shapes += g.unitKeys.length
    }
  }
  return { json: JSON.stringify(entries), shapes }
}

/** What a Cut deletes: exactly what the Copy carried. A copy takes a drawn
 *  edge, curve or region as its WHOLE shape (island), so the delete names
 *  the whole shape too — deleting only the picked edge would leave a damaged
 *  shape behind a clipboard holding the entire one. Structural nodes and
 *  whole sketches pass through unchanged; hole shapes of a copied region go
 *  with it. */
export function cutTargets(wasmScene: WasmScene, copied: readonly NodeRef[]): NodeRef[] {
  const out = copied.filter((n) => !isDraftRef(n))
  const seen = new Set<string>()
  if (copied.some(isDraftRef)) {
    for (const g of planDraftTransforms(wasmScene, withRegionHoles(wasmScene, copied))) {
      for (const key of g.unitKeys) {
        if (seen.has(`${g.ctx}:${key}`)) continue
        seen.add(`${g.ctx}:${key}`)
        out.push(draftRef('draft-island', g.ctx, key))
      }
    }
  }
  return out
}

/** `selection` with, for every selected region, the shapes (islands) that
 *  bound its holes: a hole is its own island, and a copied ring must stay a
 *  ring. */
function withRegionHoles(wasmScene: WasmScene, selection: readonly NodeRef[]): NodeRef[] {
  const out = [...selection]
  for (const n of selection) {
    if (n.kind !== 'draft-region' || !isDraftRef(n)) continue
    const pu = readPlaneUnits(wasmScene, n.ctx)
    const region = pu?.regions.find((r) => r.key === n.key)
    if (pu === null || pu === undefined || region === undefined) continue
    for (const hole of region.holes) {
      const segs: [V3, V3][] = hole.map((p, i) => [p, hole[(i + 1) % hole.length]])
      for (const u of unitsAtSegments(pu, segs)) out.push(draftRef('draft-island', n.ctx, u.key))
    }
  }
  return out
}

/** The shapes a paste drew, found again by geometry in the fresh listings:
 *  each plane's units holding a pasted segment, and every unit of a new
 *  detached sketch. */
export function pastedDrawingRefs(wasmScene: WasmScene, drawn: readonly DrawnInsert[]): NodeRef[] {
  const out: NodeRef[] = []
  const seen = new Set<string>()
  for (const d of drawn) {
    const ctx =
      d.detached !== null
        ? ctxOfSketchOnly(d.detached)
        : ctxOfPlane([d.plane[0], d.plane[1], d.plane[2]], [d.plane[3], d.plane[4], d.plane[5]])
    if (ctx === null) continue
    const pu = readPlaneUnits(wasmScene, ctx)
    if (pu === null) continue
    const units = d.detached !== null ? pu.units : unitsAtSegments(pu, d.anchors)
    for (const u of units) {
      const k = `${ctx}:${u.key}`
      if (seen.has(k)) continue
      seen.add(k)
      out.push(draftRef('draft-island', ctx, u.key))
    }
  }
  return out
}

/** The toast after a Copy: objects only, drawing only, or both. */
export function copiedToast(objects: number, shapes: number): string {
  if (shapes === 0) return plural(objects, { one: 'Copied # object', other: 'Copied # objects' })
  if (objects === 0) return plural(shapes, { one: 'Copied # shape', other: 'Copied # shapes' })
  return plural(objects + shapes, { one: 'Copied # item', other: 'Copied # items' })
}

/** The toast for a selection with nothing a Copy can take. */
export function nothingToCopyToast(): string {
  return t`Nothing to copy`
}

/** The toast for a paste that drew nothing new: the pasted drawing lies
 *  exactly over what is drawn (Paste In Place right after Copy). */
export function alreadyHereToast(): string {
  return t`Already here: the pasted drawing matches what is drawn`
}
