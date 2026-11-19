/**
 * Linked copies — the app-side operations on a placement of a linked set.
 *
 * A linked set is what Move, Rotate, Paste, and arrays make by default: the
 * copies share one shape, so editing one changes all. These helpers answer
 * "is this selection one such copy?" and run the four things a person can do
 * about it: Unlink (this copy goes its own way), Unlink all, Make component
 * (the set becomes a named component), and rename.
 *
 * They are scene-level and UI-free; each mutating one is a single undo step.
 */

import { t } from '@lingui/core/macro'
import type { Scene as WasmScene } from './wasm/loader'
import { linkedSetKind, nodeRefFromJs, type NodeRef } from './panels/treeModel'
import { inHistoryBracket } from './tools/transformSelection'
import { kernelErrorMessage } from './kernelErrors'

type LinkScene = Pick<
  WasmScene,
  | 'is_linked'
  | 'instance_def'
  | 'definition_usage'
  | 'instances_of'
  | 'instance_name'
  | 'component_name'
  | 'group_ids'
  | 'owner_definition'
  | 'component_has_nested_members'
>

export interface LinkedPlacement {
  instance: bigint
  definition: bigint
  /** Placements sharing the set, this one included, wherever they sit. Always more than one. */
  copies: number
  /**
   * The copies Unlink can reach from here: the ones standing at the top
   * level of the document. A copy nested inside a component (or another set)
   * still shares the shape and still counts in `copies`, but the kernel
   * only unlinks top-level placements, so it is left out of Unlink all and
   * of "select every copy".
   */
  topLevel: bigint[]
  /** What the shared shape is: an Object, or a Group. */
  kind: 'object' | 'group'
}

/** The placements of `definition` that stand at the top level of the document. */
export function topLevelCopies(scene: Pick<LinkScene, 'instances_of' | 'owner_definition'>, definition: bigint): bigint[] {
  return Array.from(scene.instances_of(definition)).filter((i) => scene.owner_definition(2, i) === undefined)
}

/**
 * Is this placement one the app presents as the plain Object or Group it
 * wraps? True for every placement of a linked wrapper, including one whose
 * set is down to a single copy (deleting copies leaves such a wrapper on
 * purpose). Use this to decide how a thing LOOKS; use `isLinkedSet` to
 * decide whether anything is linked.
 */
export function isWrapperPlacement(scene: Pick<LinkScene, 'is_linked'>, instance: bigint): boolean {
  return scene.is_linked(instance)
}

/**
 * Is this placement one of a linked set of two or more? Only then do the
 * link icon, the "(N linked)" count, Unlink and the "linked" wording apply.
 */
export function isLinkedSet(scene: Pick<LinkScene, 'is_linked' | 'instance_def' | 'definition_usage'>, instance: bigint): boolean {
  if (!scene.is_linked(instance)) return false
  const definition = scene.instance_def(instance)
  return definition !== undefined && scene.definition_usage(definition) > 1
}

/**
 * The selection as one placement of a linked wrapper, however many copies
 * the set has (one is fine), or null when it is anything else.
 */
export function wrapperPlacementOf(scene: LinkScene, selected: readonly NodeRef[]): LinkedPlacement | null {
  if (selected.length !== 1 || selected[0].kind !== 'instance') return null
  const instance = selected[0].id
  if (!scene.is_linked(instance)) return null
  const definition = scene.instance_def(instance)
  if (definition === undefined) return null
  const copies = scene.definition_usage(definition)
  return { instance, definition, copies, topLevel: topLevelCopies(scene, definition), kind: linkedSetKind(scene, definition) }
}

/**
 * The selection as one placement of a linked set, or null when it is anything
 * else. "Linked" means the set has more than one placement: a set that is
 * down to one is just a plain Object, whatever the data still says. A copy
 * that is not at the top level (selected inside an open component) is not
 * actionable either: the kernel only unlinks top-level placements.
 */
export function linkedPlacementOf(scene: LinkScene, selected: readonly NodeRef[]): LinkedPlacement | null {
  const p = wrapperPlacementOf(scene, selected)
  if (p === null || p.copies < 2 || !p.topLevel.includes(p.instance)) return null
  return p
}

export interface LinkGates {
  canUnlink: boolean
  /** Two or more copies stand at the top level, so there is more than this one to unlink. */
  canUnlinkAll: boolean
  /** Make component: any placement of a wrapper, a set of one included. */
  canPromote: boolean
  /** What the linked set is, for the dock's chip; null when nothing is linked. */
  linkedKind: 'object' | 'group' | null
  /** What the placement reads as (a set of one included), for the chip; null for a component. */
  presentedKind: 'object' | 'group' | null
}

