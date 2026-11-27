/**
 * measureInfo — Volume, Surface area, and Size for the Object Info panel.
 *
 * The numbers come from the kernel's `hew.query.measure` builder through the
 * wasm `measure_json(kind, id)` (one call per selected root; hidden members
 * count, a scaled or mirrored instance is scaled by its pose determinant
 * kernel-side). This module only sums the roots and picks a Size:
 *
 * - volume is `null` when ANY leaf under ANY root is open (an open leaf makes
 *   the sum meaningless); area still sums.
 * - a root that sits inside another selected root is counted once.
 * - Size is the part frame's extents for a single node when `part_frame_json`
 *   answers (any source), else the combined world box.
 *
 * Results are memoized per scene by `history_generation`, so a repaint
 * costs nothing and an undo/redo or any edit recomputes.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import { nodeKey, nodeKindToNumber, nodeRefFromJs, type NodeRef } from './treeModel'
import type { Vec3 } from './objectBounds'

export interface SelectionMeasure {
  /** Cubic meters; `null` when any leaf is open (not closed). */
  volume: number | null
  /** Square meters, summed over every leaf. */
  area: number
  /** Length, width, thickness along the part frame, or the world box. */
  size: Vec3 | null
  /** The part frame's `source` when `size` came from it, else `null`. */
  sizeSource: string | null
}

interface MeasureJson {
  volume: number | null
  area: number
  bbox: { min: Vec3; max: Vec3 }
  open_leaves?: number
}

interface PartFrameJson {
  extents: Vec3
  source: string
}

/** Keys of every node strictly inside a selected group (recursively). */
function nestedKeys(scene: WasmScene, nodes: readonly NodeRef[]): Set<string> {
  const out = new Set<string>()
  const walk = (groupId: bigint) => {
    for (const raw of scene.group_members(groupId)) {
      const child = nodeRefFromJs(raw)
      out.add(nodeKey(child))
      if (child.kind === 'group') walk(child.id)
    }
  }
  for (const n of nodes) if (n.kind === 'group') walk(n.id)
  return out
}

/** The selected nodes that are not inside another selected node. */
function rootsOf(scene: WasmScene, nodes: readonly NodeRef[]): NodeRef[] {
  const inside = nestedKeys(scene, nodes)
  const seen = new Set<string>()
  const roots: NodeRef[] = []
  for (const n of nodes) {
    const key = nodeKey(n)
    if (nodeKindToNumber(n.kind) < 0 || inside.has(key) || seen.has(key)) continue
    seen.add(key)
    roots.push(n)
  }
  return roots
}

/** Thrown when a measure fails for a reason other than "nothing to measure". */
class MeasureRefused extends Error {}

/**
 * One root's measure. `null` means it contributes nothing (`NotPlaced`: stale
 * handle or a nested member; `NoGeometry`). Any other error (a refusal such
 * as `TooManyOccurrences`) throws `MeasureRefused` so no partial sum shows.
 */
function measureOne(scene: WasmScene, node: NodeRef): MeasureJson | null {
  try {
    const m = JSON.parse(scene.measure_json(nodeKindToNumber(node.kind), node.id)) as MeasureJson
    // Spec: volume is null when ANY leaf is open (the kernel sums only the
    // closed leaves of a mixed group); the bus shape stays as is.
    if ((m.open_leaves ?? 0) > 0) m.volume = null
    return m
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    if (/^(NotPlaced|NoGeometry)\b/.test(message)) return null
    throw new MeasureRefused(message)
  }
}

function partFrameOne(scene: WasmScene, node: NodeRef): PartFrameJson | null {
  try {
    return JSON.parse(scene.part_frame_json(nodeKindToNumber(node.kind), node.id)) as PartFrameJson
  } catch {
    return null
  }
}

function compute(scene: WasmScene, nodes: readonly NodeRef[]): SelectionMeasure | null {
  let roots: NodeRef[]
  try {
    roots = rootsOf(scene, nodes)
  } catch {
    return null
  }
  let volume: number | null = 0
  let area = 0
  let min: [number, number, number] | null = null
  let max: [number, number, number] | null = null
  let measured = 0
  let only: NodeRef | null = null
  for (const root of roots) {
    let m: MeasureJson | null
    try {
      m = measureOne(scene, root)
    } catch (e) {
      if (e instanceof MeasureRefused) return null
      throw e
    }
    if (m === null) continue
    measured++
    only = root
    area += m.area
    volume = volume === null || m.volume === null ? null : volume + m.volume
    if (min === null || max === null) {
      min = [...m.bbox.min]
      max = [...m.bbox.max]
    } else {
      for (let i = 0; i < 3; i++) {
        min[i] = Math.min(min[i], m.bbox.min[i])
        max[i] = Math.max(max[i], m.bbox.max[i])
      }
    }
  }
  if (measured === 0 || min === null || max === null) return null
  let size: Vec3 = [max[0] - min[0], max[1] - min[1], max[2] - min[2]]
  let sizeSource: string | null = null
  if (nodes.length === 1 && only !== null) {
    const frame = partFrameOne(scene, only)
    if (frame !== null) {
      size = frame.extents
      sizeSource = frame.source
    }
  }
  return { volume, area, size, sizeSource }
}

const cache = new WeakMap<WasmScene, { generation: bigint | number; epoch: bigint | number; key: string; value: SelectionMeasure | null }>()

/**
 * Volume, area, and size of a selection; `null` when nothing in it has
 * geometry (a Sketch, a stale handle). Memoized by `history_generation` and `document_epoch`.
 */
export function measureSelection(scene: WasmScene, nodes: readonly NodeRef[]): SelectionMeasure | null {
  let generation: bigint | number
  let epoch: bigint | number
  try {
    generation = scene.history_generation()
    // Scene.load swaps the document at generation 0, so the generation alone
    // can repeat across different documents.
    epoch = scene.document_epoch()
  } catch {
    return compute(scene, nodes)
  }
  const key = nodes.map(nodeKey).join('|')
  const hit = cache.get(scene)
  if (hit !== undefined && hit.generation === generation && hit.epoch === epoch && hit.key === key) return hit.value
  const value = compute(scene, nodes)
  cache.set(scene, { generation, epoch, key, value })
  return value
}
