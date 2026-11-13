import { t } from '@lingui/core/macro'
import { compareNames } from './nameSort'
import { draftKindLabel, draftRef, draftRefAlive, resolveDraft, type OutlinerDrawing, type PlaneUnitsScene } from '../tools/planeUnits'

/**
 * Pure model helpers for the document tree + editing context ( navigation).
 *
 * Editing context is app/session state (DESIGN #17), so all of its derived
 * presentation logic lives here as plain functions — UI-free and three.js-free,
 * unit-tested like geoHelpers. The React panel and the renderer consume these.
 */

/**
 * Delimiter injected by hew_export_tags.rb, as it survives SketchUp's name
 * sanitization: the original `@@HEWTAG@@` arrives as a run of underscores around
 * the `HEWTAG` token (e.g. `___HEWTAG__`), so match `_+HEWTAG_+`. See
 * `tagModel.ts` for the full rationale.
 */
const HEWTAG_DELIM_RE = /_+HEWTAG_+/

/**
 * Strip the `__HEWTAG__<tag path>` suffix from a raw kernel name, returning only
 * the human-readable display portion. If the delimiter is absent the name is
 * returned unchanged.
 */
export function stripTagSuffix(name: string): string {
  const m = HEWTAG_DELIM_RE.exec(name)
  return m === null ? name : name.slice(0, m.index)
}

/**
 * Kind of a document node. `'sketch'` is a whole free-standing sketch (a row
 * of the Outliner) with no kernel `NodeId`/FFI `node_id` ('s NodeId
 * enumerates only Object/Group/Instance), so it deliberately stays out of
 * `nodeKindToNumber`'s mapping.
 *
 * The four `draft-*` kinds are the DRAWING on a plane (unified-drafting
 * design §8): every line, curve, and shape drawn on a plane — whether it
 * lives in a solid's face topology, an object-owned sketch, or a free
 * sketch — is addressed by the plane's context string and a kernel unit
 * key, never by a store handle. `'draft-island'` is one connected shape (the
 * user-facing unit), `'draft-curve'` a drawn circle's facets, `'draft-edge'`
 * one line, `'draft-region'` one filled region. None has a kernel NodeId;
 * see `app/src/tools/planeUnits.ts`.
 */
export type NodeKind =
  | 'object'
  | 'group'
  | 'instance'
  | 'sketch'
  | 'draft-island'
  | 'draft-curve'
  | 'draft-edge'
  | 'draft-region'

/** A reference to a document node: kind + opaque handle. The draft kinds
 * additionally carry the canonical plane context string and the unit's key
 * (16 lowercase hex digits); `id` is that key as a bigint. */
export interface NodeRef {
  kind: NodeKind
  id: bigint
  /** Canonical plane context (`{"plane":[…]}` + optional `"instance"`) —
   *  set iff `kind` is a draft kind. */
  ctx?: string
  /** The kernel unit/edge/curve/region key, hex — set iff a draft kind. */
  key?: string
}

/** Whether `kind` is one of the draft (drawing-on-a-plane) kinds. */
export function isDraftKind(kind: NodeKind): kind is 'draft-island' | 'draft-curve' | 'draft-edge' | 'draft-region' {
  return kind === 'draft-island' || kind === 'draft-curve' || kind === 'draft-edge' || kind === 'draft-region'
}

/** Whether `kind` has no kernel NodeId: a whole sketch or any draft kind. */
export function isSketchScopedKind(kind: NodeKind): kind is 'sketch' | 'draft-island' | 'draft-curve' | 'draft-edge' | 'draft-region' {
  return kind === 'sketch' || isDraftKind(kind)
}

/** Return true when two NodeRefs refer to the same node. */
export function nodeEq(a: NodeRef, b: NodeRef): boolean {
  return a.kind === b.kind && a.id === b.id && a.ctx === b.ctx
}

/** Stable string key for a NodeRef, usable in a Set or Map. */
export function nodeKey(n: NodeRef): string {
  if (n.ctx !== undefined) return `${n.kind}:${n.ctx}:${n.key ?? n.id}`
  return `${n.kind}:${n.id}`
}

/** Convert a NodeJs FFI value (has .kind and .id) to a plain NodeRef. */
export function nodeRefFromJs(n: { kind: string; id: bigint }): NodeRef {
  return { kind: n.kind as NodeKind, id: n.id }
}

/**
 * Recursively expand a node to the leaf object/instance ids that actually own
 * renderable geometry. Objects and instances are already leaves; groups have
 * no geometry of their own (DESIGN: groups are pure organization) and are
 * expanded via `getGroupMembers`, which may itself return nested groups —
 * this recurses until only objects/instances remain. Sketches contribute no
 * leaves (they have their own selection/highlight path).
 *
 * Pure — the caller supplies `getGroupMembers` (typically backed by the wasm
 * `scene.group_members()` FFI call) so this stays free of wasm/three deps.
 * Shared by the hidden-tag union path (App.tsx) and the selection-highlight
 * path (Viewport.tsx) so both agree on what a group "contains".
 */
