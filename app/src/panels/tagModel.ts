/**
 * tagModel.ts — pure helpers for the Tags panel.
 *
 * SketchUp's COLLADA exporter drops Tag/Layer assignments; the companion Ruby
 * script (tools/sketchup/hew_export_tags.rb) smuggles each entity's tag path
 * into its name using the format:
 *
 *   <display name>__HEWTAG__<seg>__HEWSEP__<seg>...
 *
 * **Important:** SketchUp sanitizes COLLADA node names to `[A-Za-z0-9_]` — every
 * other character (including the Ruby's original `@`/`/` and any spaces in a tag
 * name) is collapsed to `_`, and an extra leading `_` may be prepended. So the
 * delimiters arrive as a RUN of underscores around the `HEWTAG`/`HEWSEP` tokens
 * (e.g. `@@HEWTAG@@` was seen in the wild as `___HEWTAG__`). We therefore match
 * the delimiter/separator with `_+TOKEN_+` rather than a literal, and a tag whose
 * SketchUp name had spaces shows here with underscores (lossy, unavoidable).
 *
 * This module is UI-free and three.js-free — pure TS functions for parsing,
 * tree building, and path comparison. Unit-tested in tagModel.test.ts.
 */

import type { NodeRef } from './treeModel'

/** Delimiter between the display name and the tag data (underscore-tolerant). */
const DELIM_RE = /_+HEWTAG_+/
/** Separator between nestable tag-path segments (underscore-tolerant). */
const NEST_RE = /_+HEWSEP_+/

/**
 * Parse the encoded name into a display string and an optional tag path array.
 *
 * Returns:
 *   - `display`: the portion before the `__HEWTAG__` delimiter (or the whole
 *     name when untagged).
 *   - `path`: root-first tag-path segments, or `null` if untagged.
 */
export function parseTag(name: string): { display: string; path: string[] | null } {
  const m = DELIM_RE.exec(name)
  if (m === null) {
    return { display: name, path: null }
  }
  const display = name.slice(0, m.index)
  const encoded = name.slice(m.index + m[0].length)
  const path = encoded.split(NEST_RE).filter((s) => s.length > 0)
  return { display, path: path.length > 0 ? path : null }
}

// ---------------------------------------------------------------------------
// Tag tree
// ---------------------------------------------------------------------------

/**
 * A node in the nestable tag tree.
 *
 * - `segment`: the display name of this tag folder/tag (one path segment).
 * - `path`: the full root-first path array down to this node (immutable, used as key).
 * - `nodes`: the set of NodeRefs tagged *exactly* at this path (not descendants).
 * - `children`: sub-tags nested under this tag.
 */
export interface TagTreeNode {
  segment: string
  path: string[]
  nodes: NodeRef[]
  children: TagTreeNode[]
}

/**
 * Build a nestable TagTreeNode array from a list of tagged nodes.
 *
 * The input is a flat list of `{ node: NodeRef; path: string[] }` pairs.
 * The output is the root-level tag nodes; each carries its direct NodeRefs and
 * any child tag nodes.
 *
 * Tag paths are root-first (e.g. `["Structure", "Roof"]`).  A node tagged at
 * `["Structure", "Roof"]` appears in the `nodes` array of the "Roof" child of
 * the "Structure" root tag — NOT at the "Structure" root itself.
 */
export function buildTagTree(
  tagged: { node: NodeRef; path: string[] }[],
): TagTreeNode[] {
  // Use a map keyed by JSON-serialised path for O(1) lookup/insertion.
  const byPath = new Map<string, TagTreeNode>()

  /** Get or create the TagTreeNode at `path`, recursively creating parents. */
  function getOrCreate(path: string[]): TagTreeNode {
    const key = tagPathKey(path)
    const existing = byPath.get(key)
    if (existing !== undefined) return existing

    const segment = path[path.length - 1]
    const node: TagTreeNode = { segment, path, nodes: [], children: [] }
    byPath.set(key, node)

    if (path.length > 1) {
      const parent = getOrCreate(path.slice(0, -1))
      parent.children.push(node)
    }

    return node
  }

  for (const { node, path } of tagged) {
    if (path.length === 0) continue
    const treeNode = getOrCreate(path)
    treeNode.nodes.push(node)
  }

  // Collect only the root-level tag nodes (path.length === 1).
  const roots: TagTreeNode[] = []
  for (const node of byPath.values()) {
    if (node.path.length === 1) {
      roots.push(node)
    }
  }

  // Sort roots and children alphabetically for stable display.
  roots.sort((a, b) => a.segment.localeCompare(b.segment))
  for (const root of byPath.values()) {
    root.children.sort((a, b) => a.segment.localeCompare(b.segment))
  }

  return roots
}

/**
 * Collect ALL NodeRefs under a TagTreeNode — the node's own `nodes` plus every
 * descendant tag's nodes. Used when hiding a parent tag hides its subtree.
 */
export function collectTagDescendantNodes(tagNode: TagTreeNode): NodeRef[] {
  const result: NodeRef[] = [...tagNode.nodes]
  for (const child of tagNode.children) {
    result.push(...collectTagDescendantNodes(child))
  }
  return result
}

/**
 * Stable string key for a tag path array. Keyed by JSON so the key is
 * round-trippable and distinct from any encoded name string.
 */
export function tagPathKey(path: string[]): string {
  return JSON.stringify(path)
}

/**
 * Whether `candidatePath` is at or under `anchorPath` in the tag hierarchy.
 * Used to determine whether a node's tag path is covered by a hidden tag path.
 *
 * e.g. `isPathUnder(["A","B"], ["A"])` → true  (B is under A)
 *      `isPathUnder(["A"], ["A"])` → true      (exact match)
 *      `isPathUnder(["A"], ["A","B"])` → false  (A is above B, not under)
 */
export function isPathUnder(candidatePath: string[], anchorPath: string[]): boolean {
  if (candidatePath.length < anchorPath.length) return false
  for (let i = 0; i < anchorPath.length; i++) {
    if (candidatePath[i] !== anchorPath[i]) return false
  }
  return true
}
