/**
 * profileSelection — the drawn chain the current selection stands for, as
 * loose segments with their circles, ready for `chainItemBytes`. A whole
 * free sketch, or any drawn lines, curves and edges picked on a plane. Null
 * when nothing selected is drawing.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { isDraftKind } from '../panels/treeModel'
import { planeOfCtx, resolveDraft, type PlaneUnitsScene } from '../tools/planeUnits'
import type { ChainSegment } from './profiles'

type V3 = [number, number, number]

export interface SelectedChain {
  segments: ChainSegment[]
  /** The drawing plane's normal. */
  normal: V3
}

function sketchChain(scene: WasmScene, sketch: bigint): SelectedChain | null {
  const plane = scene.sketch_plane(sketch)
  if (plane === undefined) return null
  let list: BigUint64Array
  try {
    list = scene.sketch_edge_list(sketch)
  } catch {
    return null
  }
  const segments: ChainSegment[] = []
  for (const edge of list) {
    const ends = scene.sketch_edge_endpoints(sketch, edge)
    if (ends === undefined || ends.length < 6) return null
    const curveId = scene.sketch_edge_curve(sketch, edge)
    const geom = curveId === undefined ? undefined : scene.sketch_curve_geom(sketch, curveId)
    segments.push({
      a: [ends[0], ends[1], ends[2]],
      b: [ends[3], ends[4], ends[5]],
      curve: geom === undefined ? null : { center: [geom[0], geom[1], geom[2]], radius: geom[3] },
    })
  }
  return segments.length === 0 ? null : { segments, normal: [plane[3], plane[4], plane[5]] }
}

/**
 * The chain the selection draws: all of one sketch's edges, or the edges of
 * the selected drawn units (all on one plane). Never throws.
 */
export function selectionChain(scene: WasmScene, selected: readonly NodeRef[]): SelectedChain | null {
  const sketches = selected.filter((n) => n.kind === 'sketch')
  if (sketches.length === 1 && selected.length === 1) return sketchChain(scene, sketches[0].id)
  const drafts = selected.filter((n) => isDraftKind(n.kind) && n.ctx !== undefined)
  if (drafts.length === 0 || drafts.length !== selected.length) return null
  const ctx = drafts[0].ctx as string
  if (drafts.some((d) => d.ctx !== ctx)) return null
  const plane = planeOfCtx(ctx)
  if (plane === null) return null
  const seen = new Set<string>()
  const segments: ChainSegment[] = []
  for (const ref of drafts) {
    const resolved = resolveDraft(scene as unknown as PlaneUnitsScene, ref)
    if (resolved === null) return null
    for (const e of resolved.edges) {
      if (seen.has(e.key)) continue
      seen.add(e.key)
      segments.push({
        a: [e.a[0], e.a[1], e.a[2]],
        b: [e.b[0], e.b[1], e.b[2]],
        curve:
          e.curve !== null && e.curve.radius !== null
            ? { center: [e.curve.center[0], e.curve.center[1], e.curve.center[2]], radius: e.curve.radius }
            : null,
      })
    }
  }
  return segments.length === 0 ? null : { segments, normal: plane.normal }
}