export function collectLeafIds(
  node: NodeRef,
  getGroupMembers: (groupId: bigint) => NodeRef[],
): { objectIds: bigint[]; instanceIds: bigint[] } {
  if (node.kind === 'object') return { objectIds: [node.id], instanceIds: [] }
  if (node.kind === 'instance') return { objectIds: [], instanceIds: [node.id] }
  if (isSketchScopedKind(node.kind)) {
    return { objectIds: [], instanceIds: [] }
  }
  // Group: recurse into members (may themselves be nested groups).
  const objectIds: bigint[] = []
  const instanceIds: bigint[] = []
  for (const child of getGroupMembers(node.id)) {
    const { objectIds: os, instanceIds: is_ } = collectLeafIds(child, getGroupMembers)
    objectIds.push(...os)
    instanceIds.push(...is_)
  }
  return { objectIds, instanceIds }
}

/**
 * All descendants of `children` (a container's DIRECT children), expanded
 * recursively through nested groups — every level, not just the renderable
 * leaves `collectLeafIds` returns. Used by the Outliner's "hide/show all
 * children" control: it needs every descendant's own `nodeKey` (a nested
 * group can carry its OWN hidden flag, independent of its parent's) both to
 * test "is anything under here hidden" and, on Show All, to clear every one
 * of them — not just the direct children — so a hidden grandchild doesn't
 * stay stranded hidden under a freshly-shown group.
 *
 * Pure — the caller supplies `getGroupMembers` like `collectLeafIds`/
 * `buildTreeIndexMap` do.
 */
export function collectDescendants(
  children: readonly NodeRef[],
  getGroupMembers: (groupId: bigint) => NodeRef[],
): NodeRef[] {
  const out: NodeRef[] = []
  const walk = (nodes: readonly NodeRef[]) => {
    for (const n of nodes) {
      out.push(n)
      if (n.kind === 'group') walk(getGroupMembers(n.id))
    }
  }
  walk(children)
  return out
}

/**
 * Outliner text filter (MaterialPalette's filter pattern, applied to the
 * document tree): case-insensitive substring match against each node's
 * display label, walked recursively through group members. Returns `null`
 * for a blank/whitespace-only query — the "no filter active" sentinel the
 * caller uses to skip all of the visibility/dimming logic below.
 *
 * `matches` is every node whose OWN label matched; `ancestors` is every
 * group that isn't itself a match but contains one (directly or nested) —
 * the Outliner force-expands these and renders them dimmed, so a match deep
 * in a collapsed group is reachable without hiding the path to it. A group
 * that matches by name is naturally shown but its NON-matching children are
 * not auto-revealed (only the path TO a match is forced open, not
 * everything under one).
 *
 * Pure — `topNodes`/`getChildren`/`getLabel` mirror the callback shape
 * `buildTreeIndexMap` already uses, so the caller (DocumentTree) can reuse
 * the same closures for both.
 */
export function filterTreeKeys(
  topNodes: readonly NodeRef[],
  getChildren: (node: NodeRef) => NodeRef[],
  getLabel: (node: NodeRef) => string,
  query: string,
): { matches: Set<string>; ancestors: Set<string> } | null {
  const q = query.trim().toLowerCase()
  if (q === '') return null
  const matches = new Set<string>()
  const ancestors = new Set<string>()
  // Returns true when `node` itself matches or contains a match anywhere
  // beneath it — the caller marks it as an ancestor to force-expand/dim.
  const walk = (node: NodeRef, path: readonly NodeRef[]): boolean => {
    const isMatch = getLabel(node).toLowerCase().includes(q)
    let anyDescendantMatch = false
    for (const child of getChildren(node)) {
      if (walk(child, [...path, node])) anyDescendantMatch = true
    }
    if (isMatch) matches.add(nodeKey(node))
    if (isMatch || anyDescendantMatch) {
      for (const ancestor of path) ancestors.add(nodeKey(ancestor))
      return true
    }
    return false
  }
  for (const node of topNodes) walk(node, [])
  return { matches, ancestors }
}

/** Kind order for the Outliner's Kind sort: objects, groups, components,
 *  sketches, then the draft kinds. */
export function outlinerKindRank(kind: NodeKind): number {
  switch (kind) {
    case 'object': return 0
    case 'group': return 1
    case 'instance': return 2
    case 'sketch': return 3
    case 'draft-island': return 4
    case 'draft-curve': return 5
    case 'draft-edge': return 6
    case 'draft-region': return 7
  }
}

/**
 * Order the rows of ONE Outliner level. `'document'` keeps the given order;
 * `'name'` sorts by the label each row prints (case-insensitive, numeric, so
 * "Group 2" precedes "Group 10"); `'kind'` groups by kind, then by name.
 * Display order only: each entry keeps the positional `index` its generated
 * label was built from, so "Group 3" stays "Group 3". Stable, and returns a
 * new array.
 */
