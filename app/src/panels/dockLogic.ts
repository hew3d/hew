/**
 * dockLogic — pure selection -> contextual-dock derivation (
 * `05_contextual_dock.md`, rescoped).
 *
 * The spec keys the dock off Face/Edge/Component/Empty/Multi selection.
 * Hew's real selection model (`treeModel.ts`'s `NodeRef`) only distinguishes
 * `object | group | instance | sketch` — there is no face/edge selection,
 * and building one (plus the `face_area()`/`edge_length()` wasm-api queries
 * Entity Info would also need) was explicitly declined as cross-crate kernel
 * work outside a UI push (see the decision log). So this module derives a
 * SMALLER context set from what Hew actually has: Object / Group / Instance
 * / Empty / Multi. A selected sketch or construction guide has no
 * curated verb set defined here — the caller hides the dock for those
 * (matches today's baseline: no dock exists at all yet, so this is not a
 * regression for either case).
 *
 * No React import — pure data in, pure data out, unit-testable directly.
 */

import type { NodeRef } from './treeModel'

export type DockContext = 'empty' | 'object' | 'group' | 'instance' | 'multi'

export interface DockVerb {
  /** Matches a `menuActionRef.current(id)` payload — see App.tsx. */
  id: string
  label: string
}

/**
 * Derive the dock's context from the current selection. `selectedGuide`
 * non-null means a construction guide is selected (mutually exclusive with
 * `selectedIds` elsewhere in App.tsx) — returns null (no dock) since the
 * spec has no defined verb set for it and inventing one is out of scope here.
 */
export function deriveDockContext(selectedIds: NodeRef[], selectedGuide: bigint | null): DockContext | null {
  if (selectedGuide !== null) return null
  if (selectedIds.length === 0) return 'empty'
  if (selectedIds.length > 1) return 'multi'
  const kind = selectedIds[0].kind
  if (kind === 'sketch') return null
  return kind
}

const EMPTY_VERBS: DockVerb[] = [
  { id: 'tool-rectangle', label: 'Rectangle' },
  { id: 'tool-line', label: 'Line' },
  { id: 'tool-circle', label: 'Circle' },
]

const OBJECT_VERBS: DockVerb[] = [
  { id: 'tool-move', label: 'Move' },
  { id: 'tool-pushpull', label: 'Push/Pull' },
  { id: 'tool-paint', label: 'Paint' },
  { id: 'edit-delete', label: 'Erase' },
]

const GROUP_VERBS: DockVerb[] = [
  { id: 'tool-move', label: 'Move' },
  { id: 'enter-context', label: 'Edit' },
  { id: 'ungroup', label: 'Ungroup' },
  { id: 'edit-delete', label: 'Erase' },
]

const INSTANCE_VERBS: DockVerb[] = [
  { id: 'tool-move', label: 'Move' },
  { id: 'enter-context', label: 'Edit' },
  { id: 'make-unique', label: 'Make Unique' },
  { id: 'edit-delete', label: 'Erase' },
]

const MULTI_VERBS: DockVerb[] = [
  { id: 'tool-move', label: 'Move' },
  { id: 'edit-delete', label: 'Erase' },
]

/** The curated, ordered verb list for a context — first item is primary. */
export function dockVerbsFor(context: DockContext): DockVerb[] {
  switch (context) {
    case 'empty': return EMPTY_VERBS
    case 'object': return OBJECT_VERBS
    case 'group': return GROUP_VERBS
    case 'instance': return INSTANCE_VERBS
    case 'multi': return MULTI_VERBS
  }
}

/** Display label for the context chip (`FACE`/`EDGE`/`COMPNT` in the spec's
 * literal terms — Hew's rescoped equivalents below). */
export function dockChipLabel(context: DockContext): string {
  switch (context) {
    case 'empty': return 'DRAW'
    case 'object': return 'OBJECT'
    case 'group': return 'GROUP'
    case 'instance': return 'COMPNT'
    case 'multi': return 'MULTI'
  }
}
