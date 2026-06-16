/**
 * Pure model helpers for the document tree + editing context ( navigation).
 *
 * Editing context is app/session state (DESIGN #17), so all of its derived
 * presentation logic lives here as plain functions — UI-free and three.js-free,
 * unit-tested like geoHelpers. The React panel and the renderer consume these.
 */

/** Kind of a top-level document entity shown in the tree. */
export type EntityKind = 'object' | 'sketch'

/** One breadcrumb segment. `contextId === null` is the top-level "Model" crumb. */
export interface Crumb {
  label: string
  contextId: bigint | null
}

/**
 * Display label for a tree row. `index` is the 0-based position within its
 * kind's list; labels are 1-based ("Object 1", "Sketch 1"). Generated app-side
 * from handle order — no kernel naming, not persisted (pre-M3).
 */
export function entityLabel(kind: EntityKind, index: number): string {
  const name = kind === 'object' ? 'Object' : 'Sketch'
  return `${name} ${index + 1}`
}

/**
 * Breadcrumb trail for the current editing context. Top level is just
 * `[Model]`; inside an object it is `[Model, Object N]`, where N is the
 * object's 1-based position in `objectIds`. An active context not found in
 * `objectIds` (stale) collapses to top level.
 */
export function breadcrumb(activeContext: bigint | null, objectIds: bigint[]): Crumb[] {
  const root: Crumb = { label: 'Model', contextId: null }
  if (activeContext === null) {
    return [root]
  }
  const index = objectIds.indexOf(activeContext)
  if (index === -1) {
    return [root]
  }
  return [root, { label: entityLabel('object', index), contextId: activeContext }]
}

/**
 * Whether an entity should be dimmed (faded) under the active editing context.
 * At top level nothing is dimmed; inside object X everything except X fades —
 * the SketchUp "isolate" focus cue. Drives both the renderer and the tree.
 */
export function isDimmed(entityId: bigint, activeContext: bigint | null): boolean {
  return activeContext !== null && entityId !== activeContext
}

/**
 * Next selection after a click. Selection is an **ordered** list (index 0 is
 * the primary pick); order matters for booleans (Subtract = first − second).
 *
 * - `id === null` (empty click) → clear.
 * - `additive` (shift-click) → toggle: append if absent, remove if present,
 *   preserving the order of the survivors.
 * - otherwise → replace with `[id]`.
 */
export function nextSelection(
  current: bigint[],
  id: bigint | null,
  additive: boolean,
): bigint[] {
  if (id === null) {
    return []
  }
  if (!additive) {
    return [id]
  }
  return current.includes(id) ? current.filter((x) => x !== id) : [...current, id]
}