export function sortOutlinerLevel<T extends { node: NodeRef }>(
  entries: readonly T[],
  mode: 'document' | 'name' | 'kind',
  labelOf: (node: NodeRef) => string,
): T[] {
  if (mode === 'document') return [...entries]
  const keyed = entries.map((entry) => ({ entry, label: labelOf(entry.node) }))
  keyed.sort((a, b) => {
    if (mode === 'kind') {
      const byKind = outlinerKindRank(a.entry.node.kind) - outlinerKindRank(b.entry.node.kind)
      if (byKind !== 0) return byKind
    }
    return compareNames(a.label, b.label)
  })
  return keyed.map((k) => k.entry)
}

/**
 * Positional index of every node as the Outliner displays it: position within
 * its parent container (top-level order at depth 0, member order inside each
 * group), keyed by `nodeKey`. This is the index `resolveLabel`'s positional
 * fallback needs anywhere a label must match the Outliner — Object Info and
 * the command palette previously numbered from the flat per-kind id lists, so
 * an unnamed object nested in a group could be "Object 1" in the Outliner but
 * "Object 3" elsewhere.
 *
 * Pure — the caller supplies the top-level list and `getGroupMembers`
 * (typically `scene.top_level_nodes()` / `scene.group_members()` mapped
 * through `nodeRefFromJs`), mirroring `collectLeafIds`.
 */
export function buildTreeIndexMap(
  topNodes: NodeRef[],
  getGroupMembers: (groupId: bigint) => NodeRef[],
): Map<string, number> {
  const indices = new Map<string, number>()
  const walk = (nodes: NodeRef[]) => {
    nodes.forEach((node, i) => {
      indices.set(nodeKey(node), i)
      if (node.kind === 'group') walk(getGroupMembers(node.id))
    })
  }
  walk(topNodes)
  return indices
}

/**
 * The Outliner's positional index of `node` (its place among its parent's
 * children), or null when the node is not in the tree. A group session hides
 * its group in the kernel, so callers read this BEFORE opening the session
 * and keep the label they build from it.
 */
export function treeIndexOf(
  node: NodeRef,
  topNodes: NodeRef[],
  getGroupMembers: (groupId: bigint) => NodeRef[],
): number | null {
  return buildTreeIndexMap(topNodes, getGroupMembers).get(nodeKey(node)) ?? null
}

/** Kind of a top-level document entity shown in the tree. */
export type EntityKind = 'object' | 'sketch' | 'group' | 'instance'

/** One breadcrumb segment. `depth` is 0 for the root "Model" crumb. */
export interface Crumb {
  label: string
  /** Index into the path this crumb represents; -1 for the root crumb. */
  depth: number
}

/**
 * Display label for a tree row. `index` is the 0-based position within its
 * kind's list; labels are 1-based ("Object 1", "Sketch 1", "Group 1").
 * Generated app-side from handle order — no kernel naming, not persisted.
 */
export function entityLabel(kind: EntityKind, index: number | null): string {
  // No known position (a node outside the tree): the unnumbered name is
  // honest where a number would be a guess.
  if (index === null) {
    switch (kind) {
      case 'object':
        return t({ context: 'unnamed item', comment: 'Default label for an unnamed object whose position is unknown', message: 'Object' })
      case 'group':
        return t({ context: 'unnamed item', comment: 'Default label for an unnamed group whose position is unknown', message: 'Group' })
      case 'instance':
        return t({ context: 'unnamed item', comment: 'Default label for an unnamed component whose position is unknown', message: 'Component' })
      default:
        return t({ context: 'unnamed item', comment: 'Default label for an unnamed sketch whose position is unknown', message: 'Sketch' })
    }
  }
  const n = index + 1
  switch (kind) {
    case 'object':
      return t({ context: 'unnamed item', comment: 'Default row label for an unnamed object; n is its 1-based position', message: `Object ${n}` })
    case 'group':
      return t({ context: 'unnamed item', comment: 'Default row label for an unnamed group; n is its 1-based position', message: `Group ${n}` })
    case 'instance':
      return t({ context: 'unnamed item', comment: 'Default row label for an unnamed component; n is its 1-based position', message: `Component ${n}` })
    default:
      return t({ context: 'unnamed item', comment: 'Default row label for an unnamed sketch; n is its 1-based position', message: `Sketch ${n}` })
  }
}

/**
 * Resolve the display label for a tree row, preferring kernel-supplied names
 * over the positional fallback.
 *
 * Pure — no scene access. The caller resolves kernel names and passes them in.
 *
 * - `kernelName`: direct name on the node (object_name / group_name /
 *   instance_name), if any.
 * - `defName`: for instances only, the component_name of the instance's
 *   definition. An instance with no own name displays the definition name —
 *   every unrenamed instance of one component reads identically, which is
 *   what makes them obviously instances of each other. An instance that HAS
 *   its own name displays "Instance Name (Definition Name)", keeping the
 *   definition relationship visible; the parenthetical is dropped when the
 *   two names coincide ("Box (Box)" says nothing).
 * - `kind` / `index`: passed to `entityLabel` as a last-resort fallback.
 */
