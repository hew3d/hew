/**
 * dockLogic — pure selection -> contextual-dock derivation (
 * `05_contextual_dock.md`, rescoped).
 *
 * The spec keys the dock off Face/Edge/Component/Empty/Multi selection.
 * Hew's real selection model (`treeModel.ts`'s `NodeRef`) only distinguishes
 * `object | group | instance | sketch` — there is no face/edge selection,
 * and building one (plus the `face_area()`/`edge_length()` wasm-api queries
 * Object Info would also need) was explicitly declined as cross-crate kernel
 * work outside a UI push (see the decision log). So this module derives a
 * SMALLER context set from what Hew actually has: Object / Group / Instance
 * / Empty / Multi / Sketch. A selected construction guide still has no
 * curated verb set defined here — the caller hides the dock for that case
 * (matches today's baseline for guides). A selected sketch USED to fall into
 * that same "no dock" bucket, but "sketches are first-class interactable"
 * (+) gave sketches real Push/Pull/Move/Rotate/Scale/Erase behavior, so
 * the dock now has a dedicated `'sketch'` context for it.
 *
 * No React import — pure data in, pure data out, unit-testable directly.
 */

import { i18n, type MessageDescriptor } from '@lingui/core'
import { msg, t } from '@lingui/core/macro'
import type { ToolName } from '../tools/toolIcons'
import { toolLabel } from '../tools/toolRegistry'
import { isSketchScopedKind, type NodeRef } from './treeModel'

export type DockContext = 'empty' | 'object' | 'group' | 'instance' | 'multi' | 'sketch'

export interface DockVerb {
  /** Matches a `menuActionRef.current(id)` payload — see App.tsx. */
  id: string
  /** Set on verbs that activate a tool: the label comes from the tool
   * registry and the icon is keyed on it, never on translated text. */
  toolName?: ToolName
  /** Display copy for verbs that are not tools. Resolve via `dockVerbLabel`. */
  label?: MessageDescriptor
}

/** The translated display label for a verb, resolved at render time. */
export function dockVerbLabel(verb: DockVerb): string {
  if (verb.toolName !== undefined) return toolLabel(verb.toolName)
  return verb.label === undefined ? verb.id : i18n._(verb.label)
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
  // Sketch-scoped selections (a whole sketch, or any drawn shape, curve,
  // line, or region on a plane) all get the sketch verb set — same context,
  // finer selection. A drawing on a solid's face gets it too: same
  // drawing-scoped actions (erase, move), finer target.
  if (isSketchScopedKind(kind)) return 'sketch'
  return kind
}

/* oxlint-disable lingui/no-unlocalized-strings -- tool registry names are identifiers; the display label is resolved with toolLabel() */
const TOOL_VERB_NAME: Record<string, ToolName> = {
  'tool-rectangle': 'Rectangle',
  'tool-line': 'Line',
  'tool-circle': 'Circle',
  'tool-arc': 'Arc',
  'tool-pushpull': 'Push/Pull',
  'tool-move': 'Move',
  'tool-paint': 'Paint',
  'tool-position-texture': 'Position Texture',
  'tool-scale': 'Scale',
  'tool-rotate': 'Rotate',
}
/* oxlint-enable lingui/no-unlocalized-strings */

function toolVerb(id: string): DockVerb {
  return { id, toolName: TOOL_VERB_NAME[id] }
}

const EMPTY_VERBS: DockVerb[] = [
  toolVerb('tool-rectangle'),
  toolVerb('tool-line'),
  toolVerb('tool-circle'),
  toolVerb('tool-arc'),
  // Not a toggleable tool (it opens a dialog, docs/design/3d-text.md) but
  // reachable the same way every other Draw verb here is: same
  // `menuActionRef.current(id)` dispatch as the native menu and palette.
  { id: 'draw-3d-text', label: msg`3D Text…` },
]

/** Spec's Face row (Push/Pull primary, then Offset · Move · Paint · Erase) is
 * the nearest analog for Hew's Object context — Offset is skipped because Hew
 * has no Offset tool (align only where the verb already exists). Make
 * Component rides along (gated by `DockGates.canMakeComponent`) so turning a
 * plain object into a reusable definition is one click, not a menu dive. */
