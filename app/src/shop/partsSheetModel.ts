/**
 * partsSheetModel — pure row/section builder for Shop Mode's Parts sheet
 * (PartsSheet.tsx): a mobile presentation "merging the Outliner and Tags
 * intents" over the SAME `treeModel.ts`/`tagModel.ts` logic the desktop
 * DocumentTree/TagsPanel already read — those two panels stay untouched;
 * this is a new, alternate read of the same scene data, UI-free like
 * `inspect.ts`/`isolate.ts` and unit-tested the same way.
 *
 * Sections = the document's tag tree (`tagModel.buildTagTree`), FLATTENED —
 * every `TagTreeNode` (root or nested) becomes its own section, labeled
 * with its full joined path ("Structure / Roof") rather than rendered as a
 * visually nested sub-list: a phone-width sheet has no room for
 * `TagsPanel`'s deep desktop indentation at the SECTION level, so the label
 * carries the hierarchy instead. A section's master-eye state is
 * `hiddenTagPaths.has(tagPathKey(path))` — exactly `TagsPanel`'s own
 * per-row toggle, independent of its parent's (a nested tag is never
 * implicitly hidden just because its parent tag is).
 *
 * Every top-level object/group/instance with NO tag at all lands in a
 * trailing synthetic catch-all section ("Unfiled" once real tag sections
 * exist, "Parts" when the document uses no tags at all) — nothing backs a
 * master toggle for it (a tag path is the only thing `hiddenTagPaths`/
 * `unionHiddenLeafIds` know how to hide by category), so it renders without
 * one. This keeps every part reachable from the sheet even in the common
 * case of a document that never used tags.
 *
 * Within a section, each directly-tagged (or, for the catch-all, top-level
 * untagged) node gets one row; a GROUP additionally gets one indented row
 * per member, recursively (`scene.group_members`, the same recursion
 * `collectLeafIds`/DocumentTree's own row nesting use) — mirroring how the
 * Outliner nests a group's contents. A Component instance is a leaf here,
 * same as everywhere else in the app (its definition's members aren't
 * independently toggleable view-state). A member that ALSO carries its own,
 * different tag still nests under its structural parent's row here AND gets
 * its own row in its own tag's section — an accepted, deliberately rare
 * double-listing (tagging a nested member separately from its container is
 * unusual) in exchange for the Outliner's structural nesting never silently
 * dropping an untagged member.
 *
 * A row's `hidden` flag reads straight off `unionHiddenLeafIds`'s output,
 * unioned with long-press isolate's complement when `isolatedNode` is
 * non-null (`isolate.ts`'s `isolateHiddenFor`) — together the SAME
 * renderer-hidden computation `ShopApp.tsx`'s `pushHidden` feeds the
 * viewport, so a row can never disagree with what's actually on screen
 * (isolating a part dims every OTHER row's eye here too, not just the
 * viewport). Because of that, tapping a row's own eye toggle only ever
 * flips its MANUAL hidden-key membership (`ShopApp.tsx`'s
 * `toggleHiddenNode`), same as `DocumentTree`'s per-node eye: if a row reads
 * hidden because its SECTION's tag is hidden (or because isolate is
 * active), toggling the row alone won't reveal it until that source is
 * also cleared — existing, documented desktop behavior (`TagsPanel.tsx`'s
 * module doc) for the tag case, and the same "Show all" chip for isolate.
 *
 * Sketches are excluded outright — the Parts sheet is a cutlist of real
 * solids; a sketch has no `objectBounds` dimensions to show.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import { unionHiddenLeafIds } from '../io/documentLoad'
import { isolateHiddenFor } from './isolate'
import { boundsExtents, worldBoundsForSelection, type Vec3 } from '../panels/objectBounds'
import {
  buildTreeIndexMap,
  collectLeafIds,
  nodeKey,
  nodeKindToNumber,
  nodeRefFromJs,
  resolveLabel,
  type NodeRef,
} from '../panels/treeModel'
import { buildTagTree, tagPathKey, type TagTreeNode } from '../panels/tagModel'

/** One row in the sheet — a whole part (object/group/instance). */
export interface PartsSheetRow {
  node: NodeRef
  label: string
  /** Indentation within its section: 0 = the section's own directly-tagged
   *  (or, for the catch-all section, top-level untagged) node; deeper for
   *  a group's nested members. */
  depth: number
  /** Overall world-space dimensions (`objectBounds`'s formatter, same as
   *  `InspectCard`) — `null` for a node that resolves to no mesh (an empty
   *  group, or a stale handle). */
  extentsM: Vec3 | null
  hidden: boolean
}