export function resolveLabel(
  kernelName: string | undefined,
  defName: string | undefined,
  kind: EntityKind,
  index: number | null,
): string {
  // A name that is purely a tag suffix (unnamed group/object that the Ruby
  // tagged) strips to empty → fall through to the positional label, not a blank.
  const strippedDef = defName !== undefined ? stripTagSuffix(defName) : ''
  if (kernelName !== undefined) {
    const stripped = stripTagSuffix(kernelName)
    if (stripped.length > 0) {
      if (kind === 'instance' && strippedDef.length > 0 && strippedDef !== stripped) {
        return `${stripped} (${strippedDef})`
      }
      return stripped
    }
  }
  if (kind === 'instance' && strippedDef.length > 0) {
    return strippedDef
  }
  return entityLabel(kind, index)
}

/** The slice of the wasm `Scene` the shared label resolver reads. */
export interface LabelScene {
  group_name(id: bigint): string | undefined
  object_name(id: bigint): string | undefined
  instance_name(id: bigint): string | undefined
  instance_def(id: bigint): bigint | undefined
  component_name(def: bigint): string | undefined
  sketch_ids?(): ArrayLike<bigint>
  /** Whether an instance places a linked set (copies made by Move, Paste, ...). */
  is_linked?(instance: bigint): boolean
  component_member_objects?(def: bigint): ArrayLike<bigint>
}

const namedOrNull = (n: string | undefined): string | null => {
  if (n === undefined) return null
  const stripped = stripTagSuffix(n)
  return stripped.length > 0 ? stripped : null
}

/** What every copy of a linked set is called when none has a name of its own:
 *  the set's name, else its single part's name, else null. */
export function sharedLinkedLabel(scene: LabelScene, def: bigint | undefined): string | null {
  if (def === undefined) return null
  const shared = namedOrNull(scene.component_name(def))
  if (shared !== null) return shared
  if (scene.component_member_objects !== undefined) {
    const members = Array.from(scene.component_member_objects(def))
    if (members.length === 1) return namedOrNull(scene.object_name(members[0]))
  }
  return null
}

/**
 * The label of an instance row. A placement of a linked set reads as the
 * thing it is, never as a component: the placement's own name, else the
 * set's name, else its single part's name, else "Copies". Any other
 * instance keeps `resolveLabel`'s component wording.
 */
export function placementLabel(scene: LabelScene, instance: bigint, index: number | null): string {
  const def = scene.instance_def(instance)
  const defName = def !== undefined ? scene.component_name(def) : undefined
  if (scene.is_linked === undefined || !scene.is_linked(instance)) {
    return resolveLabel(scene.instance_name(instance), defName, 'instance', index)
  }
  const own = namedOrNull(scene.instance_name(instance))
  if (own !== null) return own
  const shared = sharedLinkedLabel(scene, def)
  if (shared !== null) return shared
  return t({ context: 'unnamed item', comment: 'Row label for a placement of a linked set that has no name at all', message: 'Copies' })
}

/**
 * The one label resolver every surface prints: Outliner rows and breadcrumb,
 * the Selection path strip, the hover tip line, and Object Info
 * (docs/design/selection-path.md "Shared code"). `treeIndex` is the
 * positional-fallback map from `buildTreeIndexMap`; `frameLabelByKey` carries
 * the labels captured for open session frames, whose nodes the kernel hides
 * for the session's duration so a scene query answers nothing for them.
 */
export function makeLabelFor(
  scene: LabelScene,
  treeIndex: ReadonlyMap<string, number>,
  frameLabelByKey: ReadonlyMap<string, string> = new Map(),
  drawing?: () => OutlinerDrawing,
): (node: NodeRef) => string {
  // The Outliner numbers free sketch rows by their position among
  // `drawing.free`, and numbers drawn units per owning object
  // (`drawing.labels`). Both come from one lazily read OutlinerDrawing so a
  // model without drawn selections never pays for the kernel read.
  let sketchRowOf: Map<bigint, number> | undefined
  const sketchRows = (): Map<bigint, number> => {
    if (sketchRowOf !== undefined) return sketchRowOf
    const m = new Map<bigint, number>()
    drawing?.().free.forEach((u, i) => {
      for (const e of u.unit.edges) {
        if (e.store.kind === 'free' && !m.has(e.store.sketch)) m.set(e.store.sketch, i)
      }
    })
    return (sketchRowOf = m)
  }
  return (node) => {
    const frameLabel = frameLabelByKey.get(nodeKey(node))
    if (frameLabel !== undefined) return frameLabel
    if (isDraftKind(node.kind)) {
      const d = drawing?.()
      if (d !== undefined) {
        const own = d.labels.get(nodeKey(node))
        if (own !== undefined) return own
        if (node.kind !== 'draft-island' && node.ctx !== undefined) {
          const unit = resolveDraft(scene as unknown as PlaneUnitsScene, node)?.unit
          if (unit !== null && unit !== undefined) {
            const viaUnit = d.labels.get(nodeKey(draftRef('draft-island', node.ctx, unit.key)))
            if (viaUnit !== undefined) return viaUnit
          }
        }
      }
      return draftKindLabel(node.kind)
    }
    if (node.kind === 'sketch') {
      let at = sketchRows().get(node.id)
      if (at === undefined) {
        const ids = scene.sketch_ids !== undefined ? Array.from(scene.sketch_ids()) : []
        at = ids.findIndex((id) => id === node.id)
      }
      return entityLabel('sketch', at >= 0 ? at : null)
    }
    const idx = treeIndex.get(nodeKey(node)) ?? 0
    if (node.kind === 'group') {
      return resolveLabel(scene.group_name(node.id), undefined, 'group', idx)
    } else if (node.kind === 'instance') {
      return placementLabel(scene, node.id, idx)
    }
    return resolveLabel(scene.object_name(node.id), undefined, 'object', idx)
  }
}

