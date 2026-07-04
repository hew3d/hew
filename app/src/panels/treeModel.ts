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
 * Kind of a document node. `'sketch'` is a free-standing,
 * not-yet-extruded sketch — it has no kernel `NodeId`/FFI `node_id` ('s
 * NodeId enumerates only Object/Group/Instance), so it deliberately stays out
 * of `nodeKindToNumber`'s mapping; sketch delete/pick route through dedicated
 * `delete_sketch`/`pick_sketch` wasm methods instead of `delete_node`.
 */
export type NodeKind = 'object' | 'group' | 'instance' | 'sketch'

/** A reference to a document node: kind + opaque handle. */
export interface NodeRef {
  kind: NodeKind
  id: bigint
}

/** Return true when two NodeRefs refer to the same node. */
export function nodeEq(a: NodeRef, b: NodeRef): boolean {
  return a.kind === b.kind && a.id === b.id
}

/** Stable string key for a NodeRef, usable in a Set or Map. */
export function nodeKey(n: NodeRef): string {
  return `${n.kind}:${n.id}`
}

/** Convert a NodeJs FFI value (has .kind and .id) to a plain NodeRef. */
export function nodeRefFromJs(n: { kind: string; id: bigint }): NodeRef {
  return { kind: n.kind as NodeKind, id: n.id }
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
export function entityLabel(kind: EntityKind, index: number): string {
  const name =
    kind === 'object' ? 'Object' :
    kind === 'group' ? 'Group' :
    kind === 'instance' ? 'Component' :
    'Sketch'
  return `${name} ${index + 1}`
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
 *   definition; used when the instance has no own name.
 * - `kind` / `index`: passed to `entityLabel` as a last-resort fallback.
 */
export function resolveLabel(
  kernelName: string | undefined,
  defName: string | undefined,
  kind: EntityKind,
  index: number,
): string {
  // A name that is purely a tag suffix (unnamed group/object that the Ruby
  // tagged) strips to empty → fall through to the positional label, not a blank.
  if (kernelName !== undefined) {
    const stripped = stripTagSuffix(kernelName)
    if (stripped.length > 0) return stripped
  }
  if (kind === 'instance' && defName !== undefined) {
    const stripped = stripTagSuffix(defName)
    if (stripped.length > 0) return stripped
  }
  return entityLabel(kind, index)
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
  const root: Crumb = { label: 'Model', depth: -1 }
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
 * Whether a selection can be folded into a component.
 * Rules: one or more distinct objects/groups (no instances — nested defs are
 * deferred). A single object is the common case; multiple must be siblings
 * (share a parent), like canGroup but without its ≥2 requirement.
 */
export function canMakeComponent(
  selected: NodeRef[],
  parentOf: (n: NodeRef) => bigint | undefined,
): boolean {
  // No instances allowed as members.
  if (selected.some((n) => n.kind === 'instance')) return false

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
 * Whether the selection can be ungrouped: exactly one selected node that is
 * a group.
 */
export function canUngroup(selected: NodeRef[]): boolean {
  return selected.length === 1 && selected[0].kind === 'group'
}

/**
 * Next selection after a click. Selection is an **ordered** list (index 0 is
 * the primary pick); order matters for booleans (Subtract = first − second).
 *
 * - `node === null` (empty click) → clear.
 * - `additive` (shift-click) → toggle: append if absent, remove if present,
 *   preserving the order of the survivors.
 * - otherwise → replace with `[node]`.
 */
export function nextSelection(
  current: NodeRef[],
  node: NodeRef | null,
  additive: boolean,
): NodeRef[] {
  if (node === null) {
    return []
  }
  if (!additive) {
    return [node]
  }
  const exists = current.some((n) => nodeEq(n, node))
  return exists ? current.filter((n) => !nodeEq(n, node)) : [...current, node]
}