/** One section — a flattened tag-tree node, or the untagged catch-all. */
export interface PartsSheetSection {
  /** `tagPathKey(path)` for a real tag section, or the catch-all sentinel. */
  key: string
  label: string
  /** `null` for the catch-all section — nothing backs a master toggle for it. */
  path: string[] | null
  /** Master-toggle state; always `false` for the catch-all (no toggle shown). */
  hidden: boolean
  rows: PartsSheetRow[]
}

const CATCH_ALL_KEY = '__parts-sheet-unfiled__'

function flattenTagTree(nodes: TagTreeNode[], out: TagTreeNode[]): void {
  for (const n of nodes) {
    out.push(n)
    flattenTagTree(n.children, out)
  }
}

/** Only the kinds a section/row can ever represent — `top_level_nodes()`
 *  and `group_members()` are documented as Object/Group/Instance only, but
 *  this stays defensive rather than assuming it. */
function isWholePart(node: NodeRef): boolean {
  return node.kind === 'object' || node.kind === 'group' || node.kind === 'instance'
}

/** Append `node` (depth-first, parent before children) to `out` — the row
 *  list one section entry expands to. See the module doc for the
 *  group-recurses/instance-is-a-leaf rule. */
function walkNode(
  node: NodeRef,
  depth: number,
  scene: WasmScene,
  treeIndex: Map<string, number>,
  getGroupMembers: (groupId: bigint) => NodeRef[],
  hiddenObjectIds: ReadonlySet<bigint>,
  hiddenInstanceIds: ReadonlySet<bigint>,
  out: PartsSheetRow[],
): void {
  if (!isWholePart(node)) return
  const idx = treeIndex.get(nodeKey(node)) ?? 0
  let label: string
  if (node.kind === 'group') {
    label = resolveLabel(scene.group_name(node.id), undefined, 'group', idx)
  } else if (node.kind === 'instance') {
    const def = scene.instance_def(node.id)
    const defName = def !== undefined ? scene.component_name(def) : undefined
    label = resolveLabel(scene.instance_name(node.id), defName, 'instance', idx)
  } else {
    label = resolveLabel(scene.object_name(node.id), undefined, 'object', idx)
  }

  const bounds = worldBoundsForSelection(scene, [node])

  let hidden: boolean
  if (node.kind === 'object') {
    hidden = hiddenObjectIds.has(node.id)
  } else if (node.kind === 'instance') {
    hidden = hiddenInstanceIds.has(node.id)
  } else {
    // Group: reads hidden only once EVERY leaf beneath it is hidden — an
    // empty group (no leaves at all) never reads hidden ("nothing to hide").
    const { objectIds, instanceIds } = collectLeafIds(node, getGroupMembers)
    hidden =
      objectIds.length + instanceIds.length > 0 &&
      objectIds.every((id) => hiddenObjectIds.has(id)) &&
      instanceIds.every((id) => hiddenInstanceIds.has(id))
  }

  out.push({ node, label, depth, extentsM: bounds === null ? null : boundsExtents(bounds), hidden })

  if (node.kind === 'group') {
    for (const child of getGroupMembers(node.id)) {
      walkNode(child, depth + 1, scene, treeIndex, getGroupMembers, hiddenObjectIds, hiddenInstanceIds, out)
    }
  }
}

/**
 * Build every section + row the Parts sheet renders, in display order (tag
 * sections in `buildTagTree`'s depth-first order, then the untagged
 * catch-all last — omitted entirely when nothing is untagged). Pure
 * function of `scene` plus the two Shop Mode view-state hidden sets;
 * `hiddenKeys`/`hiddenTagPaths` are `ShopApp.tsx`'s state, unchanged shape
 * from `unionHiddenLeafIds`'s own parameters. `isolatedNode` — `ShopApp.tsx`'s
 * long-press isolate target, or `null` — unions isolate's own hidden
 * complement into every row's `hidden` flag (module doc), so the sheet
 * never shows a stale "visible" eye for a part isolate is currently hiding.
 */