/**
 * Breadcrumb trail for the current editing context path.
 * - Top level (empty path): `[{ label: 'Model', depth: -1 }]`
 * - Inside a group: `[Model, Group N, …]`
 * - Clicking a crumb at depth `d` means "truncate path to depth d+1"
 *   (the root crumb at depth -1 means "exit to top").
 *
 * `labelFor` maps a NodeRef to its display label; provided by the caller
 * so this remains pure and testable without touching the scene.
 */
export function breadcrumb(
  path: NodeRef[],
  labelFor: (node: NodeRef) => string,
): Crumb[] {
  const root: Crumb = { label: t({ context: 'root', comment: 'Breadcrumb for the top level of the document', message: 'Model' }), depth: -1 }
  if (path.length === 0) {
    return [root]
  }
  return [
    root,
    ...path.map((node, i) => ({ label: labelFor(node), depth: i })),
  ]
}

/**
 * Whether an entity should be dimmed in the tree. A tree row is dimmed when
 * the active context path is non-empty AND the row's node is not equal to —
 * or not an ancestor of — the deepest context node.
 *
 * Since the tree is rendered hierarchically (each parent shows its children),
 * we simplify: a row at a given depth is dimmed when it is NOT the context
 * node at that depth.
 *
 * `path` is the active context path (empty = top level; nothing is dimmed).
 * `node` is the row's NodeRef.
 * `depth` is the nesting depth in the tree (0 = top-level sibling).
 */
export function isTreeRowDimmed(
  path: NodeRef[],
  node: NodeRef,
  depth: number,
): boolean {
  if (path.length === 0) return false
  // There is an active context. A row at depth d is dimmed unless it is
  // exactly the context node at depth d (or it lives inside the path).
  const ctxAtDepth = path[depth]
  if (ctxAtDepth === undefined) {
    // Row is deeper than the context path — always inside context, not dimmed.
    return false
  }
  return !nodeEq(ctxAtDepth, node)
}

/**
 * Convert a NodeKind to the numeric kind tag used in WASM API calls.
 *   0 = object, 1 = group, 2 = instance
 *
 * `'sketch'` has no kernel `NodeId` variant (see the `NodeKind` doc comment):
 * sketch operations route through their own dedicated wasm methods
 * (`delete_sketch`/`pick_sketch`/`pick_sketch_region`/`transform_sketch`/…),
 * never a `node_id`-keyed call. Whole-sketch selection is now wired throughout
 * the UI (DocumentTree/ObjectInfoPanel/MaterialPalette/TagsPanel); every one of
 * those callers checks `kind === 'sketch'` and takes its own sketch-specific
 * path *before* reaching this function. This used to throw for `'sketch'` —
 * back when no caller was wired for a sketch selection, that was the loud
 * signal of a real gap. Now that they all guard, throwing would just be a
 * crash waiting for the one caller that forgets to; return the -1 sentinel
 * instead so a stray path degrades to "matches nothing" rather than throwing
 * mid-render. `-1` is not a valid `node_id` kind — never forward it to a
 * `node_id`-keyed wasm call.
 */
export function nodeKindToNumber(kind: NodeKind): number {
  if (kind === 'object') return 0
  if (kind === 'group') return 1
  if (kind === 'instance') return 2
  return -1
}

/**
 * Collapse a structural selection into the kernel's parallel kind/id arrays
 * (`group_nodes`, `make_component`, …) — or refuse with `null` if ANY node
 * has no kernel NodeId (the sketch-scoped kinds).
 *
 * This is the id-space boundary: sketch handles live in a different slotmap
 * than node ids, and slotmaps reuse bit patterns, so forwarding a sketch id
 * as kind 0 can silently address an UNRELATED live object. Callers that feed
 * node-id-keyed wasm calls must collapse through here rather than mapping
 * kinds ad hoc, and must treat `null` as a typed refusal — never a fallback.
 */
export function structuralSelection(
  nodes: readonly NodeRef[],
): { kinds: Uint8Array; ids: BigUint64Array } | null {
  const kinds: number[] = []
  const ids: bigint[] = []
  for (const n of nodes) {
    const kind = nodeKindToNumber(n.kind)
    if (kind < 0) return null
    kinds.push(kind)
    ids.push(n.id)
  }
  return { kinds: new Uint8Array(kinds), ids: new BigUint64Array(ids) }
}

