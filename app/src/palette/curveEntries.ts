/**
 * palette/curveEntries — the command palette's curve-feature entries that are
 * not one-per-tool: Deform's four mode entries, Skin (Loft under SketchUp's
 * name), Chamfer, Object ▸ Smooth, Soften Edges…, and Tangent Edges.
 * `registry.ts` splices them in where the equivalent hand-written entries
 * used to sit, so the palette's order is unchanged.
 */

import { t } from '@lingui/core/macro'
import type { PaletteEntry } from './registry'

/** Deform's four palette entries: each opens the one Deform tool in its own
 *  mode (the menus do the same). The ids are `menuActionRef` payloads. */
// oxlint-disable lingui/no-unlocalized-strings -- group and id values are identifiers; every label, description, and synonym is wrapped in t``
export function deformPaletteEntries(): PaletteEntry[] {
  return [
    {
      id: 'tool-deform-twist',
      label: t`Twist`,
      description: t`Twist a solid around its length: click an end of its box, then turn.`,
      group: 'Tools',
      synonyms: [t`twist`, t`fredoscale`, t`spiral`, t`deform`],
    },
    {
      id: 'tool-deform-bend',
      label: t`Bend`,
      description: t`Bend a solid into an arc: click an end of its box, then pull it sideways.`,
      group: 'Tools',
      synonyms: [t`bend`, t`flex`, t`curve`, t`fredoscale`, t`deform`],
    },
    {
      id: 'tool-deform-taper',
      label: t`Taper`,
      description: t`Narrow or widen one end of a solid toward the other.`,
      group: 'Tools',
      synonyms: [t`taper`, t`narrow`, t`cone`, t`fredoscale`, t`deform`],
    },
    {
      id: 'tool-deform-bend-path',
      label: t`Bend to Path`,
      description: t`Bend a solid along a drawn line or curve.`,
      group: 'Tools',
      synonyms: [t`shape bender`, t`bend along path`, t`wrap`, t`deform`],
    },
  ]
}
// oxlint-enable lingui/no-unlocalized-strings


/** The same tool as Loft, under the name SketchUp users know. */
// oxlint-disable lingui/no-unlocalized-strings -- group and id values are identifiers; every label, description, and synonym is wrapped in t``
export function skinPaletteEntry(): PaletteEntry {
  return {
    id: 'tool-skin',
    label: t`Skin`,
    description: t`Skin a solid through two or more shapes (the same as Loft).`,
    group: 'Tools',
    synonyms: [t`loft`, t`skin`],
  }
}

/** Chamfer, Object ▸ Smooth, Soften Edges…, and Tangent Edges. */
export function curveActionPaletteEntries(): PaletteEntry[] {
  return [
    { id: 'tool-chamfer', label: t`Chamfer`, description: t`Bevel the edges of a solid, or a drawn corner, with a flat cut.`, group: 'Tools', synonyms: [t`bevel`] },
    { id: 'smooth-off', label: t`Smooth: Off`, description: t`Turn Smooth off for the selected objects.`, group: 'Actions', synonyms: [t`subdivide`, t`subd`, t`pillow`], gate: 'selection' },
    { id: 'smooth-1', label: t`Smooth: 1`, description: t`Round the selected objects one step.`, group: 'Actions', synonyms: [t`subdivide`, t`subd`, t`pillow`], gate: 'selection' },
    { id: 'smooth-2', label: t`Smooth: 2`, description: t`Round the selected objects two steps.`, group: 'Actions', synonyms: [t`subdivide`, t`subd`, t`pillow`], gate: 'selection' },
    { id: 'smooth-3', label: t`Smooth: 3`, description: t`Round the selected objects three steps.`, group: 'Actions', synonyms: [t`subdivide`, t`subd`, t`pillow`], gate: 'selection' },
    { id: 'bake-smooth', label: t`Bake Smooth`, description: t`Keep the rounded surface of the selected smooth objects as real faces.`, group: 'Actions', synonyms: [t`subdivide`, t`subd`, t`pillow`, t`apply smooth`], gate: 'selection' },
    { id: 'soften-edges', label: t`Soften Edges…`, description: t`Hide the lines on gentle edges of the selected objects so curved shapes look smooth.`, group: 'Actions', synonyms: [t`soften`, t`smooth edges`, t`hide edges`], gate: 'canSoften' },
    { id: 'toggle-tangent-edges', label: t`Toggle Tangent Edges`, description: t`Show or hide the thin lines where a rounded edge or corner meets a flat face.`, group: 'Actions', synonyms: [t`rails`, t`tangent lines`] },
  ]
}
// oxlint-enable lingui/no-unlocalized-strings
