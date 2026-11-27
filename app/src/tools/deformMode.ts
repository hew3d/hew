/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * deformMode — which Deform mode the next (or the running) Deform tool is in.
 *
 * Twist, Bend, Taper and Bend to Path are four entries in the menus and the
 * command palette that all open the one Deform tool, the way Fillet and
 * Chamfer share theirs. The entries set the mode here before selecting the
 * tool; a Deform tool already running is told, and restarts in the new mode.
 */

export type DeformMode = 'twist' | 'bend' | 'taper' | 'bend-to-path'

export const DEFORM_MODES: readonly DeformMode[] = ['twist', 'bend', 'taper', 'bend-to-path']

export const DEFORM_MODE_LABEL: Record<DeformMode, string> = {
  'twist': 'Twist',
  'bend': 'Bend',
  'taper': 'Taper',
  'bend-to-path': 'Bend to Path',
}

let current: DeformMode = 'twist'
const subscribers = new Set<(mode: DeformMode) => void>()

export function getDeformMode(): DeformMode {
  return current
}

export function setDeformMode(mode: DeformMode): void {
  current = mode
  for (const fn of [...subscribers]) fn(mode)
}

/** Subscribe to mode changes; returns the unsubscribe. Also the shape React's
 *  `useSyncExternalStore` wants (`getDeformMode` is the snapshot). */
export function subscribeDeformMode(fn: (mode: DeformMode) => void): () => void {
  subscribers.add(fn)
  return () => {
    subscribers.delete(fn)
  }
}
