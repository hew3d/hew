/* oxlint-disable lingui/no-unlocalized-strings -- Smooth is a Preview feature: the Eraser's crease copy is English only until Smooth graduates (src/preview, behavior `eraser-crease`). Nothing here is reachable, or in a catalog, in a production build. */
/**
 * The words of the Eraser's crease half: Ctrl/Cmd-click marks an edge of a
 * solid so it stays sharp under Smooth. Kept apart from `EraserTool.ts` so
 * the tool's live strings go through translation and these do not. In a
 * production build `creaseCopy` is `null` and the text is compiled out of the
 * bundle; a preview build reaches it only while `behaviorEnabled('eraser-crease')`.
 */

import { isMac } from '../platform'
import { previewBuild } from '../preview'

const modifierName = (): string => (isMac ? 'Cmd' : 'Ctrl')

/** The words themselves. In a production build `creaseCopy` is `null`, a
 *  literal the bundler folds, so none of the text below is in the built JS. */
function build() {
  return {
    /** The confirmation after a crease commit: what is now creased and what
     *  is not, in plain words. */
    toast(creased: number, uncreased: number): string {
      const edges = (n: number): string => (n === 1 ? 'the edge' : `${n} edges`)
      if (creased > 0 && uncreased > 0) {
        return `Creased ${creased === 1 ? 'one edge' : `${creased} edges`} and uncreased ${uncreased === 1 ? 'one' : uncreased}.`
      }
      if (creased > 0) return `Creased ${edges(creased)}. ${creased === 1 ? 'It stays' : 'They stay'} sharp under Smooth.`
      return `Uncreased ${edges(uncreased)}. ${uncreased === 1 ? 'It softens' : 'They soften'} under Smooth again.`
    },
    /** Nothing under the pointer. */
    idle: (erasing: string): string => `${erasing} ${modifierName()}-click an edge of a solid to crease it.`,
    /** A drawn line under the pointer while the modifier is held. */
    drawnWithModifier: (): string => `${modifierName()}-click works on the edges of solids. Let go of ${modifierName()} to erase this line.`,
    /** A solid edge, modifier not held. */
    solidEdge: (): string => `Edges of a solid can't be erased. ${modifierName()}-click to crease (keeps it sharp under Smooth).`,
    creaseOn: 'Crease: keeps this edge sharp under Smooth',
    creaseOff: 'Uncrease: lets this edge soften under Smooth',
    seamWithModifier: "Soft seams can't be creased.",
    seamToast: "Soft seams can't be creased.",
    seamCode: 'CreaseOnSoftEdge',
    historyLabel: 'Crease',
    componentEdge: 'Open the component first to erase or crease its edges.',
  }
}

export type CreaseCopy = ReturnType<typeof build>

export const creaseCopy: CreaseCopy | null = previewBuild ? build() : null
