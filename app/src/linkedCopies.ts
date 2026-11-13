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
import type { NodeRef } from './panels/treeModel'
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
>

export interface LinkedPlacement {
  instance: bigint
  definition: bigint
  /** Placements sharing the set, this one included. */
  copies: number
}

/** The selection as one placement of a linked set, or null when it is anything else. */
export function linkedPlacementOf(scene: LinkScene, selected: readonly NodeRef[]): LinkedPlacement | null {
  if (selected.length !== 1 || selected[0].kind !== 'instance') return null
  const instance = selected[0].id
  if (!scene.is_linked(instance)) return null
  const definition = scene.instance_def(instance)
  if (definition === undefined) return null
  return { instance, definition, copies: scene.definition_usage(definition) }
}

export interface LinkGates {
  canUnlink: boolean
  /** More than one placement shares the set. */
  canUnlinkAll: boolean
  canPromote: boolean
}

export function linkGates(scene: LinkScene, selected: readonly NodeRef[], sessionOpen: boolean): LinkGates {
  const p = sessionOpen ? null : linkedPlacementOf(scene, selected)
  return { canUnlink: p !== null, canUnlinkAll: p !== null && p.copies > 1, canPromote: p !== null }
}

/** Make this one copy independent; the others stay linked. One undo step. */
export function unlinkCopy(scene: WasmScene, instance: bigint): void {
  inHistoryBracket(scene, t`Unlink`, () => {
    scene.make_unique_by_path_json(BigUint64Array.of(instance), true, false)
  })
}

/** Make every copy of the set independent. One undo step. Returns how many were unlinked. */
export function unlinkAllCopies(scene: WasmScene, instance: bigint): number {
  const def = scene.instance_def(instance)
  if (def === undefined) return 0
  const all = Array.from(scene.instances_of(def))
  inHistoryBracket(scene, t`Unlink all`, () => {
    for (const id of all) scene.make_unique_by_path_json(BigUint64Array.of(id), true, false)
  })
  return all.length
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