export function buildPartsSheetSections(
  scene: WasmScene,
  hiddenKeys: ReadonlySet<string>,
  hiddenTagPaths: ReadonlySet<string>,
  isolatedNode: NodeRef | null = null,
): PartsSheetSection[] {
  const getGroupMembers = (groupId: bigint): NodeRef[] => scene.group_members(groupId).map(nodeRefFromJs)
  const topNodes = scene.top_level_nodes().map(nodeRefFromJs).filter(isWholePart)
  const treeIndex = buildTreeIndexMap(topNodes, getGroupMembers)

  const { objectIds, instanceIds } = unionHiddenLeafIds(scene, hiddenKeys, hiddenTagPaths)
  const hiddenObjectIds = new Set(objectIds)
  const hiddenInstanceIds = new Set(instanceIds)
  if (isolatedNode !== null) {
    const iso = isolateHiddenFor(
      isolatedNode,
      Array.from(scene.object_ids()),
      Array.from(scene.instance_ids()),
      getGroupMembers,
    )
    for (const id of iso.hiddenObjectIds) hiddenObjectIds.add(id)
    for (const id of iso.hiddenInstanceIds) hiddenInstanceIds.add(id)
  }

  // Every object/group/instance in the ENTIRE document (not just top-level)
  // that carries at least one tag — the same input shape TagsPanel.tsx
  // builds, so section membership matches that panel exactly.
  const allNodes: NodeRef[] = [
    ...Array.from(scene.object_ids()).map((id) => ({ kind: 'object' as const, id })),
    ...Array.from(scene.group_ids()).map((id) => ({ kind: 'group' as const, id })),
    ...Array.from(scene.instance_ids()).map((id) => ({ kind: 'instance' as const, id })),
  ]
  const tagged: { node: NodeRef; path: string[] }[] = []
  const taggedTopLevelKeys = new Set<string>()
  for (const node of allNodes) {
    const kindNum = nodeKindToNumber(node.kind)
    const rawTags = kindNum >= 0 ? scene.node_tags(kindNum, node.id) : []
    for (const rawTag of rawTags) {
      const path = rawTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
      if (path.length === 0) continue
      tagged.push({ node, path })
      taggedTopLevelKeys.add(nodeKey(node))
    }
  }
  const registryPaths = Array.from(scene.tag_meta_paths())
    .map((raw) => raw.split('/').map((s) => s.trim()).filter((s) => s.length > 0))
    .filter((path) => path.length > 0)
  const tagTree = buildTagTree(tagged, registryPaths)

  const flatTagNodes: TagTreeNode[] = []
  flattenTagTree(tagTree, flatTagNodes)

  const sections: PartsSheetSection[] = flatTagNodes.map((tn) => {
    const rows: PartsSheetRow[] = []
    for (const node of tn.nodes) {
      walkNode(node, 0, scene, treeIndex, getGroupMembers, hiddenObjectIds, hiddenInstanceIds, rows)
    }
    return {
      key: tagPathKey(tn.path),
      label: tn.path.join(' / '),
      path: tn.path,
      hidden: hiddenTagPaths.has(tagPathKey(tn.path)),
      rows,
    }
  })

  const catchAllRows: PartsSheetRow[] = []
  for (const node of topNodes) {
    if (taggedTopLevelKeys.has(nodeKey(node))) continue
    walkNode(node, 0, scene, treeIndex, getGroupMembers, hiddenObjectIds, hiddenInstanceIds, catchAllRows)
  }
  if (catchAllRows.length > 0) {
    sections.push({
      key: CATCH_ALL_KEY,
      label: sections.length === 0 ? 'Parts' : 'Unfiled',
      path: null,
      hidden: false,
      rows: catchAllRows,
    })
  }

  return sections
}

/** Total row count across every section — the sheet header's "N of M shown"
 *  pill's M (design_handoff_shop_mode/README.md §1 "Sheet header"). */
export function totalPartCount(sections: readonly PartsSheetSection[]): number {
  return sections.reduce((sum, s) => sum + s.rows.length, 0)
}

/** Count of rows NOT currently hidden — the same pill's N. Reads each row's
 *  already-unioned `hidden` flag (manual hides ∪ tag hides ∪ isolate — the
 *  module doc), so it always agrees with what the eye/tag/isolate state
 *  actually renders without re-deriving any of it. */
export function visiblePartCount(sections: readonly PartsSheetSection[]): number {
  return sections.reduce((sum, s) => sum + s.rows.filter((r) => !r.hidden).length, 0)
}
