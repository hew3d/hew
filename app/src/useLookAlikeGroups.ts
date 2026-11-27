/**
 * The look-alike groups behind Edit ▸ Link identical copies and the Model
 * Health notice, read off the render path.
 *
 * Finding them asks `hew.query.signature` once per loose Object, so the read
 * waits until the document has been quiet for `delayMs` (one read per burst
 * of edits), never runs while a component is open, and is skipped for a very
 * large model. Until a read lands for the current document the answer is
 * the last one for that document (or none), so the gate and the notice
 * follow the model a moment behind it. The action itself always reads the
 * model afresh.
 */

import { useEffect, useState } from 'react'
import { findLookAlikes, type LookAlikeGroup } from './linkSimilar'

type LookAlikeScene = Parameters<typeof findLookAlikes>[0]

/** Past this many Objects the whole-model read is skipped. */
export const LOOK_ALIKE_MAX_OBJECTS = 400
/** How long the document must be quiet before the read (ms). */
export const LOOK_ALIKE_DELAY_MS = 250

const NONE: LookAlikeGroup[] = []

export function useLookAlikeGroups(
  scene: LookAlikeScene | null | undefined,
  docRev: number,
  disabled: boolean,
  delayMs = LOOK_ALIKE_DELAY_MS,
): LookAlikeGroup[] {
  const [read, setRead] = useState<{ scene: LookAlikeScene | null; groups: LookAlikeGroup[] }>({
    scene: null,
    groups: NONE,
  })
  useEffect(() => {
    if (scene == null || disabled) return
    const timer = setTimeout(() => {
      let groups = NONE
      try {
        if (scene.object_ids().length <= LOOK_ALIKE_MAX_OBJECTS) groups = findLookAlikes(scene, null)
      } catch {
        groups = NONE
      }
      setRead({ scene, groups })
    }, delayMs)
    return () => clearTimeout(timer)
  }, [scene, docRev, disabled, delayMs])
  if (scene == null || disabled || read.scene !== scene) return NONE
  return read.groups
}