/**
 * Whether a selection can be folded into a component.
 * Rules: one or more distinct objects, groups, or instances — a selected
 * instance becomes a nested member of the new definition — and no
 * sketch-scoped nodes (they have no kernel NodeId; see
 * `structuralSelection`). A single object is the common case; multiple must
 * be siblings (share a parent), like canGroup but without its ≥2 requirement.
 */
export function canMakeComponent(
  selected: NodeRef[],
  parentOf: (n: NodeRef) => bigint | undefined,
): boolean {
  // Objects, groups, and instances: a selected instance becomes a NESTED
  // member of the new definition (the kernel keeps groups whole and folds
  // instances in). A sketch-kind node must never reach the kernel's
  // node-id space.
  if (selected.some((n) => n.kind !== 'object' && n.kind !== 'group' && n.kind !== 'instance'))
    return false

  // Deduplicate by kind+id.
  const seen = new Set<string>()
  const distinct: NodeRef[] = []
  for (const n of selected) {
    const key = `${n.kind}:${n.id}`
    if (!seen.has(key)) {
      seen.add(key)
      distinct.push(n)
    }
  }
  if (distinct.length < 1) return false

  // A single node is always fine; multiple must share one parent (siblings).
  const firstParent = parentOf(distinct[0])
  return distinct.every((n) => {
    const p = parentOf(n)
    if (firstParent === undefined) return p === undefined
    return p === firstParent
  })
}

/**
 * Whether the selection contains exactly one instance (for Place Instance).
 */
export function canPlaceInstance(selected: NodeRef[]): boolean {
  return selected.length === 1 && selected[0].kind === 'instance'
}

/**
 * Whether the selection is exactly one instance (for Explode).
 * Mirrors canPlaceInstance — same rule, separate name for clarity.
 */
export function canExplodeInstance(selected: NodeRef[]): boolean {
  return selected.length === 1 && selected[0].kind === 'instance'
}

/**
 * Whether the selection is exactly one instance (for Make Unique).
 * Mirrors canPlaceInstance — same rule, separate name for clarity.
 */
export function canMakeUnique(selected: NodeRef[]): boolean {
  return selected.length === 1 && selected[0].kind === 'instance'
}

/**
 * Whether two or more selected nodes can be grouped: they must all
 * share the same parent (all top-level, or all direct children of one group).
 * Requires at least 2 distinct nodes.
 *
 * `parentOf` should return the containing group id, or undefined if top-level.
 */
export function canGroup(
  selected: NodeRef[],
  parentOf: (n: NodeRef) => bigint | undefined,
): boolean {
  if (selected.length < 2) return false

  // Only nodes with a kernel NodeId can be grouped — a sketch-scoped ref in
  // the selection disqualifies it outright (see `structuralSelection`).
  if (selected.some((n) => nodeKindToNumber(n.kind) < 0)) return false

  // Deduplicate by kind+id
  const seen = new Set<string>()
  const distinct: NodeRef[] = []
  for (const n of selected) {
    const key = `${n.kind}:${n.id}`
    if (!seen.has(key)) {
      seen.add(key)
      distinct.push(n)
    }
  }
  if (distinct.length < 2) return false

  // All must share the same parent
  const firstParent = parentOf(distinct[0])
  return distinct.every((n) => {
    const p = parentOf(n)
    // Both undefined (top-level), or both same group id
    if (firstParent === undefined) return p === undefined
    return p === firstParent
  })
}

/**
 * Whether the boolean commands (Union / Subtract / Intersect) apply: exactly
 * two distinct **top-level** operands, each a plain object, a group, or a
 * component instance.
 *
 * Top-level-ness mirrors the kernel's `GroupedOperand` refusal (a replacing
 * op consumes its operands and emits top-level results), so the gate never
 * lights up for a nested node picked in the Outliner only to be refused on
 * commit. An instance operand is deliberately allowed to light up the
 * command (playtest finding 4: the engrave/emboss use case needs a boolean
 * against 3D text, which is always a component) — the kernel refuses an
 * instance operand outright (`BooleanOperandHasInstance`), but the app
 * transparently makes it unique, explodes it, and retries
 * (`Viewport.tsx`'s `runBoolean`/`explodeInstanceOperand`) rather than
 * leaving the command disabled for a selection the flow can actually
 * handle. Deeper eligibility — solidity, an instance nested INSIDE a
 * group operand's subtree (which the auto-explode does not reach) — stays
 * with the kernel, which refuses typed.
 *
 * `isOperand` reports whether the node is a live object/group/instance in
 * the scene (the caller checks against the scene's id lists); `parentOf`
 * returns the containing group id, or undefined if top-level (as for
 * `canGroup`).
 */
