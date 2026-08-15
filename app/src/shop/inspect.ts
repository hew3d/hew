/**
 * inspect — Shop Mode's tap-to-inspect resolution (design §"Tap to
 * inspect"): given what a single tap resolved to, decide what the compact
 * floating info card shows. Kept UI-free and unit-tested with a fake
 * `Scene`, matching `panels/objectBounds.ts`'s pattern (client-side reads
 * over the same accessors the renderer/panels already call — no new
 * wasm-api surface).
 *
 * A tap reaches this module as TWO pieces of information from the SAME
 * Select-tool click resolution (`Viewport`'s `onSelect`/`onSelectSnap`):
 *   - `node` — the whole Object/Group/Instance the tap resolved UP to
 *     (`resolveSelectableRef`'s job; a face/edge/vertex snap on an Object
 *     all collapse to that Object's node).
 *   - `snap` — the raw pre-resolution snap, which still knows WHICH face or
 *     edge was actually under the tap (`snap.elementKind`).
 *
 * A tap that landed on a plain object edge reports THAT edge's length (the
 * design's "single most common workshop question answered in one tap")
 * ALONGSIDE its owning part's label — the card's title is never a bare
 * "Edge" (design_handoff_shop_mode/README.md §3: "never bare Edge"), so the
 * edge arm reuses `nodeLabelAndTag` (the same name resolution the node arm
 * uses) rather than inventing a second label path; every other tap —
 * including a face hit, deliberately (see `resolveInspect`) — falls back to
 * the whole node's card. `null` for an empty tap (both `node` and any
 * elementKind absent).
 */

import type { Scene as WasmScene } from '../wasm/loader'
import type { Snap } from '../tools/types'
import { boundsExtents, worldBoundsForSelection, type Vec3 } from '../panels/objectBounds'
import {
  buildTreeIndexMap,
  nodeKey,
  nodeKindToNumber,
  nodeRefFromJs,
  resolveLabel,
  type NodeRef,
} from '../panels/treeModel'

export type InspectResult =
  | { kind: 'edge'; lengthM: number; partLabel: string }
  | { kind: 'node'; node: NodeRef; label: string; tagLabel: string | null; extentsM: Vec3 | null }

/** Whole-part kinds long-press isolate and the object card apply to — the
 *  three kernel `NodeId` kinds (`treeModel.nodeKindToNumber`'s domain), as
 *  opposed to a free-standing sketch or one of its sub-entities, which
 *  Shop Mode's Select/Orbit/Tape Measure tool set can barely reach anyway. */
export function isWholePartKind(kind: NodeRef['kind']): boolean {
  return kind === 'object' || kind === 'group' || kind === 'instance'
}

/** The world-space distance between an edge's two endpoints, or `null` for
 *  a stale handle (`edge_endpoints`/`edge_endpoints_in_instance` returned
 *  `undefined`, or a malformed result). */
function edgeLengthM(scene: WasmScene, snap: Snap): number | null {
  if (snap.object === undefined || snap.element === undefined) return null
  const ends =
    snap.instance !== undefined
      ? scene.edge_endpoints_in_instance(snap.instance, snap.object, snap.element)
      : scene.edge_endpoints(snap.object, snap.element)
  if (ends === undefined || ends.length < 6) return null
  const dx = ends[3] - ends[0]
  const dy = ends[4] - ends[1]
  const dz = ends[5] - ends[2]
  return Math.hypot(dx, dy, dz)
}

/** The display name + optional single tag for `node`'s card — the same
 *  `resolveLabel`/positional-index/tag lookup `ObjectInfoPanel` runs,
 *  trimmed to what the compact card shows (one tag path, not the full
 *  list — the design calls for "name, tag", singular). */
function nodeLabelAndTag(scene: WasmScene, node: NodeRef): { label: string; tagLabel: string | null } {
  const kindNum = nodeKindToNumber(node.kind)
  let nameFromScene: string | undefined
  let defName: string | undefined
  if (node.kind === 'object') {
    nameFromScene = scene.object_name(node.id)
  } else if (node.kind === 'group') {
    nameFromScene = scene.group_name(node.id)
  } else if (node.kind === 'instance') {
    nameFromScene = scene.instance_name(node.id)
    const def = scene.instance_def(node.id)
    if (def !== undefined) defName = scene.component_name(def)
  }

  const treeIndex = buildTreeIndexMap(
    scene.top_level_nodes().map(nodeRefFromJs),
    (groupId) => scene.group_members(groupId).map(nodeRefFromJs),
  )
  const idx = treeIndex.get(nodeKey(node)) ?? 0
  // Callers only reach here for a whole-part kind (isWholePartKind's
  // check, run by resolveInspect before calling this) — object/group/
  // instance are exactly EntityKind's non-'sketch' members, so this
  // narrowing is safe despite NodeKind's wider sketch-scoped union.
  const label = resolveLabel(nameFromScene, defName, node.kind as 'object' | 'group' | 'instance', idx)

  const rawTags = kindNum >= 0 ? scene.node_tags(kindNum, node.id) : []
  const firstTag = rawTags[0]
  const tagLabel =
    firstTag === undefined
      ? null
      : firstTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0).join(' / ') || null

  return { label, tagLabel }
}

/** Resolve one tap's inspect result — see the module doc for the
 *  edge/node precedence. `null` means "dismiss the card": nothing was
 *  under the tap. */
export function resolveInspect(scene: WasmScene, node: NodeRef | null, snap: Snap | null): InspectResult | null {
  // The edge card requires a resolved whole-part `node`, not just an
  // `elementKind: 'edge'` snap (shop-mode playtest finding: phantom edges).
  // `onSelect` resolves an edge snap UP to its owning Object/Group/Instance
  // and hands us `node`; that resolution returns `null` for anything NOT
  // selectable here — most importantly a HIDDEN object (Viewport's
  // `selectionDeps().resolveObject` rejects hidden object/instance ids). A
  // tap in open space near a hidden part's edge would otherwise still report
  // that edge's length with a bare "Part" label, dimensioning geometry the
  // user can't even see. Gating on a real `node` drops that phantom: no
  // visible part under the tap → no card, exactly like an empty tap.
  if (snap !== null && snap.elementKind === 'edge' && node !== null && isWholePartKind(node.kind)) {
    const lengthM = edgeLengthM(scene, snap)
    if (lengthM !== null) {
      const partLabel = nodeLabelAndTag(scene, node).label
      return { kind: 'edge', lengthM, partLabel }
    }
  }
  // A face tap deliberately does NOT get a face card: with the widened
  // touch aperture, the middle of any part is a face hit, so face
  // precedence would make the whole-part card — the "tap a part for its
  // dimensions" gesture this mode exists for — unreachable from the
  // viewport. Edges keep precedence (a near-edge tap answering "how long
  // is this?" is the one-tap win); everything else is the part.
  if (node !== null && isWholePartKind(node.kind)) {
    const { label, tagLabel } = nodeLabelAndTag(scene, node)
    const bounds = worldBoundsForSelection(scene, [node])
    return { kind: 'node', node, label, tagLabel, extentsM: bounds === null ? null : boundsExtents(bounds) }
  }
  return null
}
