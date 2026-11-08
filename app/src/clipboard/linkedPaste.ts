/**
 * Linked paste — a paste into the same document that makes a placement of
 * the copied thing's set instead of an independent copy.
 *
 * Copy records where the bytes came from (`LinkSource`). A paste links only
 * while the document and window still match and every copied node is still a
 * live node; after a delete (Cut) or a different document it is the ordinary
 * independent paste. A loose Object or Group is wrapped as a set first; an
 * Instance simply gets another placement of its own shape.
 */

import { t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import type { LinkSource } from './modelClipboard'
import type { LibraryInsertResult } from '../tools/LibraryPlaceTool'
import { inHistoryBracket } from '../tools/transformSelection'
import { composeAffine } from '../tools/transformMath'
import { invertAffine3x4 } from '../viewport/geoHelpers'

type PasteScene = Pick<
  WasmScene,
  | 'ensure_document_id'
  | 'extract_item_placement'
  | 'node_is_live'
  | 'wrap_linked'
  | 'instance_def'
  | 'instance_pose'
  | 'place_instance'
  | 'history_bracket_open'
  | 'history_bracket_commit'
  | 'history_bracket_abort'
>

const KIND_NUMBER = { object: 0, group: 1, instance: 2 } as const
const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]

/**
 * The copied nodes a paste can link to right now, or null when it cannot:
 * the same document (and load of it), the same window, and every node still
 * live. After a linked paste wrapped the loose source the placement stands
 * for it, but undoing that paste unwraps it again, so both spellings are
 * tried.
 */
export function resolveLinkSource(
  scene: PasteScene,
  source: LinkSource | null,
  nonce: string,
  generation: number,
): LinkSource['nodes'] | null {
  if (source === null || source.nodes.length === 0) return null
  if (source.nonce !== nonce || source.generation !== generation) return null
  if (scene.ensure_document_id() !== source.documentId) return null
  const live = (nodes: LinkSource['nodes']): boolean =>
    nodes.length > 0 && nodes.every((n) => scene.node_is_live(KIND_NUMBER[n.kind], n.id))
  if (live(source.nodes)) return source.nodes
  if (source.repointed !== undefined && live(source.repointed)) return source.repointed
  return null
}

/** Whether a paste may link (see `resolveLinkSource`). */
export function canPasteLinked(
  scene: PasteScene,
  source: LinkSource | null,
  nonce: string,
  generation: number,
): boolean {
  return resolveLinkSource(scene, source, nonce, generation) !== null
}

type Affine12 = [number, number, number, number, number, number, number, number, number, number, number, number]

function asAffine(a: ArrayLike<number>): Affine12 {
  return Array.from(a) as Affine12
}

/**
 * Place linked copies of the copied nodes. `pasteAffine` is the affine the
 * independent paste would have passed to `insert_item` (item to world). Every
 * copy moves by `pasteAffine · live⁻¹`, where `live` is where the SOURCE sits
 * now (the same placement `extract_item` would use today), so the copy lands
 * where the ghost showed even when the source was moved after Copy.
 * `copyAffine` (the placement at copy time) is only the fallback when the
 * live one cannot be read. One undo step. Returns the new placements and the
 * sources that were wrapped, so the caller can keep the clipboard pointing at
 * live nodes.
 */
export function pasteLinked(
  scene: PasteScene,
  nodes: LinkSource['nodes'],
  copyAffine: readonly number[] | null,
  pasteAffine: ArrayLike<number>,
): { result: LibraryInsertResult; sources: LinkSource['nodes'] } {
  const live = scene.extract_item_placement(
    Uint8Array.from(nodes.map((n) => KIND_NUMBER[n.kind])),
    BigUint64Array.from(nodes.map((n) => n.id)),
  )
  const back = invertAffine3x4(live !== undefined ? Array.from(live) : (copyAffine ?? IDENTITY))
  if (back === null) throw new Error('Singular: the copied position cannot be inverted')
  const delta = composeAffine(asAffine(back), asAffine(pasteAffine))
  return inHistoryBracket(scene as WasmScene, t`Paste`, () => {
    const ids: bigint[] = []
    const sources: LinkSource['nodes'] = []
    for (const node of nodes) {
      if (node.kind === 'instance') {
        const def = scene.instance_def(node.id)
        const pose = scene.instance_pose(node.id)
        if (def === undefined || pose === undefined) throw new Error('UnknownInstance: the copied component is gone')
        ids.push(scene.place_instance(def, Float64Array.from(composeAffine(asAffine(pose), delta))))
        sources.push(node)
      } else {
        const wrapped = scene.wrap_linked(KIND_NUMBER[node.kind], node.id)
        const def = scene.instance_def(wrapped)
        if (def === undefined) throw new Error('UnknownInstance: the set was not created')
        ids.push(scene.place_instance(def, Float64Array.from(delta)))
        sources.push({ kind: 'instance', id: wrapped })
      }
    }
    return {
      result: {
        rootKinds: ids.map(() => 2),
        rootIds: ids,
        definitionsReused: 0,
        worldSketchesSkipped: 0,
        annotationsSkipped: 0,
      },
      sources,
    }
  })
}

export type PasteMode = 'default' | 'independent' | 'linked'

/** What a paste should do, decided before anything is placed. */
export type PastePlan =
  /** Place linked copies of these live nodes. */
  | { kind: 'linked'; nodes: LinkSource['nodes'] }
  /** Asked for a link and none is possible: say so, paste nothing. */
  | { kind: 'refuse' }
  /** An ordinary paste. `origin` is what `insert_item` links component
   *  definitions back with: the window's token when links are wanted, none
   *  when the paste must stand on its own. */
  | { kind: 'plain'; origin: string | undefined }

export function planPaste(
  mode: PasteMode,
  preferLinked: boolean,
  resolved: LinkSource['nodes'] | null,
  nonce: string,
): PastePlan {
  const wantLinked = mode === 'linked' || (mode === 'default' && preferLinked)
  if (wantLinked && resolved !== null) return { kind: 'linked', nodes: resolved }
  if (mode === 'linked') return { kind: 'refuse' }
  return { kind: 'plain', origin: wantLinked ? nonce : undefined }
}