export function canBoolean(
  selected: NodeRef[],
  parentOf: (n: NodeRef) => bigint | undefined,
  isOperand: (n: NodeRef) => boolean,
): boolean {
  if (selected.length < 2) return false
  if (new Set(selected.map(nodeKey)).size !== selected.length) return false
  return selected.every(
    (n) =>
      (n.kind === 'object' || n.kind === 'group' || n.kind === 'instance') &&
      isOperand(n) &&
      parentOf(n) === undefined,
  )
}

/**
 * Whether the boolean commands apply while editing INSIDE a component
 * instance's own definition (component-edit-parity.md phase A2): two or
 * more distinct 'object' nodes, all live MEMBERS of the entered definition —
 * `boolean_in_component_many` requires every operand from the SAME component. A
 * definition is flat (no nesting), so there is no "top-level" concept to
 * check here the way `canBoolean` checks it for the world.
 *
 * `isMember` reports whether the node is a live member of the entered
 * definition (the caller checks against `component_member_objects`).
 */
export function canBooleanInComponent(
  selected: NodeRef[],
  isMember: (n: NodeRef) => boolean,
): boolean {
  if (selected.length < 2) return false
  if (new Set(selected.map(nodeKey)).size !== selected.length) return false
  return selected.every((n) => n.kind === 'object' && isMember(n))
}

/**
 * Where an Outliner drag from `dragged` may drop onto `target`, or `null`
 * if the drop is refused — a pure gate the row's pointer handlers consult
 * before highlighting a drop target and before firing `reparent_nodes`.
 * Mirrors `Document::reparent_nodes`'s own refusals (`GroupCycle`,
 * `ExplodeSessionScope`) so the UI never highlights a target the kernel
 * would immediately refuse.
 *
 * `dragged` is every row being moved — the single dragged row, or the
 * whole current selection when the dragged row is part of it. `target` is
 * the row the pointer is over: a group row to move into, or `'root'` for
 * the Model row (move to the top level).
 *
 * Refused (`null`) when:
 * - `dragged` is empty, or any dragged node lacks a kernel NodeId (a
 *   sketch-scoped ref never reaches `reparent_nodes`; see
 *   `structuralSelection`).
 * - `view.sessionOpen` — a group/component edit session is open; the
 *   kernel refuses `ExplodeSessionScope` regardless of target.
 * - `target` is neither a group row nor `'root'` (an instance row, a
 *   sketch row, an object row — none can contain other nodes).
 * - `target` names a group that IS one of the dragged nodes, or lies
 *   inside one of the dragged groups' own subtree (dropping a group onto
 *   itself, or onto one of its own descendants) — the kernel's
 *   `GroupCycle`.
 *
 * Returns the resolved parent to pass to `reparent_nodes`: the target
 * group's id, or `undefined` for the top level. A target every dragged
 * node already sits directly under is still a valid (if no-op) drop —
 * `reparent_nodes` itself silently skips already-there nodes.
 */
export function dropTargetFor(
  dragged: readonly NodeRef[],
  target: NodeRef | 'root',
  view: {
    getGroupMembers: (groupId: bigint) => NodeRef[]
    sessionOpen: boolean
  },
): { group: bigint | undefined } | null {
  if (dragged.length === 0) return null
  if (view.sessionOpen) return null
  if (dragged.some((n) => nodeKindToNumber(n.kind) < 0)) return null

  if (target === 'root') return { group: undefined }
  if (target.kind !== 'group') return null

  // A group cannot end up inside itself or inside one of its own
  // descendants — mirrors the kernel's subtree walk (GroupCycle).
  const subtreeContains = (root: NodeRef, needle: bigint): boolean => {
    if (root.kind === 'group' && root.id === needle) return true
    if (root.kind !== 'group') return false
    return view.getGroupMembers(root.id).some((child) => subtreeContains(child, needle))
  }
  if (dragged.some((n) => subtreeContains(n, target.id))) return null

  return { group: target.id }
}

/**
 * Whether the selection can be ungrouped: exactly one selected node that is
 * a group.
 */
export function canUngroup(selected: NodeRef[]): boolean {
  return selected.length === 1 && selected[0].kind === 'group'
}

/**
 * How a pick combines with the current selection — SketchUp's Select-tool
 * modifier matrix (`selectModeFor` in viewport/selectModifiers.ts maps the
 * pointer event's modifiers onto it):
 *
 * - `'replace'` — a plain click / marquee: the pick becomes the selection.
 * - `'toggle'`  — Shift: each picked node flips (in ↔ out).
 * - `'add'`     — Ctrl/⌘/Option: picked nodes join; nothing ever leaves.
 * - `'subtract'`— Shift+Ctrl/⌘/Option: picked nodes leave; nothing joins.
 */
export type SelectMode = 'replace' | 'toggle' | 'add' | 'subtract'

/**
 * Next selection after a click. Selection is an **ordered** list (index 0 is
 * the primary pick); order matters for booleans (Subtract = first − second).
 *
 * - `node === null` (empty click) → clear under `'replace'`; every other
 *   mode leaves the selection alone (a Shift-click on air is not a
 *   "deselect everything" — SketchUp keeps the selection too).
 * - `'toggle'` → append if absent, remove if present, preserving the order
 *   of the survivors.
 * - `'add'` → append if absent, else unchanged.
 * - `'subtract'` → remove if present, else unchanged.
 * - `'replace'` → `[node]`.
 */