const OBJECT_VERBS: DockVerb[] = [
  toolVerb('tool-pushpull'),
  toolVerb('tool-move'),
  toolVerb('tool-paint'),
  // Position Texture (paint-tool design §3): shown unconditionally, same
  // posture as Paint — the dock has no per-face gate (no face-level
  // selection model, see this module's doc comment), so the TOOL itself
  // toasts on a click that misses a textured face rather than the dock
  // hiding/disabling the verb.
  toolVerb('tool-position-texture'),
  { id: 'edit-make-component', label: msg`Make Component` },
  // Library effort: saves the selection as a reusable `.hew` item (gated by
  // DockGates.canSaveToLibrary — hidden outright on a platform with no
  // library backend). Sits ahead of Erase, matching the destructive-verb-
  // last convention every other row already follows.
  { id: 'save-to-library', label: msg`Save to Library` },
  { id: 'edit-delete', label: msg`Erase` },
]

/** Spec's Component/Group row: Edit (enter) primary, then Move · Scale ·
 * Make Unique · Explode. For a Group, Make Unique/Explode don't apply —
 * Ungroup is Hew's group-level explode analog (kept under its menu name so
 * the dock stays a shortcut surface for the same verb, not a synonym).
 * Make Component (gated) promotes the group to a component definition. */
const GROUP_VERBS: DockVerb[] = [
  { id: 'enter-context', label: msg`Edit` },
  toolVerb('tool-move'),
  toolVerb('tool-scale'),
  { id: 'edit-make-component', label: msg`Make Component` },
  { id: 'ungroup', label: msg`Ungroup` },
  // See OBJECT_VERBS's comment — same Library effort addition, same
  // destructive-verb-last placement.
  { id: 'save-to-library', label: msg`Save to Library` },
  { id: 'edit-delete', label: msg`Erase` },
]

/** Spec's Component row verbatim — every verb already exists in Hew
 * (enter-context / Move / Scale / runMakeUnique / runExplodeInstance), plus
 * Save to Library (Library effort — appended, since this row has no Erase
 * verb to sit ahead of). */
const INSTANCE_VERBS: DockVerb[] = [
  { id: 'enter-context', label: msg`Edit` },
  toolVerb('tool-move'),
  toolVerb('tool-scale'),
  { id: 'make-unique', label: msg`Make Unique` },
  { id: 'explode-instance', label: msg`Explode` },
  { id: 'save-to-library', label: msg`Save to Library` },
]

/** A placement of a linked set (copies made by Move, Paste, ...): the copy
 * actions replace Make Unique, and Make component is the one way out. */
const LINKED_COPY_VERBS: DockVerb[] = [
  { id: 'enter-context', label: msg`Edit` },
  toolVerb('tool-move'),
  toolVerb('tool-scale'),
  { id: 'edit-unlink', label: msg`Unlink` },
  { id: 'edit-unlink-all', label: msg`Unlink all` },
  { id: 'edit-make-component', label: msg`Make component` },
  { id: 'explode-instance', label: msg`Explode` },
  { id: 'save-to-library', label: msg`Save to Library` },
]

/** Group (gated by `DockGates.canGroup`) is the natural next step after
 * sweeping up several things — the same `edit-group` action as Object ▸ Group. */
const MULTI_VERBS: DockVerb[] = [
  toolVerb('tool-move'),
  { id: 'edit-group', label: msg`Group` },
  { id: 'edit-delete', label: msg`Erase` },
]

/** A free-standing (not-yet-extruded) sketch's row: Push/Pull primary (it's
 * the verb that turns the sketch into a real Object), then the transform
 * trio Move/Rotate/Scale (MoveTool/RotateTool/ScaleTool all already handle
 * `NodeRef.kind === 'sketch'` — "sketches are first-class interactable"),
 * then Erase. Same verb ids as the other rows so `menuActionRef` dispatch
 * and `activeToolId` highlighting need no special-casing for this context. */
const SKETCH_VERBS: DockVerb[] = [
  toolVerb('tool-pushpull'),
  toolVerb('tool-move'),
  toolVerb('tool-rotate'),
  toolVerb('tool-scale'),
  { id: 'edit-delete', label: msg`Erase` },
]

