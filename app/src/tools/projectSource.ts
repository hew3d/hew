/**
 * projectSource — what the Project tool projects: a free sketch and, when
 * the selection names only some of its shapes, the regions to use.
 *
 * Selecting a whole sketch projects every closed shape in it (the wasm call
 * takes an empty region list for that). A drawn shape picked in the drawing
 * (an island, a curve, an edge or a region of a plane's listing) resolves to
 * the closed region its outline bounds, and only those regions are sent.
 * A shape drawn on a solid's face is part of that face, not a free profile,
 * so it never resolves here.
 */

import type { NodeRef } from '../panels/treeModel'
import {
  edgesOfCurveRef,
  isDraftRef,
  readPlaneUnits,
  type PlaneUnits,
  type PlaneUnitsScene,
} from './planeUnits'
import { regionOfEdge } from './regionPick'

export interface ProjectSource {
  sketch: bigint
  /** The region handles to project; empty means every closed shape. */
  regions: bigint[]
}

/** Region handles (of free profiles) the draft ref `n` bounds. */
function regionsOfRef(pu: PlaneUnits, n: NodeRef & { key: string }): { sketch: bigint; region: bigint }[] {
  const out: { sketch: bigint; region: bigint }[] = []
  const add = (key: string): void => {
    const r = regionOfEdge(pu, key)
    if (r !== null && r.extrude !== null) out.push(r.extrude)
  }
  switch (n.kind) {
    case 'draft-region': {
      const r = pu.regionByKey.get(n.key)
      if (r !== undefined && r.extrude !== null) out.push(r.extrude)
      break
    }
    case 'draft-edge':
      add(n.key)
      break
    case 'draft-curve':
      for (const e of edgesOfCurveRef(pu, n.key)) add(e.key)
      break
    case 'draft-island': {
      const unit = pu.unitByKey.get(n.key)
      if (unit !== undefined) for (const e of unit.edges) add(e.key)
      break
    }
    default:
      break
  }
  return out
}

/**
 * The sketch and regions the selection stands for, or null when it holds no
 * closed free shape. Shapes from more than one sketch are not mixed: the
 * first sketch found wins.
 */
export function projectSourceOf(scene: PlaneUnitsScene, selection: readonly NodeRef[]): ProjectSource | null {
  let sketch: bigint | null = null
  let whole = false
  const regions = new Set<bigint>()
  for (const n of selection) {
    if (n.kind === 'sketch') {
      if (sketch === null || sketch === n.id) {
        sketch = n.id
        whole = true
      }
      continue
    }
    if (!isDraftRef(n)) continue
    const pu = readPlaneUnits(scene, n.ctx)
    if (pu === null) continue
    for (const hit of regionsOfRef(pu, n)) {
      if (sketch === null) sketch = hit.sketch
      if (hit.sketch === sketch) regions.add(hit.region)
    }
  }
  if (sketch === null) return null
  return { sketch, regions: whole ? [] : [...regions] }
}