export function nextSelection(
  current: NodeRef[],
  node: NodeRef | null,
  mode: SelectMode,
): NodeRef[] {
  if (node === null) {
    return mode === 'replace' ? [] : current
  }
  if (mode === 'replace') {
    return [node]
  }
  const exists = current.some((n) => nodeEq(n, node))
  if (mode === 'add') return exists ? current : [...current, node]
  if (mode === 'subtract') return exists ? current.filter((n) => !nodeEq(n, node)) : current
  return exists ? current.filter((n) => !nodeEq(n, node)) : [...current, node]
}

/**
 * Next selection after a MULTI-node pick (a marquee, Select All, Invert
 * Selection). The same four modes as `nextSelection`, applied per node:
 *
 * - `'replace'` → `nodes` (an empty marquee clears, like clicking air).
 * - `'toggle'` → every picked node flips: the ones already selected leave,
 *   the rest join (SketchUp's Shift-marquee).
 * - `'add'` → the ones not yet selected join, in pick order, no duplicates.
 * - `'subtract'` → the picked ones leave.
 *
 * Survivors keep their order (index 0 stays the primary pick). Returns the
 * ORIGINAL array when nothing changes, so a state setter can hand it back
 * without a render churn.
 */
export function mergeSelection(
  current: NodeRef[],
  nodes: NodeRef[],
  mode: SelectMode,
): NodeRef[] {
  if (mode === 'replace') return nodes
  const picked = new Set(nodes.map(nodeKey))
  const have = new Set(current.map(nodeKey))
  if (mode === 'add') {
    const fresh = nodes.filter((n) => !have.has(nodeKey(n)))
    return fresh.length === 0 ? current : [...current, ...fresh]
  }
  if (mode === 'subtract') {
    const kept = current.filter((n) => !picked.has(nodeKey(n)))
    return kept.length === current.length ? current : kept
  }
  const kept = current.filter((n) => !picked.has(nodeKey(n)))
  const fresh = nodes.filter((n) => !have.has(nodeKey(n)))
  if (fresh.length === 0 && kept.length === current.length) return current
  return [...kept, ...fresh]
}

/** The slice of the wasm `Scene` that `pruneDeadSelection` reads. A draft
 * ref is alive while its plane's listing still holds its key
 * (`plane_units_json`); a scene without the listing treats every draft ref
 * as dead. */
export interface SelectionLivenessView {
  object_ids(): ArrayLike<bigint>
  group_ids(): ArrayLike<bigint>
  instance_ids(): ArrayLike<bigint>
  sketch_ids(): ArrayLike<bigint>
  plane_units_json?(ctx: string): string
  pick_plane_unit?(ctx: string, ray: Float64Array, tolerance: number): string
  history_generation?(): bigint
}

/**
 * Drop selection entries whose nodes the document no longer holds — the
 * reconcile that keeps the APP selection honest after any mutation that can
 * kill nodes (undo/redo above all, but also deletes, booleans consuming
 * their operands, extrusions consuming sketches…). Object Info and the
 * contextual dock render from this selection, so a stale entry shows a
 * phantom "N selected" and arms tools with dead handles.
 *
 * Handles are generational (DEVELOPMENT.md): a dead handle can never alias
 * a live node, so membership tests against the live id lists are exact.
 * Returns the ORIGINAL array when every entry is alive, so a state setter
 * can hand the result straight back without a render churn.
 */
export function pruneDeadSelection(
  scene: SelectionLivenessView,
  selected: NodeRef[],
  scopedObjects: readonly bigint[] = [],
  scopedSketches: readonly bigint[] = [],
): NodeRef[] {
  if (selected.length === 0) return selected

  // Lazily built: most selections are structural nodes only.
  let objects: Set<bigint> | null = null
  let groups: Set<bigint> | null = null
  let instances: Set<bigint> | null = null
  let sketches: Set<bigint> | null = null
  const sketchSet = (): Set<bigint> =>
    (sketches ??= new Set([...Array.from(scene.sketch_ids()), ...scopedSketches]))

  const alive = (n: NodeRef): boolean => {
    switch (n.kind) {
      case 'object':
        return (objects ??= new Set([...Array.from(scene.object_ids()), ...scopedObjects])).has(n.id)
      case 'group':
        return (groups ??= new Set(Array.from(scene.group_ids()))).has(n.id)
      case 'instance':
        return (instances ??= new Set(Array.from(scene.instance_ids()))).has(n.id)
      case 'sketch':
        return sketchSet().has(n.id)
      case 'draft-island':
      case 'draft-curve':
      case 'draft-edge':
      case 'draft-region':
        if (scene.plane_units_json === undefined) return false
        return draftRefAlive(scene as PlaneUnitsScene, n)
    }
  }

  const kept = selected.filter(alive)
  return kept.length === selected.length ? selected : kept
}