export function linkGates(scene: LinkScene, selected: readonly NodeRef[], sessionOpen: boolean): LinkGates {
  const w = sessionOpen ? null : wrapperPlacementOf(scene, selected)
  const p = w !== null ? linkedPlacementOf(scene, selected) : null
  return {
    canUnlink: p !== null,
    canUnlinkAll: p !== null && p.topLevel.length > 1,
    canPromote: w !== null,
    linkedKind: p?.kind ?? null,
    presentedKind: w?.kind ?? null,
  }
}

/**
 * Unlink: this copy becomes an ordinary Object (or Group) of its own; the
 * rest stay linked, and a set left with one copy is plain too. One undo step.
 * Returns the new nodes the copy became: the old instance id is gone, so the
 * selection moves to these.
 */
export function unlinkCopy(scene: WasmScene, instance: bigint): NodeRef[] {
  // A copy nested in a component is not the kernel's to unlink from here.
  if (scene.owner_definition(2, instance) !== undefined) return []
  return Array.from(scene.unlink_copies(BigUint64Array.of(instance)), nodeRefFromJs)
}

/** Unlink every copy of the set. One undo step. Returns the new nodes, in copy order. */
export function unlinkAllCopies(scene: WasmScene, instance: bigint): NodeRef[] {
  const def = scene.instance_def(instance)
  if (def === undefined) return []
  // Top-level copies only: the kernel refuses a placement owned by another
  // definition, and one such copy would abort the whole operation.
  const copies = topLevelCopies(scene, def)
  if (copies.length === 0) return []
  return Array.from(scene.unlink_copies(BigUint64Array.from(copies)), nodeRefFromJs)
}

/**
 * Make Unique on one copy (the lock toast's button): the copy gets a shape of
 * its own but stays a linked-style placement with the same id. One undo step.
 */
export function makeUniqueCopy(scene: WasmScene, instance: bigint): void {
  inHistoryBracket(scene, t`Make Unique`, () => {
    scene.make_unique_by_path_json(BigUint64Array.of(instance), true, false)
  })
}

/** Turn the set into a named component. One undo step. */
export function promoteToComponent(scene: WasmScene, instance: bigint, name: string): void {
  const def = scene.instance_def(instance)
  if (def === undefined) throw new Error('UnknownInstance: that copy is gone')
  inHistoryBracket(scene, t`Make Component`, () => {
    scene.set_linked(def, false)
    scene.set_component_name(def, name)
  })
}

/**
 * Rename from the Outliner row: the copy's own name when it has one (the
 * person already named this copy), else the shared name (all copies).
 */
export function renameLinkedCopy(scene: WasmScene, instance: bigint, name: string): 'copy' | 'all' {
  const trimmed = name.trim()
  const next = trimmed === '' ? null : trimmed
  const own = scene.instance_name(instance)
  if (own !== undefined && own !== '') {
    inHistoryBracket(scene, t`Rename`, () => scene.set_node_name(2, instance, next))
    return 'copy'
  }
  const def = scene.instance_def(instance)
  if (def === undefined) throw new Error('UnknownInstance: that copy is gone')
  inHistoryBracket(scene, t`Rename`, () => scene.set_component_name(def, next))
  return 'all'
}

/**
 * The same gate the menus use, for actions that reach the scene by another
 * road (an Outliner row's menu, Object Info's buttons): while a component is
 * open for editing a copy action is refused out loud, never ignored. Returns
 * true when the action must stop.
 */
export function blockedWhileEditing(sessionOpen: boolean, toast: (message: string, code?: string) => void): boolean {
  if (!sessionOpen) return false
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code
  toast(kernelErrorMessage('ExplodeSessionScope', ''), 'ExplodeSessionScope')
  return true
}

/**
 * Save a linked set to the Library: the set becomes a named component first
 * (a Library item needs a name), but only if the save itself succeeds. When
 * `save` throws, the promotion is undone and the error rethrown.
 * `promotedAt` is the history generation just after the promotion, for
 * `undoPromotionIfUnmoved` (a later, asynchronous failure).
 */
export function promoteForSave<T>(
  scene: WasmScene,
  instance: bigint,
  name: string,
  save: () => T,
): { value: T; promotedAt: bigint } {
  promoteToComponent(scene, instance, name)
  const promotedAt = scene.history_generation()
  try {
    return { value: save(), promotedAt }
  } catch (err) {
    undoPromotionIfUnmoved(scene, promotedAt)
    throw err
  }
}

/** Take back a promotion when nothing else has happened since. Returns whether it did. */
export function undoPromotionIfUnmoved(scene: WasmScene, promotedAt: bigint): boolean {
  if (scene.history_generation() !== promotedAt) return false
  scene.scene_undo().free()
  return true
}
