/**
 * curveActions — the command ids of the curve features that are NOT tools:
 * Object ▸ Smooth (Off, 1, 2, 3, Bake Smooth), Soften Edges…, and View ▸
 * Tangent Edges. One declaration behind the web menus, the command palette,
 * and App.tsx's dispatch, so each reads the same ids.
 *
 * (The tool commands live in `tools/toolActions.ts`; the native menu's own
 * ids are mapped to these in the Tauri shell, which this module does not
 * touch.)
 */

import { actionAllowed, behaviorEnabled } from './preview'

/** Object ▸ Smooth ▸ Off / 1 / 2 / 3: the subdivision level each id sets. */
export const SMOOTH_LEVEL_ACTIONS: readonly { id: string; level: 0 | 1 | 2 | 3 }[] = [
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- command ids
  { id: 'smooth-off', level: 0 },
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- command ids
  { id: 'smooth-1', level: 1 },
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- command ids
  { id: 'smooth-2', level: 2 },
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- command ids
  { id: 'smooth-3', level: 3 },
]

// oxlint-disable-next-line lingui/no-unlocalized-strings -- command id
export const BAKE_SMOOTH_ACTION = 'bake-smooth'
// oxlint-disable-next-line lingui/no-unlocalized-strings -- command id
export const SOFTEN_EDGES_ACTION = 'soften-edges'
// oxlint-disable-next-line lingui/no-unlocalized-strings -- command id
export const TANGENT_EDGES_ACTION = 'toggle-tangent-edges'

/** The Smooth level command `id` sets, or undefined when it is not one. */
export function smoothLevelFor(id: string): 0 | 1 | 2 | 3 | undefined {
  return SMOOTH_LEVEL_ACTIONS.find((a) => a.id === id)?.level
}

/**
 * Whether command `id` is OFFERED: as a menu item, a palette entry, a key, a
 * native-menu item. The Preview wall withholds what the registry lists.
 * Smooth ▸ Off is the one command that is not listed (it must keep working,
 * for a file that arrives with Smooth on) and is still not offered as a menu
 * item, palette entry or key: without the Smooth preview it is reachable only
 * from Object Info, for an object that already has a level
 * (`panels/SmoothSection.tsx`). `menuActionRef` itself runs it either way.
 */
export function actionOffered(id: string): boolean {
  if (!actionAllowed(id)) return false
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- command id
  return id !== 'smooth-off' || behaviorEnabled('smooth-apply')
}