/**
 * Whether a verb is actually usable given how the dock arrived at its
 * context. When the dock shows the sketch row only because the cursor is
 * HOVERING a sketch region (nothing is selected — `hoverPreviewOnly`), the
 * only verb that genuinely acts on the hovered region from a click-through
 * is Push/Pull; Move/Rotate/Scale/Erase all need the sketch to actually be
 * selected first, so they render disabled in the preview. With a real
 * selection (hoverPreviewOnly = false) every verb is enabled.
 */
export function isDockVerbEnabled(verb: DockVerb, hoverPreviewOnly: boolean): boolean {
  return !hoverPreviewOnly || verb.id === 'tool-pushpull'
}

/**
 * Selection-dependent applicability for the structural verbs. Derived in
 * App.tsx from the same `menuGates` memo the Edit menu uses, so the dock and
 * the menus always agree on what's currently possible. A verb whose gate is
 * false is HIDDEN (the dock is a curated shortcut row, not a full menu —
 * an inapplicable verb is noise, not information).
 */
export interface DockGates {
  /** ≥2 distinct sibling nodes, no sketch sub-entities (Object ▸ Group). */
  canGroup: boolean
  /** ≥1 sibling object/group/instance, top level (Object ▸ Make Component). */
  canMakeComponent: boolean
  /** Whether the platform has a working library backend
   * (`libraryStore().available()`) — hides Save to Library when false.
   * Optional (unlike the two gates above) so call sites that predate the
   * Library effort keep compiling unchanged; omitted is treated as false
   * (hidden), matching the dock's "an inapplicable verb is noise" rule. */
  canSaveToLibrary?: boolean
  /** The selection is one placement of a linked set: the dock offers the
   * copy actions in place of Make Unique. Optional like `canSaveToLibrary`. */
  canUnlink?: boolean
  /** More than one placement shares the set (Unlink all makes sense). */
  canUnlinkAll?: boolean
}

/** The curated, ordered verb list for a context — first item is primary.
 * `gates` (when given) filters out the structural verbs that don't apply to
 * the actual selection. */
export function dockVerbsFor(context: DockContext, gates?: DockGates): DockVerb[] {
  const verbs = (() => {
    switch (context) {
      case 'empty': return EMPTY_VERBS
      case 'object': return OBJECT_VERBS
      case 'group': return GROUP_VERBS
      case 'instance': return gates?.canUnlink === true ? LINKED_COPY_VERBS : INSTANCE_VERBS
      case 'multi': return MULTI_VERBS
      case 'sketch': return SKETCH_VERBS
    }
  })()
  if (gates === undefined) return verbs
  return verbs.filter((v) => {
    if (v.id === 'edit-group') return gates.canGroup
    if (v.id === 'edit-make-component') return gates.canMakeComponent
    if (v.id === 'save-to-library') return gates.canSaveToLibrary ?? false
    if (v.id === 'edit-unlink-all') return gates.canUnlinkAll ?? false
    return true
  })
}

/** Display label for the context chip (`FACE`/`EDGE`/`COMPNT` in the spec's
 * literal terms — Hew's rescoped equivalents below). */
export function dockChipLabel(context: DockContext, linkedCopy = false): string {
  if (context === 'instance' && linkedCopy) {
    return t({ comment: 'Short uppercase chip on the contextual dock when one linked copy is selected', message: 'COPIES' })
  }
  switch (context) {
    case 'empty': return t({ comment: 'Short uppercase chip on the contextual dock naming what is selected', message: 'DRAW' })
    case 'object': return t({ comment: 'Short uppercase chip on the contextual dock naming what is selected', message: 'OBJECT' })
    case 'group': return t({ comment: 'Short uppercase chip on the contextual dock naming what is selected', message: 'GROUP' })
    case 'instance': return t({ comment: 'Short uppercase chip on the contextual dock naming what is selected', message: 'COMPNT' })
    case 'multi': return t({ comment: 'Short uppercase chip on the contextual dock naming what is selected', message: 'MULTI' })
    case 'sketch': return t({ comment: 'Short uppercase chip on the contextual dock naming what is selected', message: 'SKETCH' })
  }
}
