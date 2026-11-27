/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * smoothActions — the Smooth level, Bake and Soften edits the Object Info
 * rows and the command palette share. Each applies to a list of objects and,
 * for more than one, lands as a single undo step (a history bracket).
 *
 * Pure of React: the callers decide what to refresh and toast.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { collectLeafIds, nodeRefFromJs, type NodeRef } from './treeModel'

/** The Smooth levels the UI offers (0 is Off). */
export const SMOOTH_LEVELS = [0, 1, 2, 3] as const

/** The Soften row's starting angle, in degrees. */
export const DEFAULT_SOFTEN_DEGREES = 20

/** Run `edit` for every object, as one undo step when there are several. */
function inBracket<T>(scene: WasmScene, label: string, ids: readonly bigint[], edit: (id: bigint) => T): T[] {
  if (ids.length <= 1) return ids.map(edit)
  scene.history_bracket_open()
  try {
    const out = ids.map(edit)
    scene.history_bracket_commit(label)
    return out
  } catch (err) {
    scene.history_bracket_abort()
    throw err
  }
}

/** Each object's Smooth level (0 = Off). */
export function smoothLevels(scene: WasmScene, ids: readonly bigint[]): number[] {
  return ids.map((id) => scene.smooth_level(id))
}

/** The plain-words copy for a refused Smooth level. */
export function smoothRefusal(err: unknown, level: number): { message: string; code: string | undefined } {
  const code = parseKernelErrorCode(err)
  if (code === 'SmoothTooDense') {
    return { message: kernelErrorMessage(code, ''), code }
  }
  if (code === 'AmountOutOfRange') {
    return { message: `This cage is too dense for Smooth level ${level}.`, code: code ?? undefined }
  }
  const raw = err instanceof Error ? err.message : String(err)
  return { message: kernelErrorMessage(code ?? 'Unknown', raw), code: code ?? undefined }
}

/** The code on the toast for a Smooth surface that can no longer be drawn. */
export const SMOOTH_TOO_DENSE_CODE = 'SmoothTooDense'

/** Whether a smoothed object's surface cannot be drawn right now (its level
 *  is on but the shape grew too dense for it). An object at Off, or one that
 *  can no longer be read, is never "off for now". */
export function smoothSurfaceOff(scene: WasmScene, id: bigint): boolean {
  try {
    return scene.smooth_level(id) > 0 && !scene.smooth_surface_available(id)
  } catch {
    return false
  }
}

/** The toast for `smoothSurfaceOff`. */
export function smoothDenseCopy(level: number, name: string): string {
  return `Smooth ${level} on ${name} is off for now: the shape grew too dense for it. Lower the level or simplify the shape.`
}

/**
 * After a committed edit: for each touched object whose Smooth surface is
 * off, one toast per object (`warned` remembers who already got theirs; an
 * object that recovers or goes back to Off is forgotten so a later relapse
 * speaks up again). Cost is O(`ids`), never the whole document.
 */
export function smoothDenseToasts(
  scene: WasmScene,
  ids: readonly bigint[],
  warned: Set<bigint>,
  /** True when `ids` is every smoothed object there is (a refresh that named
   *  no touched objects): a warned object no longer among them went to Off
   *  or away, and is forgotten so a later relapse speaks up again. */
  complete = false,
): { message: string; code: string }[] {
  const out: { message: string; code: string }[] = []
  if (complete) {
    const keep = new Set(ids)
    for (const id of Array.from(warned)) if (!keep.has(id)) warned.delete(id)
  }
  for (const id of ids) {
    let level = 0
    try {
      level = scene.smooth_level(id)
    } catch {
      continue
    }
    if (!smoothSurfaceOff(scene, id)) {
      warned.delete(id)
      continue
    }
    if (warned.has(id)) continue
    warned.add(id)
    let name: string | undefined
    try {
      name = scene.object_name(id)
    } catch {
      name = undefined
    }
    out.push({ message: smoothDenseCopy(level, name !== undefined && name !== '' ? name : 'this object'), code: SMOOTH_TOO_DENSE_CODE })
  }
  return out
}

/** Set `level` on every object, freeing each change. Throws the kernel's
 *  refusal (nothing is left half done for several objects). */
export function setSmoothLevel(scene: WasmScene, ids: readonly bigint[], level: number): void {
  inBracket(scene, 'Set Smooth', ids, (id) => {
    scene.set_smooth_level(id, level).free()
  })
}

/** Bake every smoothed object in `ids`; objects at Off are skipped. Returns
 *  how many were baked. */
export function bakeSmooth(scene: WasmScene, ids: readonly bigint[]): number {
  const smoothed = ids.filter((id) => scene.smooth_level(id) > 0)
  inBracket(scene, 'Bake Smooth', smoothed, (id) => {
    scene.bake(id).free()
  })
  return smoothed.length
}

/** Soften edges under `degrees` on every object. Returns the edges softened. */
export function softenEdges(scene: WasmScene, ids: readonly bigint[], degrees: number): number {
  const radians = (degrees * Math.PI) / 180
  const counts = inBracket(scene, 'Soften Edges', ids, (id) => scene.soften_by_angle(id, radians))
  return counts.reduce((a, b) => a + b, 0)
}

/** "Softened 12 edges." / "No edges to soften." */
export function softenToast(n: number): string {
  if (n === 0) return 'No edges to soften.'
  return `Softened ${n} ${n === 1 ? 'edge' : 'edges'}.`
}

/** Parse the Soften angle field: a number of degrees strictly between 0 and 180. */
export function parseSoftenDegrees(text: string): number | null {
  const t = text.trim().replace(/°$/, '').trim()
  if (!/^\d+(\.\d+)?$|^\.\d+$/.test(t)) return null
  const n = parseFloat(t)
  return n > 0 && n < 180 ? n : null
}

/** The info toast when a boolean's operands include a Smooth object. */
export const SMOOTH_BOOLEAN_NOTE = 'Combining smooth objects uses their cages. Bake first for an exact cut.'

/** Whether any object under the operands has a Smooth level. */
export function operandsHaveSmooth(scene: WasmScene, operands: readonly NodeRef[]): boolean {
  const members = (groupId: bigint): NodeRef[] => scene.group_members(groupId).map(nodeRefFromJs)
  for (const n of operands) {
    const { objectIds } = collectLeafIds(n, members)
    if (objectIds.some((id) => scene.smooth_level(id) > 0)) return true
  }
  return false
}
