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
